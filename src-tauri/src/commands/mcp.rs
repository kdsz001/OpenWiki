use crate::commands::capture::AppState;
use crate::storage::models::CapturedContent;
use crate::storage::repository::Repository;
use serde::Serialize;
use std::path::PathBuf;
use tauri::State;

/// Fallback preview when AI summary is not available.
fn fallback_preview(item: &CapturedContent) -> String {
    if let Some(ref url) = item.source_url {
        if !url.is_empty() {
            return url.clone();
        }
    }
    if let Some(ref text) = item.raw_text {
        if !text.is_empty() {
            let preview: String = text.chars().take(80).collect();
            return preview.replace('\n', " ");
        }
    }
    "[Image]".to_string()
}

// ─── MCP Integration ──────────────────────────────────────────────
//
//  Data flow:
//
//  User clicks [连接 Claude Desktop]
//       │
//       ├─ get_mcp_status()        → read config file, check for "openwiki" key
//       ├─ connect_mcp()           → backup + inject + write config
//       └─ disconnect_mcp()        → read + remove "openwiki" key + write
//
//  The entry makes the AI app start this OpenWiki with --mcp, the read-only server in
//  mcp_server.rs. Config file: ~/Library/Application Support/Claude/claude_desktop_config.json

const MCP_SERVER_KEY: &str = "openwiki";

#[derive(Debug, Clone, Copy, PartialEq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpTarget {
    Claude,
    Openclaw,
}

impl McpTarget {
    fn config_path(&self) -> Option<PathBuf> {
        match self {
            McpTarget::Claude => {
                let base = dirs::data_dir()?;
                Some(base.join("Claude").join("claude_desktop_config.json"))
            }
            McpTarget::Openclaw => {
                let home = dirs::home_dir()?;
                Some(home.join(".openclaw").join("openclaw.json"))
            }
        }
    }

    fn display_name(&self) -> &str {
        match self {
            McpTarget::Claude => "Claude Desktop",
            McpTarget::Openclaw => "OpenClaw",
        }
    }

    fn process_name(&self) -> &str {
        match self {
            McpTarget::Claude => "Claude",
            McpTarget::Openclaw => "openclaw",
        }
    }
}

#[derive(Serialize)]
pub struct McpStatus {
    pub connected: bool,
    pub installed: bool,
    pub config_path: Option<String>,
}

/// The config entry that has the AI app start this OpenWiki as its read-only MCP server.
fn mcp_server_entry() -> Result<serde_json::Value, String> {
    let exe = std::env::current_exe().map_err(|e| format!("Cannot locate OpenWiki: {}", e))?;
    Ok(serde_json::json!({
        "command": exe.to_string_lossy(),
        "args": [crate::mcp_server::MCP_ARG]
    }))
}

/// Updates the OpenWiki entry in every AI app config that has one, when it differs from what
/// this copy writes: entries from before 0.3.27 ran a SQLite server that could also change and
/// delete data, and the path goes stale when OpenWiki moves.
#[cfg_attr(debug_assertions, allow(dead_code))]
pub fn refresh_connected_entries() {
    let entry = match mcp_server_entry() {
        Ok(entry) => entry,
        Err(e) => {
            log::warn!("[mcp] {}", e);
            return;
        }
    };
    for target in [McpTarget::Claude, McpTarget::Openclaw] {
        let Some(path) = target.config_path() else {
            continue;
        };
        match refresh_entry_in(&path, &entry) {
            Ok(true) => log::info!(
                "[mcp] {} now starts OpenWiki's read-only server",
                target.display_name()
            ),
            Ok(false) => {}
            Err(e) => log::warn!(
                "[mcp] failed to update the {} config: {}",
                target.display_name(),
                e
            ),
        }
    }
}

/// Replaces an existing OpenWiki entry in one config file; returns whether it changed.
fn refresh_entry_in(path: &PathBuf, entry: &serde_json::Value) -> Result<bool, String> {
    if !path.exists() {
        return Ok(false);
    }
    let mut config = read_config(path)?;
    let Some(current) = config
        .get_mut("mcpServers")
        .and_then(|servers| servers.get_mut(MCP_SERVER_KEY))
    else {
        return Ok(false);
    };
    if current == entry {
        return Ok(false);
    }
    *current = entry.clone();
    backup_config(path)?;
    write_config(path, &config)?;
    Ok(true)
}

/// Check if a process is running by name.
fn is_process_running(name: &str) -> bool {
    #[cfg(target_os = "windows")]
    {
        return std::process::Command::new("tasklist")
            .output()
            .map(|o| {
                let stdout = String::from_utf8_lossy(&o.stdout).to_lowercase();
                stdout.contains(&name.to_lowercase())
            })
            .unwrap_or(false);
    }

    #[cfg(not(target_os = "windows"))]
    {
        std::process::Command::new("pgrep")
            .args(["-xi", name])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
}

/// Read and parse the Claude Desktop config file.
fn read_config(path: &PathBuf) -> Result<serde_json::Value, String> {
    let content =
        std::fs::read_to_string(path).map_err(|e| format!("Cannot read config file: {}", e))?;
    serde_json::from_str(&content)
        .map_err(|e| format!("Config file format error (invalid JSON): {}", e))
}

/// Write the config back to file.
fn write_config(path: &PathBuf, config: &serde_json::Value) -> Result<(), String> {
    let content = serde_json::to_string_pretty(config)
        .map_err(|e| format!("JSON serialization failed: {}", e))?;
    std::fs::write(path, content).map_err(|e| format!("Cannot write config file: {}", e))
}

/// Create a timestamped backup of the config file.
fn backup_config(path: &PathBuf) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    let timestamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let backup_path = path.with_extension(format!("json.bak.{}", timestamp));
    std::fs::copy(path, &backup_path).map_err(|e| format!("Backup failed: {}", e))?;
    log::info!("Config backed up to {:?}", backup_path);
    Ok(())
}

#[tauri::command]
pub async fn get_mcp_status(target: McpTarget) -> Result<McpStatus, String> {
    let config_path = target.config_path();
    let installed = config_path
        .as_ref()
        .map(|p| p.parent().map(|d| d.exists()).unwrap_or(false))
        .unwrap_or(false);

    let connected = if let Some(ref path) = config_path {
        if path.exists() {
            read_config(path)
                .ok()
                .and_then(|c| c.get("mcpServers")?.get(MCP_SERVER_KEY).cloned())
                .is_some()
        } else {
            false
        }
    } else {
        false
    };

    Ok(McpStatus {
        connected,
        installed,
        config_path: config_path.map(|p| p.to_string_lossy().to_string()),
    })
}

#[tauri::command]
pub async fn connect_mcp(target: McpTarget) -> Result<String, String> {
    let name = target.display_name();

    // 1. Check config directory
    let config_path = target
        .config_path()
        .ok_or(format!("Cannot determine {} config path", name))?;
    let config_dir = config_path
        .parent()
        .ok_or(format!("Cannot determine {} config directory", name))?;

    if !config_dir.exists() {
        // For OpenClaw, create the directory if it doesn't exist
        if target == McpTarget::Openclaw {
            std::fs::create_dir_all(config_dir)
                .map_err(|e| format!("Cannot create OpenClaw config directory: {}", e))?;
        } else {
            return Err(format!(
                "{} is not installed. Please install {} first.",
                name, name
            ));
        }
    }

    // 2. The entry that starts this OpenWiki as the read-only server
    let entry = mcp_server_entry()?;

    // 3. Read or create config
    let mut config = if config_path.exists() {
        // Backup first
        backup_config(&config_path)?;
        let c = read_config(&config_path)?;
        if !c.is_object() {
            return Err("Config file format error: not a valid JSON object".to_string());
        }
        c
    } else {
        serde_json::json!({})
    };

    // 4. Inject the OpenWiki MCP entry
    let mcp_servers = config
        .as_object_mut()
        .ok_or("Config is not a JSON object")?
        .entry("mcpServers")
        .or_insert_with(|| serde_json::json!({}));

    if !mcp_servers.is_object() {
        *mcp_servers = serde_json::json!({});
    }

    mcp_servers
        .as_object_mut()
        .unwrap()
        .insert(MCP_SERVER_KEY.to_string(), entry);

    // 5. Write back
    write_config(&config_path, &config)?;

    // 6. Check if the target app is running
    let msg = if is_process_running(target.process_name()) {
        format!(
            "Connected! Please quit and reopen {} for changes to take effect.",
            name
        )
    } else {
        format!(
            "Connected! Changes will take effect the next time you open {}.",
            name
        )
    };

    log::info!("MCP connected: openwiki entry added to {} config", name);
    Ok(msg)
}

#[tauri::command]
pub async fn disconnect_mcp(target: McpTarget) -> Result<(), String> {
    let config_path = target.config_path().ok_or(format!(
        "Cannot determine {} config path",
        target.display_name()
    ))?;

    if !config_path.exists() {
        return Ok(()); // Nothing to disconnect
    }

    let mut config = read_config(&config_path)?;

    // Surgically remove only the xiaoyun entry
    if let Some(servers) = config.get_mut("mcpServers").and_then(|s| s.as_object_mut()) {
        if servers.remove(MCP_SERVER_KEY).is_some() {
            backup_config(&config_path)?;
            write_config(&config_path, &config)?;
            log::info!("MCP disconnected: xiaoyun entry removed");
        }
    }

    Ok(())
}

#[tauri::command]
pub async fn copy_content_summary(state: State<'_, AppState>) -> Result<(), String> {
    let repo = Repository::new(state.db.clone());

    // Get last 7 days of content
    let now = chrono::Local::now();
    let week_ago = now - chrono::Duration::days(7);
    let contents = repo
        .get_content_for_week(
            &week_ago.format("%Y-%m-%dT00:00:00").to_string(),
            &now.format("%Y-%m-%dT23:59:59").to_string(),
        )
        .map_err(|e| e.to_string())?;

    let text = if contents.is_empty() {
        "No saved content in the last 7 days.".to_string()
    } else {
        let total = contents.len();
        let mut lines = Vec::new();
        lines.push(format!(
            "Here is my saved content from the last 7 days ({} items):\n",
            total
        ));

        for (i, item) in contents.iter().enumerate() {
            let date = &item.captured_at[..10];
            let source = &item.source_app;
            let content_type = item.content_type.as_str();

            // Use AI summary if available, otherwise fall back to a short preview
            let description = if let Some(ref summary) = item.summary {
                if !summary.is_empty() {
                    summary.clone()
                } else {
                    fallback_preview(item)
                }
            } else {
                fallback_preview(item)
            };

            let tags = item.tags.as_deref().unwrap_or("");

            if tags.is_empty() {
                lines.push(format!(
                    "{}. [{}] [{}] from {}: {}",
                    i + 1,
                    date,
                    content_type,
                    source,
                    description
                ));
            } else {
                lines.push(format!(
                    "{}. [{}] [{}] from {}: {} ({})",
                    i + 1,
                    date,
                    content_type,
                    source,
                    description,
                    tags
                ));
            }
        }

        lines.push("\nPlease help me organize and analyze this content.".to_string());
        lines.join("\n")
    };

    crate::capture::clipboard::write_text_suppressed(&text)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn make_temp_config(content: &str) -> NamedTempFile {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(content.as_bytes()).unwrap();
        f
    }

    #[test]
    fn test_read_valid_config() {
        let f = make_temp_config(r#"{"mcpServers": {}}"#);
        let config = read_config(&f.path().to_path_buf()).unwrap();
        assert!(config.get("mcpServers").is_some());
    }

    #[test]
    fn test_read_invalid_json() {
        let f = make_temp_config("not json at all");
        let result = read_config(&f.path().to_path_buf());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("invalid JSON"));
    }

    #[test]
    fn test_read_nonexistent_file() {
        let result = read_config(&PathBuf::from("/tmp/nonexistent-xiaoyun-test.json"));
        assert!(result.is_err());
    }

    #[test]
    fn test_write_and_read_config() {
        let f = make_temp_config("{}");
        let path = f.path().to_path_buf();
        let config = serde_json::json!({"mcpServers": {"test": {"command": "echo"}}});
        write_config(&path, &config).unwrap();
        let read_back = read_config(&path).unwrap();
        assert_eq!(read_back["mcpServers"]["test"]["command"], "echo");
    }

    #[test]
    fn test_backup_creates_timestamped_file() {
        let f = make_temp_config(r#"{"original": true}"#);
        let path = f.path().to_path_buf();
        backup_config(&path).unwrap();
        // Check that a .bak.YYYYMMDD file was created in the same directory
        let dir = path.parent().unwrap();
        let bak_files: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".bak."))
            .collect();
        assert!(!bak_files.is_empty());
    }

    #[test]
    fn test_inject_xiaoyun_entry_new_config() {
        let mut config = serde_json::json!({});
        let servers = config
            .as_object_mut()
            .unwrap()
            .entry("mcpServers")
            .or_insert_with(|| serde_json::json!({}));
        servers.as_object_mut().unwrap().insert(
            MCP_SERVER_KEY.to_string(),
            serde_json::json!({"command": "npx", "args": ["-y", "mcp-server-sqlite-npx"]}),
        );
        assert!(config["mcpServers"][MCP_SERVER_KEY]["command"] == "npx");
    }

    #[test]
    fn test_inject_preserves_existing_entries() {
        let mut config = serde_json::json!({
            "mcpServers": {
                "other-tool": {"command": "other"}
            }
        });
        config["mcpServers"].as_object_mut().unwrap().insert(
            MCP_SERVER_KEY.to_string(),
            serde_json::json!({"command": "npx"}),
        );
        // Both entries should exist
        assert!(config["mcpServers"]["other-tool"]["command"] == "other");
        assert!(config["mcpServers"][MCP_SERVER_KEY]["command"] == "npx");
    }

    #[test]
    fn test_remove_xiaoyun_entry() {
        let mut config = serde_json::json!({
            "mcpServers": {
                MCP_SERVER_KEY: {"command": "npx"},
                "other-tool": {"command": "other"}
            }
        });
        if let Some(servers) = config.get_mut("mcpServers").and_then(|s| s.as_object_mut()) {
            servers.remove(MCP_SERVER_KEY);
        }
        assert!(config["mcpServers"].get(MCP_SERVER_KEY).is_none());
        assert!(config["mcpServers"]["other-tool"]["command"] == "other");
    }

    #[test]
    fn test_remove_from_empty_servers() {
        let mut config = serde_json::json!({"mcpServers": {}});
        if let Some(servers) = config.get_mut("mcpServers").and_then(|s| s.as_object_mut()) {
            servers.remove(MCP_SERVER_KEY); // Should not panic
        }
        assert!(config["mcpServers"].as_object().unwrap().is_empty());
    }

    #[test]
    fn test_entry_starts_this_openwiki_as_mcp_server() {
        let entry = mcp_server_entry().unwrap();
        let command = PathBuf::from(entry["command"].as_str().unwrap());
        assert!(command.is_absolute(), "command should be absolute: {:?}", command);
        assert_eq!(entry["args"], serde_json::json!(["--mcp"]));
    }

    #[test]
    fn test_refresh_replaces_old_read_write_entry() {
        let f = make_temp_config(
            r#"{"mcpServers": {
                "openwiki": {"command": "/usr/local/bin/npx", "args": ["-y", "mcp-server-sqlite-npx", "/x/openwiki.db"]},
                "other-tool": {"command": "other"}
            }}"#,
        );
        let path = f.path().to_path_buf();
        let entry = mcp_server_entry().unwrap();

        assert!(refresh_entry_in(&path, &entry).unwrap());
        let config = read_config(&path).unwrap();
        assert_eq!(config["mcpServers"][MCP_SERVER_KEY], entry);
        assert_eq!(config["mcpServers"]["other-tool"]["command"], "other");

        // Already current: nothing to write.
        assert!(!refresh_entry_in(&path, &entry).unwrap());
    }

    #[test]
    fn test_refresh_leaves_unconnected_configs_alone() {
        let f = make_temp_config(r#"{"mcpServers": {"other-tool": {"command": "other"}}}"#);
        let path = f.path().to_path_buf();
        let entry = mcp_server_entry().unwrap();

        assert!(!refresh_entry_in(&path, &entry).unwrap());
        assert!(read_config(&path).unwrap()["mcpServers"].get(MCP_SERVER_KEY).is_none());
        assert!(!refresh_entry_in(&PathBuf::from("/tmp/nonexistent-openwiki-mcp.json"), &entry).unwrap());
    }

    #[test]
    fn test_config_no_mcp_servers_key() {
        let mut config = serde_json::json!({"someOtherKey": true});
        let servers = config
            .as_object_mut()
            .unwrap()
            .entry("mcpServers")
            .or_insert_with(|| serde_json::json!({}));
        servers.as_object_mut().unwrap().insert(
            MCP_SERVER_KEY.to_string(),
            serde_json::json!({"command": "npx"}),
        );
        assert!(config["mcpServers"][MCP_SERVER_KEY]["command"] == "npx");
        assert!(config["someOtherKey"] == true);
    }

    #[test]
    fn test_full_roundtrip() {
        let f = make_temp_config(r#"{"mcpServers": {"existing": {"command": "foo"}}}"#);
        let path = f.path().to_path_buf();

        // Connect
        let mut config = read_config(&path).unwrap();
        config["mcpServers"].as_object_mut().unwrap().insert(
            MCP_SERVER_KEY.to_string(),
            serde_json::json!({"command": "npx"}),
        );
        write_config(&path, &config).unwrap();

        // Verify connected
        let config = read_config(&path).unwrap();
        assert!(config["mcpServers"][MCP_SERVER_KEY].is_object());
        assert!(config["mcpServers"]["existing"].is_object());

        // Disconnect
        let mut config = read_config(&path).unwrap();
        config["mcpServers"]
            .as_object_mut()
            .unwrap()
            .remove(MCP_SERVER_KEY);
        write_config(&path, &config).unwrap();

        // Verify disconnected
        let config = read_config(&path).unwrap();
        assert!(config["mcpServers"].get(MCP_SERVER_KEY).is_none());
        assert!(config["mcpServers"]["existing"]["command"] == "foo");
    }
}
