use rusqlite::Connection;
use std::path::PathBuf;
use std::sync::Mutex;

pub struct Database {
    pub conn: Mutex<Connection>,
}

impl Database {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let db_path = Self::get_db_path()?;

        // Ensure parent directory exists
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let conn = Connection::open(&db_path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;

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
            .prepare("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='wiki_pages'")?
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
                .prepare(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='wiki_pages'",
                )
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

        // Migration 014: Add attention batch cache table
        let has_attention_batch_cache: bool = conn
            .prepare(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='attention_batch_cache'",
            )?
            .query_row([], |row| row.get::<_, i32>(0))
            .map(|count| count > 0)
            .unwrap_or(false);

        if !has_attention_batch_cache {
            let migration_014 = include_str!("migrations/014_add_attention_batch_cache.sql");
            conn.execute_batch(migration_014)?;
            log::info!("Migration 014 applied: added attention_batch_cache table");
        }

        // Migration 015: Add structured concept candidate cache for imported content
        let has_content_concept_candidates: bool = conn
            .prepare(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='content_concept_candidates'",
            )?
            .query_row([], |row| row.get::<_, i32>(0))
            .map(|count| count > 0)
            .unwrap_or(false);

        if !has_content_concept_candidates {
            let migration_015 = include_str!("migrations/015_add_content_concept_candidates.sql");
            conn.execute_batch(migration_015)?;
            log::info!("Migration 015 applied: added content concept candidates table");
        }

        let malformed_byte_size_count: i64 = conn
            .prepare("SELECT COUNT(*) FROM captured_content WHERE typeof(byte_size) = 'text'")?
            .query_row([], |row| row.get(0))
            .unwrap_or(0);

        if malformed_byte_size_count > 0 {
            conn.execute(
                "UPDATE captured_content
                 SET byte_size = LENGTH(COALESCE(raw_text, ''))
                 WHERE typeof(byte_size) = 'text'",
                [],
            )?;
            log::info!(
                "Migration repair applied: normalized {} malformed byte_size values",
                malformed_byte_size_count
            );
        }

        log::info!("Database migrations completed successfully");
        Ok(())
    }
}
