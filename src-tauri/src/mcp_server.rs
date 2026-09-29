//! Read-only MCP server for AI apps connected under Settings → Connection.
//!
//! The AI app starts `OpenWiki --mcp` and talks JSON-RPC 2.0 over stdin/stdout, one message per
//! line. The database is opened read-only and only SELECT queries are accepted, so a connected
//! AI can look through the user's data but never change or delete it.

use rusqlite::types::ValueRef;
use rusqlite::{Connection, OpenFlags};
use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::path::Path;
use std::time::Duration;

/// Command-line flag that starts OpenWiki as the MCP server instead of the app.
pub const MCP_ARG: &str = "--mcp";
/// Most rows one query returns, so a broad SELECT cannot flood the AI app.
const MAX_ROWS: usize = 1000;
/// Answered when the client does not say which protocol version it speaks.
const DEFAULT_PROTOCOL_VERSION: &str = "2025-06-18";

/// Serves requests from stdin until the AI app closes it.
pub fn serve_stdio() {
    let db_path = match crate::storage::database::Database::get_db_path() {
        Ok(path) => path,
        Err(e) => {
            eprintln!("[openwiki-mcp] cannot locate the OpenWiki database: {}", e);
            return;
        }
    };

    let mut stdout = std::io::stdout();
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        if let Some(reply) = handle_message(&line, &db_path) {
            if writeln!(stdout, "{}", reply).and_then(|_| stdout.flush()).is_err() {
                break;
            }
        }
    }
}

/// Answers one line from the client: a request, a notification (no reply) or a batch of them.
fn handle_message(line: &str, db_path: &Path) -> Option<Value> {
    let message: Value = match serde_json::from_str(line) {
        Ok(message) => message,
        Err(e) => return Some(error_reply(Value::Null, -32700, &format!("Parse error: {}", e))),
    };
    match message {
        Value::Array(batch) => {
            let replies: Vec<Value> = batch
                .iter()
                .filter_map(|message| handle_request(message, db_path))
                .collect();
            (!replies.is_empty()).then_some(Value::Array(replies))
        }
        message => handle_request(&message, db_path),
    }
}

fn handle_request(message: &Value, db_path: &Path) -> Option<Value> {
    // Notifications carry no id and get no reply.
    let id = message.get("id")?.clone();
    let method = message.get("method").and_then(Value::as_str).unwrap_or_default();
    let params = message.get("params").cloned().unwrap_or(Value::Null);

    let result = match method {
        "initialize" => initialize(&params),
        "ping" => json!({}),
        "tools/list" => json!({ "tools": tools() }),
        "tools/call" => call_tool(&params, db_path),
        _ => return Some(error_reply(id, -32601, &format!("Method not found: {}", method))),
    };
    Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
}

fn error_reply(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn initialize(params: &Value) -> Value {
    // Tools work the same in every protocol version so far, so answer in the client's.
    let version = params
        .get("protocolVersion")
        .and_then(Value::as_str)
        .unwrap_or(DEFAULT_PROTOCOL_VERSION);
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": {} },
        "serverInfo": { "name": "openwiki", "version": env!("CARGO_PKG_VERSION") },
        "instructions": "Read-only access to the user's OpenWiki knowledge base (SQLite). \
            Saved items are in captured_content, knowledge pages in wiki_pages.",
    })
}

fn tools() -> Value {
    json!([
        {
            "name": "read_query",
            "description": format!(
                "Run a SELECT query on the OpenWiki database and return the rows as JSON \
                 (at most {} rows). The database is read-only.",
                MAX_ROWS
            ),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "SELECT SQL query to execute" }
                },
                "required": ["query"]
            }
        },
        {
            "name": "list_tables",
            "description": "List all tables in the OpenWiki database",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "describe_table",
            "description": "Get the columns of a table in the OpenWiki database",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "table_name": { "type": "string", "description": "Name of the table to describe" }
                },
                "required": ["table_name"]
            }
        }
    ])
}

fn call_tool(params: &Value, db_path: &Path) -> Value {
    let name = params.get("name").and_then(Value::as_str).unwrap_or_default();
    let args = params.get("arguments").cloned().unwrap_or(Value::Null);
    let text_arg = |key: &str| {
        args.get(key)
            .and_then(Value::as_str)
            .ok_or_else(|| format!("Missing argument: {}", key))
    };

    let outcome = match name {
        "read_query" => text_arg("query").and_then(|query| read_query(db_path, query)),
        "list_tables" => run_query(
            db_path,
            "SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name",
            &[],
        ),
        "describe_table" => text_arg("table_name").and_then(|table| {
            run_query(
                db_path,
                "SELECT cid, name, type, \"notnull\", dflt_value, pk FROM pragma_table_info(?1)",
                &[table],
            )
        }),
        _ => Err(format!("Unknown tool: {}", name)),
    };

    match outcome {
        Ok(text) => json!({ "content": [{ "type": "text", "text": text }] }),
        Err(e) => json!({
            "content": [{ "type": "text", "text": format!("Error: {}", e) }],
            "isError": true
        }),
    }
}

/// Runs the AI's own SQL. Only SELECT (optionally after WITH) gets through: ATTACH and some
/// PRAGMAs count as read-only to SQLite but would reach other files or change the connection.
fn read_query(db_path: &Path, sql: &str) -> Result<String, String> {
    let keyword = sql
        .trim_start()
        .split(|c: char| !c.is_ascii_alphabetic())
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    if keyword != "SELECT" && keyword != "WITH" {
        return Err("Only SELECT queries are allowed: OpenWiki gives AI apps read-only access".into());
    }
    run_query(db_path, sql, &[])
}

/// Runs one read-only statement and returns the rows as pretty JSON.
fn run_query(db_path: &Path, sql: &str, params: &[&str]) -> Result<String, String> {
    let conn = open_read_only(db_path)?;
    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    if !stmt.readonly() {
        return Err("Only read queries are allowed: OpenWiki gives AI apps read-only access".into());
    }
    let columns: Vec<String> = stmt.column_names().into_iter().map(String::from).collect();
    let mut rows = stmt
        .query(rusqlite::params_from_iter(params))
        .map_err(|e| e.to_string())?;

    let mut out = Vec::new();
    let mut truncated = false;
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        if out.len() == MAX_ROWS {
            truncated = true;
            break;
        }
        let mut object = serde_json::Map::new();
        for (index, column) in columns.iter().enumerate() {
            let value = row.get_ref(index).map_err(|e| e.to_string())?;
            object.insert(column.clone(), cell(value));
        }
        out.push(Value::Object(object));
    }

    let mut text = serde_json::to_string_pretty(&out).map_err(|e| e.to_string())?;
    if truncated {
        text.push_str(&format!(
            "\n(Only the first {} rows are shown; narrow the query with WHERE or LIMIT.)",
            MAX_ROWS
        ));
    }
    Ok(text)
}

fn open_read_only(db_path: &Path) -> Result<Connection, String> {
    let conn = Connection::open_with_flags(
        db_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| format!("Cannot open the OpenWiki database at {}: {}", db_path.display(), e))?;
    // OpenWiki may be writing at the same time; wait for it instead of failing.
    conn.busy_timeout(Duration::from_secs(5))
        .map_err(|e| e.to_string())?;
    conn.execute_batch("PRAGMA query_only = ON;")
        .map_err(|e| e.to_string())?;
    Ok(conn)
}

fn cell(value: ValueRef<'_>) -> Value {
    match value {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(number) => json!(number),
        ValueRef::Real(number) => json!(number),
        ValueRef::Text(text) => json!(String::from_utf8_lossy(text)),
        ValueRef::Blob(bytes) => json!(format!("<{} bytes of binary data>", bytes.len())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_db() -> (TempDir, std::path::PathBuf) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("openwiki.db");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE notes (id INTEGER PRIMARY KEY, body TEXT, data BLOB);
             INSERT INTO notes (body, data) VALUES ('first', x'0102'), ('second', NULL);",
        )
        .unwrap();
        (dir, path)
    }

    fn request(db_path: &Path, id: i64, method: &str, params: Value) -> Value {
        let line = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        handle_message(&line.to_string(), db_path).expect("a request gets a reply")
    }

    fn call(db_path: &Path, name: &str, arguments: Value) -> Value {
        request(db_path, 7, "tools/call", json!({ "name": name, "arguments": arguments }))["result"]
            .clone()
    }

    fn note_count(db_path: &Path) -> i64 {
        Connection::open(db_path)
            .unwrap()
            .query_row("SELECT count(*) FROM notes", [], |row| row.get(0))
            .unwrap()
    }

    #[test]
    fn initialize_answers_in_the_clients_protocol_version() {
        let (_dir, path) = test_db();
        let reply = request(&path, 1, "initialize", json!({ "protocolVersion": "2024-11-05" }));
        assert_eq!(reply["id"], 1);
        assert_eq!(reply["result"]["protocolVersion"], "2024-11-05");
        assert!(reply["result"]["capabilities"]["tools"].is_object());
        assert_eq!(reply["result"]["serverInfo"]["name"], "openwiki");
    }

    #[test]
    fn notifications_get_no_reply() {
        let (_dir, path) = test_db();
        let line = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
        assert!(handle_message(&line.to_string(), &path).is_none());
    }

    #[test]
    fn only_read_tools_are_offered() {
        let (_dir, path) = test_db();
        let reply = request(&path, 2, "tools/list", Value::Null);
        let names: Vec<&str> = reply["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["read_query", "list_tables", "describe_table"]);
    }

    #[test]
    fn select_returns_rows_as_json() {
        let (_dir, path) = test_db();
        let result = call(&path, "read_query", json!({ "query": "SELECT body, data FROM notes ORDER BY id" }));
        assert!(result.get("isError").is_none());
        let rows: Value = serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(rows[0]["body"], "first");
        assert_eq!(rows[0]["data"], "<2 bytes of binary data>");
        assert_eq!(rows[1]["data"], Value::Null);
    }

    #[test]
    fn writes_are_refused_and_change_nothing() {
        let (_dir, path) = test_db();
        for query in [
            "DELETE FROM notes",
            "DROP TABLE notes",
            "WITH doomed AS (SELECT 1) DELETE FROM notes",
            "ATTACH DATABASE ':memory:' AS other",
        ] {
            let result = call(&path, "read_query", json!({ "query": query }));
            assert_eq!(result["isError"], true, "{} should be refused", query);
        }
        assert_eq!(note_count(&path), 2);
    }

    #[test]
    fn list_and_describe_tables() {
        let (_dir, path) = test_db();
        let tables = call(&path, "list_tables", json!({}));
        assert!(tables["content"][0]["text"].as_str().unwrap().contains("\"notes\""));

        let columns = call(&path, "describe_table", json!({ "table_name": "notes" }));
        let columns: Value = serde_json::from_str(columns["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(columns.as_array().unwrap().len(), 3);
        assert_eq!(columns[1]["name"], "body");
    }

    #[test]
    fn long_results_are_cut_at_the_row_limit() {
        let (_dir, path) = test_db();
        let query = format!(
            "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < {}) SELECT i FROM n",
            MAX_ROWS + 5
        );
        let result = call(&path, "read_query", json!({ "query": query }));
        let text = result["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("Only the first 1000 rows are shown"));
        assert!(!text.contains(&format!("\"i\": {}", MAX_ROWS + 1)));
    }

    #[test]
    fn unknown_methods_get_an_error() {
        let (_dir, path) = test_db();
        let reply = request(&path, 3, "resources/list", Value::Null);
        assert_eq!(reply["error"]["code"], -32601);
    }
}
