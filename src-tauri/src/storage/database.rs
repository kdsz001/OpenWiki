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
        conn.create_scalar_function(
            "contains_sensitive",
            1,
            FunctionFlags::SQLITE_DETERMINISTIC | FunctionFlags::SQLITE_UTF8,
            |ctx| {
                let input: String = ctx.get(0).unwrap_or_default();
                Ok(crate::capture::sensitive_filter::contains_sensitive_data(
                    &input,
                ))
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

    pub(crate) fn get_db_path() -> Result<PathBuf, Box<dyn std::error::Error>> {
        let data_dir = super::data_location::data_dir()?;
        Ok(data_dir.join(super::data_location::DB_FILE))
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

        // Migration 014: Add FTS5 virtual table for wiki_pages.
        // Wrapped in fallible block — if FTS5 is unavailable in the sqlite
        // build (shouldn't happen with rusqlite "bundled"), we log and
        // continue in degraded mode. The repository layer will detect the
        // missing table and fall back to LIKE-based search.
        let has_wiki_fts: bool = conn
            .prepare(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='wiki_pages_fts'",
            )?
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

        // Migration 017: Add category column to captured_content
        let has_category: bool = conn
            .prepare(
                "SELECT COUNT(*) FROM pragma_table_info('captured_content') WHERE name='category'",
            )?
            .query_row([], |row| row.get::<_, i32>(0))
            .map(|c| c > 0)
            .unwrap_or(false);

        if !has_category {
            let migration_017 = include_str!("migrations/017_add_category.sql");
            conn.execute_batch(migration_017)?;
            log::info!("Migration 017 applied: added category column");
        }

        // Migration 018: move saved retired preset models to supported IDs.
        let migration_018 = include_str!("migrations/018_migrate_stale_ai_models.sql");
        conn.execute_batch(migration_018)?;

        // Migration 019: refresh provider-specific model aliases with guarded active settings.
        let migration_019 = include_str!("migrations/019_refresh_ai_model_aliases.sql");
        conn.execute_batch(migration_019)?;

        log::info!("Database migrations completed successfully");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set_migration_test_setting(db: &Database, key: &str, value: &str) {
        db.conn
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO app_settings (key, value, updated_at) \
                 VALUES (?1, ?2, '2000-01-01 00:00:00') \
                 ON CONFLICT(key) DO UPDATE \
                 SET value = excluded.value, updated_at = excluded.updated_at",
                [key, value],
            )
            .unwrap();
    }

    fn migration_test_setting(db: &Database, key: &str) -> (String, String) {
        db.conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT value, updated_at FROM app_settings WHERE key = ?1",
                [key],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap()
    }

    #[test]
    fn migration_019_refreshes_active_and_saved_provider_models() {
        let cases = [
            (
                "openrouter",
                "ai_model_openrouter",
                "qwen/qwen3.8-max",
                "qwen/qwen3.8-max-0902",
            ),
            (
                "deepseek",
                "ai_model_deepseek",
                "deepseek-v4-flash",
                "deepseek-flash",
            ),
            (
                "deepseek",
                "ai_model_deepseek",
                "deepseek-v4-flash-vision-exp",
                "deepseek-flash",
            ),
        ];

        for (provider, saved_key, stale_model, expected_model) in cases {
            for (active_provider, active_model, saved_model) in [
                (provider, stale_model, stale_model),
                ("anthropic", stale_model, stale_model),
                (provider, "active-custom-model", stale_model),
                (provider, stale_model, "saved-custom-model"),
            ] {
                let db = Database::new_in_memory().unwrap();
                set_migration_test_setting(&db, "ai_provider", active_provider);
                set_migration_test_setting(&db, "ai_model", active_model);
                set_migration_test_setting(&db, saved_key, saved_model);

                db.run_migrations().unwrap();

                let saved = migration_test_setting(&db, saved_key);
                if saved_model == stale_model {
                    assert_eq!(saved.0, expected_model);
                    assert_ne!(saved.1, "2000-01-01 00:00:00");
                } else {
                    assert_eq!(saved, (saved_model.into(), "2000-01-01 00:00:00".into()));
                }
                let active = migration_test_setting(&db, "ai_model");
                if active_provider == provider && active_model == stale_model {
                    assert_eq!(active.0, expected_model);
                    assert_ne!(active.1, "2000-01-01 00:00:00");
                } else {
                    assert_eq!(active, (active_model.into(), "2000-01-01 00:00:00".into()));
                }
                assert_eq!(
                    migration_test_setting(&db, "ai_provider"),
                    (active_provider.into(), "2000-01-01 00:00:00".into())
                );
            }
        }
    }

    #[test]
    fn migration_019_preserves_aliases_for_other_providers_and_keys() {
        let cases = [
            ("openrouter", "qwen/qwen3.8-max", "ai_model_deepseek"),
            ("deepseek", "deepseek-v4-flash", "ai_model_openrouter"),
            (
                "deepseek",
                "deepseek-v4-flash-vision-exp",
                "ai_model_openrouter",
            ),
        ];
        for (model_provider, stale_model, other_provider_key) in cases {
            for provider in [
                Some("openrouter"),
                Some("deepseek"),
                Some("anthropic"),
                Some("openai"),
                Some("gemini"),
                Some("dashscope"),
                Some("custom"),
                Some(""),
                None,
            ] {
                if provider == Some(model_provider) {
                    continue;
                }
                let db = Database::new_in_memory().unwrap();
                if let Some(provider) = provider {
                    set_migration_test_setting(&db, "ai_provider", provider);
                } else {
                    db.conn
                        .lock()
                        .unwrap()
                        .execute("DELETE FROM app_settings WHERE key = 'ai_provider'", [])
                        .unwrap();
                }
                let keys = [
                    "ai_model",
                    other_provider_key,
                    "ai_model_dashscope",
                    "ai_model_custom",
                    "ai_model_openai",
                    "ai_model_anthropic",
                    "ai_model_gemini",
                    "ai_api_key",
                    "unrelated_setting",
                ];
                for key in keys {
                    set_migration_test_setting(&db, key, stale_model);
                }

                db.run_migrations().unwrap();

                for key in keys {
                    assert_eq!(
                        migration_test_setting(&db, key),
                        (stale_model.into(), "2000-01-01 00:00:00".into()),
                        "unexpected change to {key} for provider {provider:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn migration_019_preserves_custom_and_current_models() {
        for provider in ["openrouter", "deepseek", "dashscope", "custom"] {
            for model in [
                "qwen/qwen3.8-max-custom",
                "deepseek-v4-flash-custom",
                "deepseek-v4-flash-vision-exp-custom",
                "my-private-model",
                "qwen/qwen3.8-max-0902",
                "deepseek-flash",
            ] {
                let db = Database::new_in_memory().unwrap();
                set_migration_test_setting(&db, "ai_provider", provider);
                let keys = ["ai_model", "ai_model_openrouter", "ai_model_deepseek"];
                for key in keys {
                    set_migration_test_setting(&db, key, model);
                }

                db.run_migrations().unwrap();

                for key in keys {
                    assert_eq!(
                        migration_test_setting(&db, key),
                        (model.into(), "2000-01-01 00:00:00".into())
                    );
                }
            }
        }
    }

    #[test]
    fn migration_019_does_not_rewrite_updated_at_on_repeat() {
        for (provider, saved_key, model, expected) in [
            (
                "openrouter",
                "ai_model_openrouter",
                "qwen/qwen3.8-max",
                "qwen/qwen3.8-max-0902",
            ),
            (
                "deepseek",
                "ai_model_deepseek",
                "deepseek-v4-flash",
                "deepseek-flash",
            ),
            (
                "deepseek",
                "ai_model_deepseek",
                "deepseek-v4-flash-vision-exp",
                "deepseek-flash",
            ),
        ] {
            let db = Database::new_in_memory().unwrap();
            set_migration_test_setting(&db, "ai_provider", provider);
            set_migration_test_setting(&db, "ai_model", model);
            set_migration_test_setting(&db, saved_key, model);
            db.run_migrations().unwrap();
            let active = migration_test_setting(&db, "ai_model");
            let saved = migration_test_setting(&db, saved_key);
            assert_eq!(active.0, expected);
            assert_eq!(saved.0, expected);

            // Reject any repeated write, even if datetime('now') returns the same second.
            db.conn
                .lock()
                .unwrap()
                .execute_batch(
                    "CREATE TRIGGER reject_repeated_model_updates \
                     BEFORE UPDATE ON app_settings \
                     WHEN OLD.key IN ('ai_model', 'ai_model_openrouter', 'ai_model_deepseek') \
                     BEGIN SELECT RAISE(ABORT, 'model settings must not be rewritten'); END;",
                )
                .unwrap();

            db.run_migrations().unwrap();

            assert_eq!(migration_test_setting(&db, "ai_model"), active);
            assert_eq!(migration_test_setting(&db, saved_key), saved);
        }
    }

    #[test]
    fn migration_018_replaces_stale_provider_models() {
        let db = Database::new_in_memory().unwrap();
        let migration = include_str!("migrations/018_migrate_stale_ai_models.sql");
        let cases = [
            (
                "ai_model_anthropic",
                "claude-opus-4-20250514",
                "claude-opus-4-8",
            ),
            ("ai_model_openai", "gpt-5.2-codex", "gpt-5.6"),
            ("ai_model_openai", "gpt-5.1-codex-max", "gpt-5.6"),
            ("ai_model_openai", "gpt-5.1-codex", "gpt-5.6"),
            ("ai_model_openai", "gpt-5.1-codex-mini", "gpt-5.6-terra"),
            (
                "ai_model_openrouter",
                "openai/gpt-oss-120b:free",
                "openrouter/free",
            ),
            (
                "ai_model_openrouter",
                "google/gemini-3-pro-preview",
                "google/gemini-3.1-pro-preview",
            ),
            (
                "ai_model_openrouter",
                "deepseek/deepseek-v3.2-speciale",
                "deepseek/deepseek-v4-flash",
            ),
            (
                "ai_model_openrouter",
                "x-ai/grok-4.1-fast",
                "x-ai/grok-4.20",
            ),
        ];

        for (key, stale_model, expected_model) in cases {
            {
                let conn = db.conn.lock().unwrap();
                conn.execute(
                    "INSERT INTO app_settings (key, value) VALUES (?1, ?2) \
                     ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                    [key, stale_model],
                )
                .unwrap();
                conn.execute_batch(migration).unwrap();
            }

            let actual: String = db
                .conn
                .lock()
                .unwrap()
                .query_row(
                    "SELECT value FROM app_settings WHERE key = ?1",
                    [key],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(actual, expected_model, "failed to migrate {stale_model}");
        }
    }
}
