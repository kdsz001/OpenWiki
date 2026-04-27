use rusqlite::{functions::FunctionFlags, Connection};
use std::path::PathBuf;
use std::sync::Mutex;

pub struct Database {
    pub conn: Mutex<Connection>,
}

/// Insert a space between every adjacent CJK ideograph so that the
/// FTS5 unicode61 tokenizer treats each character as its own token.
/// Without this, a continuous Chinese sequence like "整理设计风格" is
/// indexed as a single mega-token and searches for "设计" miss it.
///
/// English words and digits pass through unchanged, since unicode61
/// already tokenizes them correctly at whitespace/punctuation.
///
/// Idempotent: applying it twice produces the same string.
pub fn cjk_segment(input: &str) -> String {
    fn is_cjk(c: char) -> bool {
        matches!(c,
            '\u{3400}'..='\u{4DBF}' |   // CJK Unified Ideographs Extension A
            '\u{4E00}'..='\u{9FFF}' |   // CJK Unified Ideographs
            '\u{F900}'..='\u{FAFF}' |   // CJK Compatibility Ideographs
            '\u{20000}'..='\u{2FFFF}'   // Extensions B-F (supplementary plane)
        )
    }
    let mut out = String::with_capacity(input.len() + input.len() / 4);
    let mut prev_cjk = false;
    for c in input.chars() {
        let cur_cjk = is_cjk(c);
        if cur_cjk && prev_cjk {
            out.push(' ');
        }
        out.push(c);
        prev_cjk = cur_cjk;
    }
    out
}

impl Database {
    /// Register the cjk_seg() SQL function on the given connection.
    /// Must be called once per Connection — used by both the on-disk
    /// and in-memory constructors so the FTS triggers can call it.
    fn register_functions(conn: &Connection) -> Result<(), Box<dyn std::error::Error>> {
        conn.create_scalar_function(
            "cjk_seg",
            1,
            FunctionFlags::SQLITE_DETERMINISTIC | FunctionFlags::SQLITE_UTF8,
            |ctx| {
                let input: String = ctx.get(0).unwrap_or_default();
                Ok(cjk_segment(&input))
            },
        )?;
        Ok(())
    }

    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let db_path = Self::get_db_path()?;

        // Ensure parent directory exists
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let conn = Connection::open(&db_path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        Self::register_functions(&conn)?;

        let db = Database {
            conn: Mutex::new(conn),
        };
        db.run_migrations()?;

        Ok(db)
    }

    /// Create an in-memory database for testing.
    #[cfg(test)]
    pub fn new_in_memory() -> Result<Self, Box<dyn std::error::Error>> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        Self::register_functions(&conn)?;
        let db = Database {
            conn: Mutex::new(conn),
        };
        db.run_migrations()?;
        Ok(db)
    }

    fn get_db_path() -> Result<PathBuf, Box<dyn std::error::Error>> {
        let data_dir = dirs::data_dir()
            .ok_or("Could not find data directory")?
            .join("com.openwiki.app");
        Ok(data_dir.join("openwiki.db"))
    }

    fn run_migrations(&self) -> Result<(), Box<dyn std::error::Error>> {
        let conn = self.conn.lock().map_err(|e| format!("Lock error: {}", e))?;

        let migration_sql = include_str!("migrations/001_initial.sql");
        conn.execute_batch(migration_sql)?;

        // Migration 002: Add user_note column (idempotent check)
        let has_user_note: bool = conn
            .prepare("SELECT COUNT(*) FROM pragma_table_info('captured_content') WHERE name = 'user_note'")?
            .query_row([], |row| row.get::<_, i32>(0))
            .map(|count| count > 0)
            .unwrap_or(false);

        if !has_user_note {
            let migration_002 = include_str!("migrations/002_add_user_note.sql");
            conn.execute_batch(migration_002)?;
            log::info!("Migration 002 applied: added user_note column");
        }

        // Migration 003: Add chat_messages table
        let has_chat_messages: bool = conn
            .prepare(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='chat_messages'",
            )?
            .query_row([], |row| row.get::<_, i32>(0))
            .map(|count| count > 0)
            .unwrap_or(false);

        if !has_chat_messages {
            let migration_003 = include_str!("migrations/003_add_chat_messages.sql");
            conn.execute_batch(migration_003)?;
            log::info!("Migration 003 applied: added chat_messages table");
        }

        // Migration 004: Add digest fields (digested_at, digest_action)
        let has_digested_at: bool = conn
            .prepare("SELECT COUNT(*) FROM pragma_table_info('captured_content') WHERE name = 'digested_at'")?
            .query_row([], |row| row.get::<_, i32>(0))
            .map(|count| count > 0)
            .unwrap_or(false);

        if !has_digested_at {
            let migration_004 = include_str!("migrations/004_add_digest_fields.sql");
            conn.execute_batch(migration_004)?;
            log::info!("Migration 004 applied: added digest fields");
        }

        // Migration 005: Add attention_insights table
        let has_attention_insights: bool = conn
            .prepare("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='attention_insights'")?
            .query_row([], |row| row.get::<_, i32>(0))
            .map(|count| count > 0)
            .unwrap_or(false);

        if !has_attention_insights {
            let migration_005 = include_str!("migrations/005_add_attention_insights.sql");
            conn.execute_batch(migration_005)?;
            log::info!("Migration 005 applied: added attention_insights table");
        }

        // Migration 006: Add summary and tags columns to captured_content
        let has_summary: bool = conn
            .prepare(
                "SELECT COUNT(*) FROM pragma_table_info('captured_content') WHERE name='summary'",
            )?
            .query_row([], |row| row.get::<_, i32>(0))
            .map(|c| c > 0)
            .unwrap_or(false);
        let has_tags: bool = conn
            .prepare(
                "SELECT COUNT(*) FROM pragma_table_info('captured_content') WHERE name='tags'",
            )?
            .query_row([], |row| row.get::<_, i32>(0))
            .map(|c| c > 0)
            .unwrap_or(false);

        if !has_summary || !has_tags {
            if !has_summary {
                conn.execute_batch("ALTER TABLE captured_content ADD COLUMN summary TEXT;")?;
            }
            if !has_tags {
                conn.execute_batch("ALTER TABLE captured_content ADD COLUMN tags TEXT;")?;
            }
            log::info!("Migration 006 applied: added summary/tags columns");
        }

        // Migration 007: Add digest column to captured_content
        let has_digest: bool = conn
            .prepare(
                "SELECT COUNT(*) FROM pragma_table_info('captured_content') WHERE name='digest'",
            )?
            .query_row([], |row| row.get::<_, i32>(0))
            .map(|c| c > 0)
            .unwrap_or(false);

        if !has_digest {
            conn.execute_batch("ALTER TABLE captured_content ADD COLUMN digest TEXT;")?;
            log::info!("Migration 007 applied: added digest column");
        }

        // Migration 008: Add wiki tables
        let has_wiki_pages: bool = conn
            .prepare(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='wiki_pages'",
            )?
            .query_row([], |row| row.get::<_, i32>(0))
            .map(|count| count > 0)
            .unwrap_or(false);

        if !has_wiki_pages {
            let migration_008 = include_str!("migrations/008_add_wiki.sql");
            conn.execute_batch(migration_008)?;
            log::info!("Migration 008 applied: added wiki tables");
        }

        // Migration 009: Add wiki hash columns to captured_content
        let has_wiki_compile_hash: bool = conn
            .prepare(
                "SELECT COUNT(*) FROM pragma_table_info('captured_content') WHERE name='wiki_compile_hash'",
            )?
            .query_row([], |row| row.get::<_, i32>(0))
            .map(|c| c > 0)
            .unwrap_or(false);

        if !has_wiki_compile_hash {
            conn.execute_batch(
                "ALTER TABLE captured_content ADD COLUMN wiki_compile_hash TEXT;
                 ALTER TABLE captured_content ADD COLUMN wiki_assessed_hash TEXT;",
            )?;
            log::info!("Migration 009 applied: added wiki hash columns");
        }

        // Migration 010: Add multi-turn chat tables
        let has_chat_sessions: bool = conn
            .prepare(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='wiki_chat_sessions'",
            )?
            .query_row([], |row| row.get::<_, i32>(0))
            .map(|count| count > 0)
            .unwrap_or(false);

        if !has_chat_sessions {
            let migration_010 = include_str!("migrations/010_add_chat_tables.sql");
            conn.execute_batch(migration_010)?;
            log::info!("Migration 010 applied: added chat session tables");
        }

        // Migration 011: Add source_message_id to wiki_pages
        let has_source_message_id: bool = conn
            .prepare(
                "SELECT COUNT(*) FROM pragma_table_info('wiki_pages') WHERE name='source_message_id'",
            )?
            .query_row([], |row| row.get::<_, i32>(0))
            .map(|c| c > 0)
            .unwrap_or(false);

        if !has_source_message_id {
            let migration_011 = include_str!("migrations/011_add_source_message_id.sql");
            conn.execute_batch(migration_011)?;
            log::info!("Migration 011 applied: added source_message_id to wiki_pages");
        }

        // Migration 012: Add clean_content column to captured_content
        let has_clean_content: bool = conn
            .prepare(
                "SELECT COUNT(*) FROM pragma_table_info('captured_content') WHERE name='clean_content'",
            )?
            .query_row([], |row| row.get::<_, i32>(0))
            .map(|c| c > 0)
            .unwrap_or(false);

        if !has_clean_content {
            conn.execute_batch("ALTER TABLE captured_content ADD COLUMN clean_content TEXT;")?;
            log::info!("Migration 012 applied: added clean_content column");
        }

        // Migration 013: Add locale columns to content and AI-generated tables
        let has_content_locale: bool = conn
            .prepare(
                "SELECT COUNT(*) FROM pragma_table_info('captured_content') WHERE name='locale'",
            )?
            .query_row([], |row| row.get::<_, i32>(0))
            .map(|c| c > 0)
            .unwrap_or(false);

        if !has_content_locale {
            conn.execute_batch(
                "ALTER TABLE captured_content ADD COLUMN locale TEXT NOT NULL DEFAULT 'zh-CN';",
            )?;
            // weekly_reports may not exist yet in some setups, so check first
            let has_reports: bool = conn
                .prepare("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='weekly_reports'")
                .and_then(|mut s| s.query_row([], |row| row.get::<_, i32>(0)))
                .map(|c| c > 0)
                .unwrap_or(false);
            if has_reports {
                conn.execute_batch(
                    "ALTER TABLE weekly_reports ADD COLUMN locale TEXT NOT NULL DEFAULT 'zh-CN';",
                )?;
            }
            let has_insights: bool = conn
                .prepare("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='attention_insights'")
                .and_then(|mut s| s.query_row([], |row| row.get::<_, i32>(0)))
                .map(|c| c > 0)
                .unwrap_or(false);
            if has_insights {
                conn.execute_batch(
                    "ALTER TABLE attention_insights ADD COLUMN locale TEXT NOT NULL DEFAULT 'zh-CN';",
                )?;
            }
            let has_wiki: bool = conn
                .prepare("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='wiki_pages'")
                .and_then(|mut s| s.query_row([], |row| row.get::<_, i32>(0)))
                .map(|c| c > 0)
                .unwrap_or(false);
            if has_wiki {
                conn.execute_batch(
                    "ALTER TABLE wiki_pages ADD COLUMN locale TEXT NOT NULL DEFAULT 'zh-CN';",
                )?;
            }
            log::info!("Migration 013 applied: added locale columns");
        }

        // Migration 014: Add FTS5 virtual table for wiki_pages.
        // Wrapped in fallible block — if FTS5 is unavailable in the sqlite
        // build (shouldn't happen with rusqlite "bundled"), we log and
        // continue in degraded mode. The repository layer will detect the
        // missing table and fall back to LIKE-based search.
        let has_wiki_fts: bool = conn
            .prepare("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='wiki_pages_fts'")?
            .query_row([], |row| row.get::<_, i32>(0))
            .map(|c| c > 0)
            .unwrap_or(false);

        if !has_wiki_fts {
            let migration_014 = include_str!("migrations/014_add_wiki_fts.sql");
            match conn.execute_batch(migration_014) {
                Ok(_) => log::info!("Migration 014 applied: added wiki_pages_fts"),
                Err(e) => log::warn!(
                    "Migration 014 skipped (FTS5 unavailable, falling back to LIKE search): {}",
                    e
                ),
            }
        }

        // Migration 015: rebuild FTS with CJK character segmentation.
        // We detect "already applied" by checking whether the trigger
        // body references cjk_seg — that's the marker of v2 layout.
        let needs_015: bool = conn
            .prepare(
                "SELECT COUNT(*) FROM sqlite_master \
                 WHERE type='trigger' AND name='wiki_pages_fts_insert' \
                 AND sql LIKE '%cjk_seg%'",
            )
            .and_then(|mut s| s.query_row([], |row| row.get::<_, i32>(0)))
            .map(|c| c == 0)
            .unwrap_or(true);

        if needs_015 {
            let migration_015 = include_str!("migrations/015_wiki_fts_cjk_segment.sql");
            match conn.execute_batch(migration_015) {
                Ok(_) => log::info!(
                    "Migration 015 applied: rebuilt wiki_pages_fts with CJK segmentation"
                ),
                Err(e) => log::warn!(
                    "Migration 015 skipped (FTS5 unavailable, keeping legacy index): {}",
                    e
                ),
            }
        }

        // Migration 016: bump stale Anthropic model IDs to current 4.X
        // family. The old dated IDs (claude-sonnet-4-20250514 etc.) are
        // discontinued — leaving them as the saved default would cause
        // every API call to fail with "model not found". We rewrite
        // exact matches only; user-chosen custom IDs are left alone.
        let _ = conn.execute(
            "UPDATE app_settings SET value = 'claude-sonnet-4-6' \
             WHERE key = 'ai_model' AND value = 'claude-sonnet-4-20250514'",
            [],
        );
        let _ = conn.execute(
            "UPDATE app_settings SET value = 'claude-opus-4-7' \
             WHERE key = 'ai_model' AND value = 'claude-opus-4-20250514'",
            [],
        );
        let _ = conn.execute(
            "UPDATE app_settings SET value = 'claude-haiku-4-5-20251001' \
             WHERE key = 'ai_model' AND value = 'claude-3-5-haiku-20241022'",
            [],
        );

        log::info!("Database migrations completed successfully");
        Ok(())
    }
}
