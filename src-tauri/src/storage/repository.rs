use super::database::Database;
use super::models::{
    AttentionInsight, CapturedContent, ContentForAnalysis, ContentType, ReportSection,
    UserFeedback, UserPreference, WeeklyReport,
};
use crate::secure_store;
use rusqlite::{params, OptionalExtension};
use rusqlite::{params_from_iter, types::Value};
use serde_json;
use std::collections::HashMap;
use std::sync::Arc;

pub struct Repository {
    db: Arc<Database>,
}

pub(crate) fn parse_page_reference_ids(raw: &str) -> Vec<String> {
    serde_json::from_str::<serde_json::Value>(raw)
        .ok()
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|item| match item {
            serde_json::Value::String(id) => Some(id),
            serde_json::Value::Object(object) => object
                .get("id")
                .and_then(|id| id.as_str())
                .map(str::to_string),
            _ => None,
        })
        .filter(|id| !id.is_empty())
        .collect()
}

impl Repository {
    pub fn new(db: Arc<Database>) -> Self {
        Repository { db }
    }

    // ========== Captured Content ==========

    pub fn save_content(
        &self,
        content: &CapturedContent,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        conn.execute(
            "INSERT INTO captured_content (id, content_type, raw_text, image_path, thumbnail_path, source_app, source_bundle_id, source_url, user_note, captured_at, content_hash, byte_size)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                content.id,
                content.content_type.as_str(),
                content.raw_text,
                content.image_path,
                content.thumbnail_path,
                content.source_app,
                content.source_bundle_id,
                content.source_url,
                content.user_note,
                content.captured_at,
                content.content_hash,
                content.byte_size,
            ],
        )?;
        Ok(())
    }

    pub fn get_all_content(
        &self,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<CapturedContent>, Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, content_type, raw_text, image_path, thumbnail_path, source_app, source_bundle_id, source_url, user_note, captured_at, content_hash, byte_size, is_deleted, created_at, updated_at, digested_at, digest_action, summary, tags, digest, wiki_compile_hash, wiki_assessed_hash, clean_content, category
             FROM captured_content WHERE is_deleted = 0 ORDER BY captured_at DESC LIMIT ?1 OFFSET ?2"
        )?;

        let rows = stmt.query_map(params![limit, offset], |row| {
            Ok(CapturedContent {
                id: row.get(0)?,
                content_type: ContentType::from_str(&row.get::<_, String>(1)?),
                raw_text: row.get(2)?,
                image_path: row.get(3)?,
                thumbnail_path: row.get(4)?,
                source_app: row.get(5)?,
                source_bundle_id: row.get(6)?,
                source_url: row.get(7)?,
                user_note: row.get(8)?,
                captured_at: row.get(9)?,
                content_hash: row.get(10)?,
                byte_size: row.get(11)?,
                is_deleted: row.get::<_, i32>(12)? != 0,
                created_at: row.get(13)?,
                updated_at: row.get(14)?,
                digested_at: row.get(15).unwrap_or(None),
                digest_action: row.get(16).unwrap_or(None),
                summary: row.get(17).unwrap_or(None),
                tags: row.get(18).unwrap_or(None),
                digest: row.get(19).unwrap_or(None),
                wiki_compile_hash: row.get(20).unwrap_or(None),
                wiki_assessed_hash: row.get(21).unwrap_or(None),
                clean_content: row.get(22).unwrap_or(None),
                category: row.get(23).unwrap_or(None),
            })
        })?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    pub fn query_content(
        &self,
        filter: &str,
        start_at: Option<&str>,
        hide_sensitive: bool,
        limit: i64,
        offset: i64,
    ) -> Result<
        (
            Vec<CapturedContent>,
            i64,
            std::collections::HashMap<String, i64>,
        ),
        Box<dyn std::error::Error>,
    > {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        let mut base = vec!["is_deleted = 0".to_string()];
        let mut base_params: Vec<Value> = Vec::new();
        if let Some(start) = start_at {
            base.push("captured_at >= ?".to_string());
            base_params.push(start.to_string().into());
        }
        if hide_sensitive {
            base.push("contains_sensitive(COALESCE(raw_text, '')) = 0".to_string());
        }
        let base_where = base.join(" AND ");
        let filter_clause = match filter {
            "document" => "source_app IN ('Markdown 导入', '导入内容')",
            "text" | "image" | "url" => {
                "content_type = ? AND source_app NOT IN ('Markdown 导入', '导入内容')"
            }
            _ => "1 = 1",
        };
        let mut query_params = base_params.clone();
        if matches!(filter, "text" | "image" | "url") {
            query_params.push(filter.to_string().into());
        }
        let total_sql = format!(
            "SELECT COUNT(*) FROM captured_content WHERE {} AND {}",
            base_where, filter_clause
        );
        let total: i64 =
            conn.query_row(&total_sql, params_from_iter(query_params.clone()), |row| {
                row.get(0)
            })?;

        let counts_sql = format!(
            "SELECT COUNT(*),
                COALESCE(SUM(CASE WHEN content_type = 'text' AND source_app NOT IN ('Markdown 导入', '导入内容') THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN content_type = 'image' AND source_app NOT IN ('Markdown 导入', '导入内容') THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN content_type = 'url' AND source_app NOT IN ('Markdown 导入', '导入内容') THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN source_app IN ('Markdown 导入', '导入内容') THEN 1 ELSE 0 END), 0)
             FROM captured_content WHERE {}",
            base_where
        );
        let values: (i64, i64, i64, i64, i64) =
            conn.query_row(&counts_sql, params_from_iter(base_params.clone()), |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            })?;
        let counts = std::collections::HashMap::from([
            ("all".to_string(), values.0),
            ("text".to_string(), values.1),
            ("image".to_string(), values.2),
            ("url".to_string(), values.3),
            ("document".to_string(), values.4),
        ]);

        query_params.push(limit.into());
        query_params.push(offset.into());
        let sql = format!(
            "SELECT id, content_type, raw_text, image_path, thumbnail_path, source_app, source_bundle_id, source_url, user_note, captured_at, content_hash, byte_size, is_deleted, created_at, updated_at, digested_at, digest_action, summary, tags, digest, wiki_compile_hash, wiki_assessed_hash, clean_content, category
             FROM captured_content WHERE {} AND {} ORDER BY captured_at DESC, id DESC LIMIT ? OFFSET ?",
            base_where, filter_clause
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(query_params), |row| {
            Ok(CapturedContent {
                id: row.get(0)?,
                content_type: ContentType::from_str(&row.get::<_, String>(1)?),
                raw_text: row.get(2)?,
                image_path: row.get(3)?,
                thumbnail_path: row.get(4)?,
                source_app: row.get(5)?,
                source_bundle_id: row.get(6)?,
                source_url: row.get(7)?,
                user_note: row.get(8)?,
                captured_at: row.get(9)?,
                content_hash: row.get(10)?,
                byte_size: row.get(11)?,
                is_deleted: row.get::<_, i32>(12)? != 0,
                created_at: row.get(13)?,
                updated_at: row.get(14)?,
                digested_at: row.get(15).unwrap_or(None),
                digest_action: row.get(16).unwrap_or(None),
                summary: row.get(17).unwrap_or(None),
                tags: row.get(18).unwrap_or(None),
                digest: row.get(19).unwrap_or(None),
                wiki_compile_hash: row.get(20).unwrap_or(None),
                wiki_assessed_hash: row.get(21).unwrap_or(None),
                clean_content: row.get(22).unwrap_or(None),
                category: row.get(23).unwrap_or(None),
            })
        })?;
        Ok((rows.collect::<Result<Vec<_>, _>>()?, total, counts))
    }

    pub fn content_position(
        &self,
        id: &str,
        hide_sensitive: bool,
    ) -> Result<Option<i64>, Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        let target: Option<(String, String)> = conn.query_row(
            "SELECT captured_at, id FROM captured_content WHERE id = ?1 AND is_deleted = 0 AND (?2 = 0 OR contains_sensitive(COALESCE(raw_text, '')) = 0)",
            params![id, hide_sensitive],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).optional()?;
        let Some((captured_at, target_id)) = target else {
            return Ok(None);
        };
        let position = conn.query_row(
            "SELECT COUNT(*) FROM captured_content WHERE is_deleted = 0 AND (?1 = 0 OR contains_sensitive(COALESCE(raw_text, '')) = 0) AND (captured_at > ?2 OR (captured_at = ?2 AND id > ?3))",
            params![hide_sensitive, captured_at, target_id],
            |row| row.get(0),
        )?;
        Ok(Some(position))
    }

    /// Search content by keyword across raw_text, source_url, source_app, and user_note.
    pub fn search_content(
        &self,
        query: &str,
        limit: i64,
        hide_sensitive: bool,
    ) -> Result<Vec<CapturedContent>, Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        let pattern = format!("%{}%", query);
        let mut stmt = conn.prepare(
            "SELECT id, content_type, raw_text, image_path, thumbnail_path, source_app, source_bundle_id, source_url, user_note, captured_at, content_hash, byte_size, is_deleted, created_at, updated_at, digested_at, digest_action, summary, tags, digest, wiki_compile_hash, wiki_assessed_hash, clean_content, category
             FROM captured_content
             WHERE is_deleted = 0
               AND (?3 = 0 OR contains_sensitive(COALESCE(raw_text, '')) = 0)
               AND (raw_text LIKE ?1 OR source_url LIKE ?1 OR source_app LIKE ?1 OR user_note LIKE ?1)
             ORDER BY captured_at DESC, id DESC LIMIT ?2"
        )?;

        let rows = stmt.query_map(params![pattern, limit, hide_sensitive], |row| {
            Ok(CapturedContent {
                id: row.get(0)?,
                content_type: ContentType::from_str(&row.get::<_, String>(1)?),
                raw_text: row.get(2)?,
                image_path: row.get(3)?,
                thumbnail_path: row.get(4)?,
                source_app: row.get(5)?,
                source_bundle_id: row.get(6)?,
                source_url: row.get(7)?,
                user_note: row.get(8)?,
                captured_at: row.get(9)?,
                content_hash: row.get(10)?,
                byte_size: row.get(11)?,
                is_deleted: row.get::<_, i32>(12)? != 0,
                created_at: row.get(13)?,
                updated_at: row.get(14)?,
                digested_at: row.get(15).unwrap_or(None),
                digest_action: row.get(16).unwrap_or(None),
                summary: row.get(17).unwrap_or(None),
                tags: row.get(18).unwrap_or(None),
                digest: row.get(19).unwrap_or(None),
                wiki_compile_hash: row.get(20).unwrap_or(None),
                wiki_assessed_hash: row.get(21).unwrap_or(None),
                clean_content: row.get(22).unwrap_or(None),
                category: row.get(23).unwrap_or(None),
            })
        })?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    /// Update the raw_text and source_url of an existing content item.
    /// Used by the URL reader to fill in fetched article content.
    pub fn update_content_for_url(
        &self,
        id: &str,
        raw_text: &str,
        source_url: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE captured_content SET raw_text = ?1, source_url = ?2, byte_size = ?3, updated_at = datetime('now') WHERE id = ?4",
            params![raw_text, source_url, raw_text.len() as i64, id],
        )?;
        Ok(())
    }

    /// Move a content item to the top by updating its captured_at to now.
    pub fn touch_captured_at(&self, id: &str) -> Result<(), Box<dyn std::error::Error>> {
        let now = chrono::Utc::now().to_rfc3339();
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE captured_content SET captured_at = ?1, updated_at = datetime('now') WHERE id = ?2",
            rusqlite::params![now, id],
        )?;
        Ok(())
    }

    /// Text captures from one app since `since` (RFC 3339), newest first, as (id, text).
    pub fn recent_texts_from_source(
        &self,
        source_app: &str,
        since: &str,
    ) -> Result<Vec<(String, String)>, Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, raw_text FROM captured_content
             WHERE is_deleted = 0 AND content_type = 'text' AND raw_text IS NOT NULL
               AND source_app = ?1 AND captured_at >= ?2
             ORDER BY captured_at DESC LIMIT 50",
        )?;
        let rows = stmt.query_map(params![source_app, since], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        Ok(rows.filter_map(Result::ok).collect())
    }

    /// Update the AI-generated summary, tags, digest, and category for a content item.
    /// An empty category keeps whatever category the item already has.
    pub fn update_summary_and_tags(
        &self,
        id: &str,
        summary: &str,
        tags: &str,
        digest: &str,
        category: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE captured_content SET summary = ?1, tags = ?2, digest = ?3, category = COALESCE(NULLIF(?4, ''), category), updated_at = datetime('now') WHERE id = ?5",
            rusqlite::params![summary, tags, digest, category, id],
        )?;
        Ok(())
    }

    /// List distinct categories in use, most frequent first.
    /// Fed into the summary prompt so the AI reuses existing categories
    /// instead of inventing near-duplicates.
    pub fn get_distinct_categories(&self) -> Result<Vec<String>, Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT category FROM captured_content \
             WHERE is_deleted = 0 AND category IS NOT NULL AND category != '' \
             GROUP BY category ORDER BY COUNT(*) DESC LIMIT 50",
        )?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    /// Update the AI-cleaned content and optionally clear wiki hash to trigger recompilation.
    pub fn update_clean_content(
        &self,
        id: &str,
        clean_content: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE captured_content SET clean_content = ?1, wiki_assessed_hash = NULL, updated_at = datetime('now') WHERE id = ?2",
            rusqlite::params![clean_content, id],
        )?;
        Ok(())
    }

    /// Update the raw_text of an existing content item (e.g. OCR result for images).
    pub fn update_raw_text(
        &self,
        id: &str,
        raw_text: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE captured_content SET raw_text = ?1, byte_size = ?2, updated_at = datetime('now') WHERE id = ?3",
            params![raw_text, raw_text.len() as i64, id],
        )?;
        Ok(())
    }

    pub fn delete_content(&self, id: &str) -> Result<(), Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE captured_content SET is_deleted = 1, updated_at = datetime('now') WHERE id = ?1",
            params![id],
        )?;
        Ok(())
    }

    /// Update the user_note for an existing content item.
    pub fn update_user_note(&self, id: &str, note: &str) -> Result<(), Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE captured_content SET user_note = ?1, updated_at = datetime('now') WHERE id = ?2",
            params![note, id],
        )?;
        Ok(())
    }

    /// Find an existing content item by its content_hash (for dedup in spotlight).
    pub fn find_content_by_hash(
        &self,
        hash: &str,
    ) -> Result<Option<CapturedContent>, Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, content_type, raw_text, image_path, thumbnail_path, source_app, source_bundle_id, source_url, user_note, captured_at, content_hash, byte_size, is_deleted, created_at, updated_at, digested_at, digest_action, summary, tags, digest, wiki_compile_hash, wiki_assessed_hash, clean_content, category
             FROM captured_content WHERE content_hash = ?1 AND is_deleted = 0 LIMIT 1"
        )?;

        let mut rows = stmt.query_map(params![hash], |row| {
            Ok(CapturedContent {
                id: row.get(0)?,
                content_type: ContentType::from_str(&row.get::<_, String>(1)?),
                raw_text: row.get(2)?,
                image_path: row.get(3)?,
                thumbnail_path: row.get(4)?,
                source_app: row.get(5)?,
                source_bundle_id: row.get(6)?,
                source_url: row.get(7)?,
                user_note: row.get(8)?,
                captured_at: row.get(9)?,
                content_hash: row.get(10)?,
                byte_size: row.get(11)?,
                is_deleted: row.get::<_, i32>(12)? != 0,
                created_at: row.get(13)?,
                updated_at: row.get(14)?,
                digested_at: row.get(15).unwrap_or(None),
                digest_action: row.get(16).unwrap_or(None),
                summary: row.get(17).unwrap_or(None),
                tags: row.get(18).unwrap_or(None),
                digest: row.get(19).unwrap_or(None),
                wiki_compile_hash: row.get(20).unwrap_or(None),
                wiki_assessed_hash: row.get(21).unwrap_or(None),
                clean_content: row.get(22).unwrap_or(None),
                category: row.get(23).unwrap_or(None),
            })
        })?;

        match rows.next() {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }

    pub fn content_exists_by_hash(&self, hash: &str) -> Result<bool, Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        let count: i32 = conn.query_row(
            "SELECT COUNT(*) FROM captured_content WHERE content_hash = ?1 AND is_deleted = 0",
            params![hash],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// Get all content captured between week_start and week_end (inclusive).
    /// Dates should be in ISO 8601 / RFC 3339 format (e.g. "2025-01-06T00:00:00+00:00").
    pub fn get_content_for_week(
        &self,
        week_start: &str,
        week_end: &str,
    ) -> Result<Vec<CapturedContent>, Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, content_type, raw_text, image_path, thumbnail_path, source_app, source_bundle_id, source_url, user_note, captured_at, content_hash, byte_size, is_deleted, created_at, updated_at, digested_at, digest_action, summary, tags, digest, wiki_compile_hash, wiki_assessed_hash, clean_content, category
             FROM captured_content
             WHERE is_deleted = 0 AND captured_at >= ?1 AND captured_at <= ?2
             ORDER BY captured_at DESC"
        )?;

        let rows = stmt.query_map(params![week_start, week_end], |row| {
            Ok(CapturedContent {
                id: row.get(0)?,
                content_type: ContentType::from_str(&row.get::<_, String>(1)?),
                raw_text: row.get(2)?,
                image_path: row.get(3)?,
                thumbnail_path: row.get(4)?,
                source_app: row.get(5)?,
                source_bundle_id: row.get(6)?,
                source_url: row.get(7)?,
                user_note: row.get(8)?,
                captured_at: row.get(9)?,
                content_hash: row.get(10)?,
                byte_size: row.get(11)?,
                is_deleted: row.get::<_, i32>(12)? != 0,
                created_at: row.get(13)?,
                updated_at: row.get(14)?,
                digested_at: row.get(15).unwrap_or(None),
                digest_action: row.get(16).unwrap_or(None),
                summary: row.get(17).unwrap_or(None),
                tags: row.get(18).unwrap_or(None),
                digest: row.get(19).unwrap_or(None),
                wiki_compile_hash: row.get(20).unwrap_or(None),
                wiki_assessed_hash: row.get(21).unwrap_or(None),
                clean_content: row.get(22).unwrap_or(None),
                category: row.get(23).unwrap_or(None),
            })
        })?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    /// Get a single content item by its ID.
    pub fn get_content_by_id(
        &self,
        id: &str,
    ) -> Result<Option<CapturedContent>, Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, content_type, raw_text, image_path, thumbnail_path, source_app, source_bundle_id, source_url, user_note, captured_at, content_hash, byte_size, is_deleted, created_at, updated_at, digested_at, digest_action, summary, tags, digest, wiki_compile_hash, wiki_assessed_hash, clean_content, category
             FROM captured_content WHERE id = ?1 AND is_deleted = 0"
        )?;

        let mut rows = stmt.query_map(params![id], |row| {
            Ok(CapturedContent {
                id: row.get(0)?,
                content_type: ContentType::from_str(&row.get::<_, String>(1)?),
                raw_text: row.get(2)?,
                image_path: row.get(3)?,
                thumbnail_path: row.get(4)?,
                source_app: row.get(5)?,
                source_bundle_id: row.get(6)?,
                source_url: row.get(7)?,
                user_note: row.get(8)?,
                captured_at: row.get(9)?,
                content_hash: row.get(10)?,
                byte_size: row.get(11)?,
                is_deleted: row.get::<_, i32>(12)? != 0,
                created_at: row.get(13)?,
                updated_at: row.get(14)?,
                digested_at: row.get(15).unwrap_or(None),
                digest_action: row.get(16).unwrap_or(None),
                summary: row.get(17).unwrap_or(None),
                tags: row.get(18).unwrap_or(None),
                digest: row.get(19).unwrap_or(None),
                wiki_compile_hash: row.get(20).unwrap_or(None),
                wiki_assessed_hash: row.get(21).unwrap_or(None),
                clean_content: row.get(22).unwrap_or(None),
                category: row.get(23).unwrap_or(None),
            })
        })?;

        match rows.next() {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }

    // ========== Weekly Reports ==========

    /// Save a complete weekly report with its sections to the database.
    pub fn save_report(&self, report: &WeeklyReport) -> Result<(), Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;

        // Insert the report
        conn.execute(
            "INSERT OR REPLACE INTO weekly_reports (id, week_start, week_end, summary_text, report_json, content_count, model_used, tokens_used, generated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                report.id,
                report.week_start,
                report.week_end,
                report.summary_text,
                report.report_json.to_string(),
                report.content_count,
                report.model_used,
                report.tokens_used,
                report.generated_at,
            ],
        )?;

        // Delete old sections for this report (in case of regeneration)
        conn.execute(
            "DELETE FROM report_sections WHERE report_id = ?1",
            params![report.id],
        )?;

        // Insert sections
        for section in &report.sections {
            let content_ids_json =
                serde_json::to_string(&section.content_ids).unwrap_or_else(|_| "[]".to_string());

            conn.execute(
                "INSERT INTO report_sections (id, report_id, section_type, title, body, relevance_score, sort_order, content_ids)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    section.id,
                    section.report_id,
                    section.section_type,
                    section.title,
                    section.body,
                    section.relevance_score,
                    section.sort_order,
                    content_ids_json,
                ],
            )?;
        }

        Ok(())
    }

    /// Get a weekly report for a specific week_start date.
    pub fn get_report_by_week(
        &self,
        week_start: &str,
    ) -> Result<Option<WeeklyReport>, Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;

        let mut stmt = conn.prepare(
            "SELECT id, week_start, week_end, summary_text, report_json, content_count, model_used, tokens_used, generated_at
             FROM weekly_reports WHERE week_start = ?1"
        )?;

        let mut rows = stmt.query_map(params![week_start], |row| {
            let report_json_str: String = row.get(4)?;
            let report_json: serde_json::Value =
                serde_json::from_str(&report_json_str).unwrap_or(serde_json::Value::Null);

            Ok(WeeklyReport {
                id: row.get(0)?,
                week_start: row.get(1)?,
                week_end: row.get(2)?,
                summary_text: row.get(3)?,
                report_json,
                content_count: row.get(5)?,
                model_used: row.get(6)?,
                tokens_used: row.get(7)?,
                generated_at: row.get(8)?,
                sections: Vec::new(), // filled below
            })
        })?;

        let report = match rows.next() {
            Some(row) => row?,
            None => return Ok(None),
        };

        // Load sections for this report
        let sections = self.get_sections_for_report_inner(&conn, &report.id)?;

        Ok(Some(WeeklyReport { sections, ..report }))
    }

    /// List all weekly reports (without full sections, just metadata).
    pub fn get_all_reports(&self) -> Result<Vec<WeeklyReport>, Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;

        let mut stmt = conn.prepare(
            "SELECT id, week_start, week_end, summary_text, report_json, content_count, model_used, tokens_used, generated_at
             FROM weekly_reports ORDER BY week_start DESC"
        )?;

        let rows = stmt.query_map([], |row| {
            let report_json_str: String = row.get(4)?;
            let report_json: serde_json::Value =
                serde_json::from_str(&report_json_str).unwrap_or(serde_json::Value::Null);

            Ok(WeeklyReport {
                id: row.get(0)?,
                week_start: row.get(1)?,
                week_end: row.get(2)?,
                summary_text: row.get(3)?,
                report_json,
                content_count: row.get(5)?,
                model_used: row.get(6)?,
                tokens_used: row.get(7)?,
                generated_at: row.get(8)?,
                sections: Vec::new(),
            })
        })?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    /// Internal helper: load sections for a report using an already-locked connection.
    fn get_sections_for_report_inner(
        &self,
        conn: &rusqlite::Connection,
        report_id: &str,
    ) -> Result<Vec<ReportSection>, Box<dyn std::error::Error>> {
        let mut stmt = conn.prepare(
            "SELECT id, report_id, section_type, title, body, relevance_score, sort_order, content_ids
             FROM report_sections WHERE report_id = ?1 ORDER BY sort_order"
        )?;

        let rows = stmt.query_map(params![report_id], |row| {
            let content_ids_str: Option<String> = row.get(7)?;
            let content_ids: Vec<String> = content_ids_str
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default();

            Ok(ReportSection {
                id: row.get(0)?,
                report_id: row.get(1)?,
                section_type: row.get(2)?,
                title: row.get(3)?,
                body: row.get(4)?,
                relevance_score: row.get(5)?,
                sort_order: row.get(6)?,
                content_ids,
            })
        })?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    // ========== User Feedback ==========

    /// Save user feedback (interested/dismissed/bookmarked) for a content or section.
    pub fn save_feedback(&self, feedback: &UserFeedback) -> Result<(), Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;

        conn.execute(
            "INSERT INTO user_feedback (id, content_id, section_id, feedback_type, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                feedback.id,
                feedback.content_id,
                feedback.section_id,
                feedback.feedback_type.as_str(),
                feedback.created_at,
            ],
        )?;

        Ok(())
    }

    // ========== User Preferences ==========

    /// Update or insert a topic preference. Increases weight by weight_delta
    /// and increments occurrence_count.
    pub fn update_preference(
        &self,
        topic: &str,
        weight_delta: f64,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;

        // Try to update existing preference
        let rows_updated = conn.execute(
            "UPDATE user_preferences SET weight = weight + ?1, occurrence_count = occurrence_count + 1, last_updated = datetime('now')
             WHERE topic = ?2",
            params![weight_delta, topic],
        )?;

        // If no existing row, insert a new one
        if rows_updated == 0 {
            let id = uuid::Uuid::new_v4().to_string();
            conn.execute(
                "INSERT INTO user_preferences (id, topic, weight, occurrence_count, last_updated)
                 VALUES (?1, ?2, ?3, 1, datetime('now'))",
                params![id, topic, weight_delta],
            )?;
        }

        Ok(())
    }

    /// Get all user preferences ordered by weight descending.
    pub fn get_all_preferences(&self) -> Result<Vec<UserPreference>, Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;

        let mut stmt = conn.prepare(
            "SELECT id, topic, weight, occurrence_count, last_updated
             FROM user_preferences ORDER BY weight DESC",
        )?;

        let rows = stmt.query_map([], |row| {
            Ok(UserPreference {
                id: row.get(0)?,
                topic: row.get(1)?,
                weight: row.get(2)?,
                occurrence_count: row.get(3)?,
                last_updated: row.get(4)?,
            })
        })?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    // ========== Data Hub ==========

    /// Get all dates that have captured content, with counts.
    pub fn get_dates_with_content(&self) -> Result<Vec<(String, i64)>, Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT DATE(captured_at) as day, COUNT(*) as cnt FROM captured_content
             WHERE is_deleted = 0 GROUP BY day ORDER BY day DESC",
        )?;

        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    /// Get all content for a specific date.
    pub fn get_content_for_date(
        &self,
        date: &str,
    ) -> Result<Vec<CapturedContent>, Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, content_type, raw_text, image_path, thumbnail_path, source_app, source_bundle_id, source_url, user_note, captured_at, content_hash, byte_size, is_deleted, created_at, updated_at, digested_at, digest_action, summary, tags, digest, wiki_compile_hash, wiki_assessed_hash, clean_content, category
             FROM captured_content WHERE DATE(captured_at) = ?1 AND is_deleted = 0 ORDER BY captured_at ASC",
        )?;

        let rows = stmt.query_map(params![date], |row| {
            Ok(CapturedContent {
                id: row.get(0)?,
                content_type: ContentType::from_str(&row.get::<_, String>(1)?),
                raw_text: row.get(2)?,
                image_path: row.get(3)?,
                thumbnail_path: row.get(4)?,
                source_app: row.get(5)?,
                source_bundle_id: row.get(6)?,
                source_url: row.get(7)?,
                user_note: row.get(8)?,
                captured_at: row.get(9)?,
                content_hash: row.get(10)?,
                byte_size: row.get(11)?,
                is_deleted: row.get::<_, i32>(12)? != 0,
                created_at: row.get(13)?,
                updated_at: row.get(14)?,
                digested_at: row.get(15).unwrap_or(None),
                digest_action: row.get(16).unwrap_or(None),
                summary: row.get(17).unwrap_or(None),
                tags: row.get(18).unwrap_or(None),
                digest: row.get(19).unwrap_or(None),
                wiki_compile_hash: row.get(20).unwrap_or(None),
                wiki_assessed_hash: row.get(21).unwrap_or(None),
                clean_content: row.get(22).unwrap_or(None),
                category: row.get(23).unwrap_or(None),
            })
        })?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    // ========== Digest ==========

    /// Get undigested content items, ordered by oldest first.
    /// Used by the "消化" feature to surface content for review.
    pub fn get_undigested_content(
        &self,
        limit: i64,
    ) -> Result<Vec<CapturedContent>, Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, content_type, raw_text, image_path, thumbnail_path, source_app, source_bundle_id, source_url, user_note, captured_at, content_hash, byte_size, is_deleted, created_at, updated_at, digested_at, digest_action, summary, tags, digest, wiki_compile_hash, wiki_assessed_hash, clean_content, category
             FROM captured_content
             WHERE is_deleted = 0 AND digested_at IS NULL
             ORDER BY captured_at ASC LIMIT ?1"
        )?;

        let rows = stmt.query_map(params![limit], |row| {
            Ok(CapturedContent {
                id: row.get(0)?,
                content_type: ContentType::from_str(&row.get::<_, String>(1)?),
                raw_text: row.get(2)?,
                image_path: row.get(3)?,
                thumbnail_path: row.get(4)?,
                source_app: row.get(5)?,
                source_bundle_id: row.get(6)?,
                source_url: row.get(7)?,
                user_note: row.get(8)?,
                captured_at: row.get(9)?,
                content_hash: row.get(10)?,
                byte_size: row.get(11)?,
                is_deleted: row.get::<_, i32>(12)? != 0,
                created_at: row.get(13)?,
                updated_at: row.get(14)?,
                digested_at: row.get(15).unwrap_or(None),
                digest_action: row.get(16).unwrap_or(None),
                summary: row.get(17).unwrap_or(None),
                tags: row.get(18).unwrap_or(None),
                digest: row.get(19).unwrap_or(None),
                wiki_compile_hash: row.get(20).unwrap_or(None),
                wiki_assessed_hash: row.get(21).unwrap_or(None),
                clean_content: row.get(22).unwrap_or(None),
                category: row.get(23).unwrap_or(None),
            })
        })?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    /// Get undigested content from the last N days, ordered oldest first.
    pub fn get_undigested_content_recent(
        &self,
        days: i64,
    ) -> Result<Vec<CapturedContent>, Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, content_type, raw_text, image_path, thumbnail_path, source_app, source_bundle_id, source_url, user_note, captured_at, content_hash, byte_size, is_deleted, created_at, updated_at, digested_at, digest_action, summary, tags, digest, wiki_compile_hash, wiki_assessed_hash, clean_content, category
             FROM captured_content
             WHERE is_deleted = 0 AND digested_at IS NULL
               AND captured_at >= datetime('now', '-' || ?1 || ' days')
             ORDER BY captured_at ASC"
        )?;

        let rows = stmt.query_map(params![days], |row| {
            Ok(CapturedContent {
                id: row.get(0)?,
                content_type: ContentType::from_str(&row.get::<_, String>(1)?),
                raw_text: row.get(2)?,
                image_path: row.get(3)?,
                thumbnail_path: row.get(4)?,
                source_app: row.get(5)?,
                source_bundle_id: row.get(6)?,
                source_url: row.get(7)?,
                user_note: row.get(8)?,
                captured_at: row.get(9)?,
                content_hash: row.get(10)?,
                byte_size: row.get(11)?,
                is_deleted: row.get::<_, i32>(12)? != 0,
                created_at: row.get(13)?,
                updated_at: row.get(14)?,
                digested_at: row.get(15).unwrap_or(None),
                digest_action: row.get(16).unwrap_or(None),
                summary: row.get(17).unwrap_or(None),
                tags: row.get(18).unwrap_or(None),
                digest: row.get(19).unwrap_or(None),
                wiki_compile_hash: row.get(20).unwrap_or(None),
                wiki_assessed_hash: row.get(21).unwrap_or(None),
                clean_content: row.get(22).unwrap_or(None),
                category: row.get(23).unwrap_or(None),
            })
        })?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    /// Mark a content item as digested with the given action (keep/archive/pin).
    pub fn update_digest_action(
        &self,
        id: &str,
        action: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        let rows = conn.execute(
            "UPDATE captured_content SET digested_at = datetime('now'), digest_action = ?1, updated_at = datetime('now') WHERE id = ?2 AND is_deleted = 0",
            params![action, id],
        )?;
        if rows == 0 {
            return Err(format!("Content not found: {}", id).into());
        }
        Ok(())
    }

    /// Count total undigested content items.
    pub fn count_undigested(&self) -> Result<i64, Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM captured_content WHERE is_deleted = 0 AND digested_at IS NULL",
            [],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    /// Total number of non-deleted captured items. Used by the insight
    /// scheduler to decide when a first auto-report should be generated.
    pub fn count_content(&self) -> Result<i64, Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM captured_content WHERE is_deleted = 0",
            [],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    // ========== App Settings ==========

    fn get_setting_from_db(&self, key: &str) -> Result<Option<String>, Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;

        let mut stmt = conn.prepare("SELECT value FROM app_settings WHERE key = ?1")?;
        let mut rows = stmt.query_map(params![key], |row| row.get::<_, String>(0))?;

        match rows.next() {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }

    fn update_setting_db(&self, key: &str, value: &str) -> Result<(), Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;

        conn.execute(
            "INSERT INTO app_settings (key, value, updated_at) VALUES (?1, ?2, datetime('now'))
             ON CONFLICT(key) DO UPDATE SET value = ?2, updated_at = datetime('now')",
            params![key, value],
        )?;
        Ok(())
    }

    /// Get a setting value by key. Sensitive values are encrypted in SQLite,
    /// with a one-time migration from older plaintext values.
    pub fn get_setting(&self, key: &str) -> Result<Option<String>, Box<dyn std::error::Error>> {
        if secure_store::is_secret_setting(key) {
            let db_value = self.get_setting_from_db(key)?;
            let Some(value) = db_value else {
                return Ok(None);
            };

            if value.is_empty() || secure_store::is_secret_placeholder(&value) {
                return Ok(None);
            }

            if secure_store::is_encrypted_value(&value) {
                return secure_store::decrypt_secret(&value)
                    .map(Some)
                    .map_err(|e| e.into());
            }

            match secure_store::encrypt_secret(&value) {
                Ok(encrypted) => {
                    if let Err(e) = self.update_setting_db(key, &encrypted) {
                        log::warn!(
                            "Encrypted secret setting '{}' but failed to persist migration: {}",
                            key,
                            e
                        );
                    }
                }
                Err(e) => log::warn!(
                    "Failed to encrypt legacy secret setting '{}'; using plaintext value for this read: {}",
                    key,
                    e
                ),
            }
            return Ok(Some(value));
        }

        self.get_setting_from_db(key)
    }

    /// Get all settings as key-value pairs.
    pub fn get_all_settings(&self) -> Result<HashMap<String, String>, Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;

        let mut stmt = conn.prepare("SELECT key, value FROM app_settings")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;

        let mut settings = HashMap::new();
        for row in rows {
            let (key, value) = row?;
            if secure_store::is_secret_setting(&key) {
                settings.insert(key, secure_store::mask_secret_value(&value));
            } else {
                settings.insert(key, value);
            }
        }

        Ok(settings)
    }

    /// Update a setting value by key. Sensitive values are encrypted before
    /// writing to SQLite, so loading settings never triggers an OS prompt.
    pub fn update_setting(&self, key: &str, value: &str) -> Result<(), Box<dyn std::error::Error>> {
        if secure_store::is_secret_setting(key) {
            if secure_store::is_secret_placeholder(value) {
                return Ok(());
            }

            if value.is_empty() {
                return self.update_setting_db(key, "");
            }

            let encrypted = secure_store::encrypt_secret(value)?;
            return self.update_setting_db(key, &encrypted);
        }

        self.update_setting_db(key, value)
    }

    // ========== Attention Insights ==========

    /// Get recent content for attention analysis (rich fields for v2).
    pub fn get_recent_content_for_analysis(
        &self,
        days: i64,
        limit: usize,
    ) -> Result<Vec<ContentForAnalysis>, Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        let cutoff = (chrono::Utc::now() - chrono::TimeDelta::days(days)).to_rfc3339();
        let mut stmt = conn.prepare(
            "SELECT id, raw_text, source_url, captured_at, summary, tags, user_note, source_app, content_type
             FROM captured_content
             WHERE is_deleted = 0 AND captured_at >= ?1
             ORDER BY captured_at DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![cutoff, limit as i64], |row| {
            Ok(ContentForAnalysis {
                id: row.get(0)?,
                raw_text: row.get(1)?,
                source_url: row.get(2)?,
                captured_at: row.get(3)?,
                summary: row.get(4)?,
                tags: row.get(5)?,
                user_note: row.get(6)?,
                source_app: row.get(7)?,
                content_type: row.get(8)?,
            })
        })?;
        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    /// Compute stats for radar v2 prompt from content items.
    pub fn get_content_stats(items: &[ContentForAnalysis]) -> serde_json::Value {
        use std::collections::HashMap;

        let total = items.len();
        if total == 0 {
            return serde_json::json!({});
        }

        // Source distribution
        let mut source_map: HashMap<&str, usize> = HashMap::new();
        for item in items {
            *source_map.entry(item.source_app.as_str()).or_default() += 1;
        }
        let source_count = source_map.len();
        let sources: Vec<serde_json::Value> = {
            let mut v: Vec<_> = source_map.iter().collect();
            v.sort_by(|a, b| b.1.cmp(a.1));
            v.iter()
                .map(|(name, count)| serde_json::json!({"name": name, "count": count}))
                .collect()
        };

        // Content type distribution
        let mut content_type_map: HashMap<&str, usize> = HashMap::new();
        for item in items {
            *content_type_map
                .entry(item.content_type.as_str())
                .or_default() += 1;
        }
        let content_types: Vec<serde_json::Value> = {
            let mut v: Vec<_> = content_type_map.iter().collect();
            v.sort_by(|a, b| b.1.cmp(a.1));
            v.iter()
                .map(|(name, count)| serde_json::json!({"name": name, "count": count}))
                .collect()
        };

        // Hour distribution
        let (mut morning, mut afternoon, mut evening, mut midnight) = (0usize, 0, 0, 0);
        for item in items {
            // Try to parse hour from ISO timestamp
            if let Some(t_pos) = item.captured_at.find('T') {
                if let Ok(hour) = item.captured_at[t_pos + 1..]
                    .get(..2)
                    .unwrap_or("0")
                    .parse::<u32>()
                {
                    match hour {
                        6..=11 => morning += 1,
                        12..=17 => afternoon += 1,
                        18..=23 => evening += 1,
                        _ => midnight += 1,
                    }
                }
            }
        }

        // Active days + peak day
        let mut day_counts: HashMap<String, usize> = HashMap::new();
        for item in items {
            let day = item.captured_at.get(..10).unwrap_or("").to_string();
            if !day.is_empty() {
                *day_counts.entry(day).or_default() += 1;
            }
        }
        let day_keys: Vec<&str> = day_counts.keys().map(|s| s.as_str()).collect();
        let min_day = day_keys.iter().min().copied().unwrap_or("");
        let max_day = day_keys.iter().max().copied().unwrap_or("");
        let active_days = day_counts.len();
        let total_days = if min_day.len() >= 10 && max_day.len() >= 10 {
            let start = chrono::NaiveDate::parse_from_str(&min_day[..10], "%Y-%m-%d").ok();
            let end = chrono::NaiveDate::parse_from_str(&max_day[..10], "%Y-%m-%d").ok();
            match (start, end) {
                (Some(s), Some(e)) => ((e - s).num_days().max(0) + 1) as usize,
                _ => active_days,
            }
        } else {
            active_days
        };

        let peak_day = day_counts
            .iter()
            .max_by_key(|(_, c)| *c)
            .map(|(d, c)| serde_json::json!({"date": d, "count": c}))
            .unwrap_or(serde_json::json!(null));

        let annotated = items
            .iter()
            .filter(|i| {
                i.user_note.as_ref().is_some_and(|n| !n.is_empty())
                    || i.tags.as_ref().is_some_and(|t| !t.is_empty())
            })
            .count();
        let annotation_rate = ((annotated as f64 / total as f64) * 100.0).round();
        let avg_per_active = if active_days > 0 {
            total as f64 / active_days as f64
        } else {
            0.0
        };

        serde_json::json!({
            "date_range": format!("{} 至 {}", min_day, max_day),
            "total_items": total,
            "active_days": active_days,
            "total_days": total_days,
            "annotated_items": annotated,
            "annotation_rate": format!("{}%", annotation_rate as i64),
            "source_count": source_count,
            "sources": sources,
            "content_types": content_types,
            "peak_day": peak_day,
            "avg_per_active_day": (avg_per_active * 10.0).round() / 10.0,
            "hour_distribution": {
                "morning": morning,
                "afternoon": afternoon,
                "evening": evening,
                "midnight": midnight,
            }
        })
    }

    /// Save a new attention insight, marking all previous as not current.
    pub fn save_attention_insight(
        &self,
        analysis_json: Option<&str>,
        status: &str,
        error_message: Option<&str>,
        window_start: &str,
        window_end: &str,
        content_count: i32,
        model_used: &str,
    ) -> Result<i64, Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        conn.execute("UPDATE attention_insights SET is_current = 0", [])?;
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO attention_insights (analysis_json, status, error_message, analyzed_at, window_start, window_end, content_count, model_used, is_current)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 1)",
            params![analysis_json, status, error_message, now, window_start, window_end, content_count, model_used],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Update the status of an insight.
    pub fn update_insight_status(
        &self,
        id: i64,
        status: &str,
        analysis_json: Option<&str>,
        error_message: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE attention_insights SET status = ?1, analysis_json = ?2, error_message = ?3 WHERE id = ?4",
            params![status, analysis_json, error_message, id],
        )?;
        Ok(())
    }

    /// Get the most recent current insight.
    pub fn get_current_insight(
        &self,
    ) -> Result<Option<AttentionInsight>, Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, analysis_json, status, error_message, analyzed_at, window_start, window_end, content_count, model_used, is_current
             FROM attention_insights
             WHERE is_current = 1
             ORDER BY analyzed_at DESC
             LIMIT 1",
        )?;
        let mut rows = stmt.query_map([], |row| {
            Ok(AttentionInsight {
                id: row.get(0)?,
                analysis_json: row.get(1)?,
                status: row.get(2)?,
                error_message: row.get(3)?,
                analyzed_at: row.get(4)?,
                window_start: row.get(5)?,
                window_end: row.get(6)?,
                content_count: row.get(7)?,
                model_used: row.get(8)?,
                is_current: row.get::<_, i32>(9)? == 1,
            })
        })?;
        match rows.next() {
            Some(Ok(insight)) => Ok(Some(insight)),
            Some(Err(e)) => Err(Box::new(e)),
            None => Ok(None),
        }
    }

    /// Check if any content was saved or updated after the given timestamp.
    pub fn has_new_content_since(&self, since: &str) -> Result<bool, Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM captured_content WHERE is_deleted = 0 AND (captured_at > ?1 OR updated_at > ?1)",
            params![since],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    // ========== Wiki Pages ==========

    pub fn save_wiki_page(
        &self,
        page: &super::models::WikiPage,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        conn.execute(
            "INSERT INTO wiki_pages (id, title, slug, page_type, body_markdown, summary, tags, status, confidence, created_at, updated_at, last_compiled_at, source_message_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                page.id, page.title, page.slug, page.page_type, page.body_markdown,
                page.summary, page.tags, page.status, page.confidence,
                page.created_at, page.updated_at, page.last_compiled_at, page.source_message_id,
            ],
        )?;
        Ok(())
    }

    pub fn update_wiki_page(
        &self,
        page: &super::models::WikiPage,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE wiki_pages SET title=?1, body_markdown=?2, summary=?3, tags=?4, status=?5, confidence=?6, updated_at=datetime('now'), last_compiled_at=?7 WHERE id=?8",
            params![page.title, page.body_markdown, page.summary, page.tags, page.status, page.confidence, page.last_compiled_at, page.id],
        )?;
        Ok(())
    }

    pub fn update_wiki_page_status(
        &self,
        page_id: &str,
        status: &str,
        confidence: f64,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE wiki_pages SET status=?1, confidence=?2, updated_at=datetime('now') WHERE id=?3",
            params![status, confidence, page_id],
        )?;
        Ok(())
    }

    pub fn get_wiki_page_by_id(
        &self,
        id: &str,
    ) -> Result<Option<super::models::WikiPage>, Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, title, slug, page_type, body_markdown, summary, tags, status, confidence, created_at, updated_at, last_compiled_at, source_message_id
             FROM wiki_pages WHERE id = ?1"
        )?;
        let mut rows = stmt.query_map(params![id], |row| {
            Ok(super::models::WikiPage {
                id: row.get(0)?,
                title: row.get(1)?,
                slug: row.get(2)?,
                page_type: row.get(3)?,
                body_markdown: row.get(4)?,
                summary: row.get(5)?,
                tags: row.get(6)?,
                status: row.get(7)?,
                confidence: row.get(8)?,
                created_at: row.get(9)?,
                updated_at: row.get(10)?,
                last_compiled_at: row.get(11)?,
                source_message_id: row.get(12).unwrap_or(None),
            })
        })?;
        match rows.next() {
            Some(Ok(page)) => Ok(Some(page)),
            Some(Err(e)) => Err(Box::new(e)),
            None => Ok(None),
        }
    }

    pub fn get_wiki_page_by_slug(
        &self,
        slug: &str,
    ) -> Result<Option<super::models::WikiPage>, Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, title, slug, page_type, body_markdown, summary, tags, status, confidence, created_at, updated_at, last_compiled_at, source_message_id
             FROM wiki_pages WHERE slug = ?1"
        )?;
        let mut rows = stmt.query_map(params![slug], |row| {
            Ok(super::models::WikiPage {
                id: row.get(0)?,
                title: row.get(1)?,
                slug: row.get(2)?,
                page_type: row.get(3)?,
                body_markdown: row.get(4)?,
                summary: row.get(5)?,
                tags: row.get(6)?,
                status: row.get(7)?,
                confidence: row.get(8)?,
                created_at: row.get(9)?,
                updated_at: row.get(10)?,
                last_compiled_at: row.get(11)?,
                source_message_id: row.get(12).unwrap_or(None),
            })
        })?;
        match rows.next() {
            Some(Ok(page)) => Ok(Some(page)),
            Some(Err(e)) => Err(Box::new(e)),
            None => Ok(None),
        }
    }

    pub fn get_all_wiki_pages(
        &self,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<super::models::WikiPage>, Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, title, slug, page_type, body_markdown, summary, tags, status, confidence, created_at, updated_at, last_compiled_at, source_message_id
             FROM wiki_pages WHERE status IN ('active', 'needs_recompile') ORDER BY updated_at DESC LIMIT ?1 OFFSET ?2"
        )?;
        let rows = stmt.query_map(params![limit, offset], |row| {
            Ok(super::models::WikiPage {
                id: row.get(0)?,
                title: row.get(1)?,
                slug: row.get(2)?,
                page_type: row.get(3)?,
                body_markdown: row.get(4)?,
                summary: row.get(5)?,
                tags: row.get(6)?,
                status: row.get(7)?,
                confidence: row.get(8)?,
                created_at: row.get(9)?,
                updated_at: row.get(10)?,
                last_compiled_at: row.get(11)?,
                source_message_id: row.get(12).unwrap_or(None),
            })
        })?;
        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    pub fn search_wiki_pages(
        &self,
        query: &str,
        limit: i64,
    ) -> Result<Vec<super::models::WikiPage>, Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        let pattern = format!("%{}%", query);
        let mut stmt = conn.prepare(
            "SELECT id, title, slug, page_type, body_markdown, summary, tags, status, confidence, created_at, updated_at, last_compiled_at, source_message_id
             FROM wiki_pages WHERE status IN ('active', 'needs_recompile')
             AND page_type != 'qa'
             AND (title LIKE ?1 OR summary LIKE ?1 OR tags LIKE ?1 OR body_markdown LIKE ?1)
             ORDER BY confidence DESC, updated_at DESC LIMIT ?2"
        )?;
        let rows = stmt.query_map(params![pattern, limit], |row| {
            Ok(super::models::WikiPage {
                id: row.get(0)?,
                title: row.get(1)?,
                slug: row.get(2)?,
                page_type: row.get(3)?,
                body_markdown: row.get(4)?,
                summary: row.get(5)?,
                tags: row.get(6)?,
                status: row.get(7)?,
                confidence: row.get(8)?,
                created_at: row.get(9)?,
                updated_at: row.get(10)?,
                last_compiled_at: row.get(11)?,
                source_message_id: row.get(12).unwrap_or(None),
            })
        })?;
        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    pub fn get_wiki_pages_by_type(
        &self,
        page_type: &str,
    ) -> Result<Vec<super::models::WikiPage>, Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, title, slug, page_type, body_markdown, summary, tags, status, confidence, created_at, updated_at, last_compiled_at, source_message_id
             FROM wiki_pages WHERE page_type = ?1 AND status IN ('active', 'needs_recompile') ORDER BY updated_at DESC"
        )?;
        let rows = stmt.query_map(params![page_type], |row| {
            Ok(super::models::WikiPage {
                id: row.get(0)?,
                title: row.get(1)?,
                slug: row.get(2)?,
                page_type: row.get(3)?,
                body_markdown: row.get(4)?,
                summary: row.get(5)?,
                tags: row.get(6)?,
                status: row.get(7)?,
                confidence: row.get(8)?,
                created_at: row.get(9)?,
                updated_at: row.get(10)?,
                last_compiled_at: row.get(11)?,
                source_message_id: row.get(12).unwrap_or(None),
            })
        })?;
        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    pub fn get_wiki_pages_by_status(
        &self,
        status: &str,
    ) -> Result<Vec<super::models::WikiPage>, Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, title, slug, page_type, body_markdown, summary, tags, status, confidence, created_at, updated_at, last_compiled_at, source_message_id
             FROM wiki_pages WHERE status = ?1 ORDER BY updated_at DESC"
        )?;
        let rows = stmt.query_map(params![status], |row| {
            Ok(super::models::WikiPage {
                id: row.get(0)?,
                title: row.get(1)?,
                slug: row.get(2)?,
                page_type: row.get(3)?,
                body_markdown: row.get(4)?,
                summary: row.get(5)?,
                tags: row.get(6)?,
                status: row.get(7)?,
                confidence: row.get(8)?,
                created_at: row.get(9)?,
                updated_at: row.get(10)?,
                last_compiled_at: row.get(11)?,
                source_message_id: row.get(12).unwrap_or(None),
            })
        })?;
        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    pub fn delete_wiki_page(&self, id: &str) -> Result<(), Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        conn.execute("DELETE FROM wiki_pages WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn get_wiki_stats(&self) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        let total_pages: i64 = conn.query_row(
            "SELECT COUNT(*) FROM wiki_pages WHERE status IN ('active', 'needs_recompile')",
            [],
            |r| r.get(0),
        )?;
        let total_edges: i64 =
            conn.query_row("SELECT COUNT(*) FROM wiki_edges", [], |r| r.get(0))?;
        let total_sources: i64 = conn.query_row(
            "SELECT COUNT(DISTINCT content_id) FROM wiki_page_sources WHERE source_status = 'active'", [], |r| r.get(0)
        )?;
        let needs_recompile: i64 = conn.query_row(
            "SELECT COUNT(*) FROM wiki_pages WHERE status = 'needs_recompile'",
            [],
            |r| r.get(0),
        )?;
        let lint_open: i64 = conn.query_row(
            "SELECT COUNT(*) FROM wiki_lint_results WHERE status = 'open'",
            [],
            |r| r.get(0),
        )?;
        Ok(serde_json::json!({
            "total_pages": total_pages,
            "total_edges": total_edges,
            "total_sources": total_sources,
            "needs_recompile": needs_recompile,
            "lint_open": lint_open,
        }))
    }

    /// Returns (id, title, summary) for all active pages — used as compile context.
    pub fn get_wiki_page_summaries(
        &self,
    ) -> Result<Vec<(String, String, String)>, Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, title, COALESCE(summary, '') FROM wiki_pages WHERE status = 'active' ORDER BY title"
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    // ========== Wiki Page Sources ==========

    pub fn add_page_source(
        &self,
        page_id: &str,
        content_id: &str,
        compile_hash: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        conn.execute(
            "INSERT OR REPLACE INTO wiki_page_sources (page_id, content_id, compile_hash, source_status, contributed_at)
             VALUES (?1, ?2, ?3, 'active', datetime('now'))",
            params![page_id, content_id, compile_hash],
        )?;
        Ok(())
    }

    pub fn get_sources_for_page(
        &self,
        page_id: &str,
    ) -> Result<Vec<super::models::WikiPageSource>, Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, page_id, content_id, compile_hash, source_status, contributed_at FROM wiki_page_sources WHERE page_id = ?1"
        )?;
        let rows = stmt.query_map(params![page_id], |row| {
            Ok(super::models::WikiPageSource {
                id: row.get(0)?,
                page_id: row.get(1)?,
                content_id: row.get(2)?,
                compile_hash: row.get(3)?,
                source_status: row.get(4)?,
                contributed_at: row.get(5)?,
            })
        })?;
        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    pub fn get_pages_for_content(
        &self,
        content_id: &str,
    ) -> Result<Vec<super::models::WikiPageSource>, Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, page_id, content_id, compile_hash, source_status, contributed_at FROM wiki_page_sources WHERE content_id = ?1"
        )?;
        let rows = stmt.query_map(params![content_id], |row| {
            Ok(super::models::WikiPageSource {
                id: row.get(0)?,
                page_id: row.get(1)?,
                content_id: row.get(2)?,
                compile_hash: row.get(3)?,
                source_status: row.get(4)?,
                contributed_at: row.get(5)?,
            })
        })?;
        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    pub fn update_source_status(
        &self,
        page_id: &str,
        content_id: &str,
        status: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE wiki_page_sources SET source_status = ?1 WHERE page_id = ?2 AND content_id = ?3",
            params![status, page_id, content_id],
        )?;
        Ok(())
    }

    pub fn update_source_status_by_content(
        &self,
        content_id: &str,
        status: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE wiki_page_sources SET source_status = ?1 WHERE content_id = ?2",
            params![status, content_id],
        )?;
        Ok(())
    }

    pub fn count_active_sources(
        &self,
        page_id: &str,
    ) -> Result<(i64, i64), Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        let active: i64 = conn.query_row(
            "SELECT COUNT(*) FROM wiki_page_sources WHERE page_id = ?1 AND source_status = 'active'",
            params![page_id], |r| r.get(0),
        )?;
        let total: i64 = conn.query_row(
            "SELECT COUNT(*) FROM wiki_page_sources WHERE page_id = ?1",
            params![page_id],
            |r| r.get(0),
        )?;
        Ok((active, total))
    }

    pub fn delete_sources_for_page(&self, page_id: &str) -> Result<(), Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        conn.execute(
            "DELETE FROM wiki_page_sources WHERE page_id = ?1",
            params![page_id],
        )?;
        Ok(())
    }

    // ========== Wiki Edges ==========

    pub fn save_wiki_edge(
        &self,
        source_id: &str,
        target_id: &str,
        relation: &str,
        weight: f64,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        conn.execute(
            "INSERT OR REPLACE INTO wiki_edges (source_page_id, target_page_id, relation, weight, created_at)
             VALUES (?1, ?2, ?3, ?4, datetime('now'))",
            params![source_id, target_id, relation, weight],
        )?;
        Ok(())
    }

    pub fn get_edges_for_page(
        &self,
        page_id: &str,
    ) -> Result<Vec<super::models::WikiEdge>, Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, source_page_id, target_page_id, relation, weight, created_at
             FROM wiki_edges WHERE source_page_id = ?1 OR target_page_id = ?1",
        )?;
        let rows = stmt.query_map(params![page_id], |row| {
            Ok(super::models::WikiEdge {
                id: row.get(0)?,
                source_page_id: row.get(1)?,
                target_page_id: row.get(2)?,
                relation: row.get(3)?,
                weight: row.get(4)?,
                created_at: row.get(5)?,
            })
        })?;
        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    pub fn get_all_wiki_edges(
        &self,
    ) -> Result<Vec<super::models::WikiEdge>, Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, source_page_id, target_page_id, relation, weight, created_at FROM wiki_edges"
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(super::models::WikiEdge {
                id: row.get(0)?,
                source_page_id: row.get(1)?,
                target_page_id: row.get(2)?,
                relation: row.get(3)?,
                weight: row.get(4)?,
                created_at: row.get(5)?,
            })
        })?;
        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    pub fn delete_edges_for_page(&self, page_id: &str) -> Result<(), Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        conn.execute(
            "DELETE FROM wiki_edges WHERE source_page_id = ?1 OR target_page_id = ?1",
            params![page_id],
        )?;
        Ok(())
    }

    /// Delete all edges of a given relation type. Used when rebuilding the
    /// tag-based "related" graph from scratch with a new algorithm.
    pub fn delete_edges_by_relation(
        &self,
        relation: &str,
    ) -> Result<usize, Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        let n = conn.execute(
            "DELETE FROM wiki_edges WHERE relation = ?1",
            params![relation],
        )?;
        Ok(n)
    }

    pub fn migrate_wiki_edge_relations(&self) -> Result<usize, Box<dyn std::error::Error>> {
        const MIGRATION_KEY: &str = "wiki_edge_relations_v1";
        let mut conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        let tx = conn.transaction()?;
        let completed: Option<String> = tx
            .query_row(
                "SELECT value FROM app_settings WHERE key=?1",
                params![MIGRATION_KEY],
                |row| row.get(0),
            )
            .optional()?;
        if completed.as_deref() == Some("done") {
            tx.commit()?;
            return Ok(0);
        }

        let mut migrated = tx.execute(
            "INSERT OR IGNORE INTO wiki_edges (source_page_id, target_page_id, relation, weight, created_at)
             SELECT e.source_page_id, e.target_page_id, 'qa_reference', e.weight, e.created_at
             FROM wiki_edges e JOIN wiki_pages s ON s.id=e.source_page_id JOIN wiki_pages t ON t.id=e.target_page_id
             WHERE e.relation='related' AND (s.page_type='qa' OR t.page_type='qa')",
            [],
        )?;
        migrated += tx.execute(
            "INSERT OR IGNORE INTO wiki_edges (source_page_id, target_page_id, relation, weight, created_at)
             SELECT e.target_page_id, e.source_page_id, 'qa_reference', e.weight, e.created_at
             FROM wiki_edges e JOIN wiki_pages s ON s.id=e.source_page_id JOIN wiki_pages t ON t.id=e.target_page_id
             WHERE e.relation='related' AND (s.page_type='qa' OR t.page_type='qa')",
            [],
        )?;
        let qa_references = {
            let mut stmt = tx.prepare(
                "SELECT p.id, m.pages_used FROM wiki_pages p
                 JOIN wiki_chat_messages m ON m.id=p.source_message_id
                 WHERE p.page_type='qa' AND m.pages_used IS NOT NULL",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        for (qa_page_id, pages_used) in qa_references {
            for reference_id in parse_page_reference_ids(&pages_used) {
                for (source, target) in [(&qa_page_id, &reference_id), (&reference_id, &qa_page_id)]
                {
                    migrated += tx.execute(
                        "INSERT OR IGNORE INTO wiki_edges (source_page_id, target_page_id, relation, weight, created_at)
                         SELECT ?1, ?2, 'qa_reference', 1.0, datetime('now')
                         WHERE EXISTS (SELECT 1 FROM wiki_pages WHERE id=?1)
                           AND EXISTS (SELECT 1 FROM wiki_pages WHERE id=?2)",
                        params![source, target],
                    )?;
                }
            }
        }
        tx.execute("DELETE FROM wiki_edges WHERE relation='related'", [])?;
        tx.execute(
            "INSERT INTO app_settings (key, value, updated_at) VALUES (?1, 'done', datetime('now'))
             ON CONFLICT(key) DO UPDATE SET value='done', updated_at=datetime('now')",
            params![MIGRATION_KEY],
        )?;
        tx.commit()?;
        Ok(migrated)
    }

    // ========== Wiki Compile Log ==========

    pub fn acquire_compile_lock(
        &self,
        content_id: &str,
        content_hash: &str,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        match conn.execute(
            "INSERT INTO wiki_compile_log (content_id, content_hash, status, created_at)
             VALUES (?1, ?2, 'compiling', datetime('now'))",
            params![content_id, content_hash],
        ) {
            Ok(_) => Ok(true),
            Err(rusqlite::Error::SqliteFailure(e, _))
                if e.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                Ok(false)
            }
            Err(e) => Err(Box::new(e)),
        }
    }

    pub fn release_compile_lock(
        &self,
        content_id: &str,
        status: &str,
        pages_touched: Option<&str>,
        model_used: Option<&str>,
        error_message: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE wiki_compile_log SET status=?1, pages_touched=?2, model_used=?3, error_message=?4, compiled_at=datetime('now')
             WHERE content_id=?5 AND status='compiling'",
            params![status, pages_touched, model_used, error_message, content_id],
        )?;
        Ok(())
    }

    pub fn cleanup_stale_compile_locks(&self) -> Result<u64, Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        let count = conn.execute(
            "UPDATE wiki_compile_log SET status='error', error_message='stale lock cleaned on startup' WHERE status='compiling'",
            [],
        )?;
        Ok(count as u64)
    }

    // ========== Wiki Hash Updates ==========

    pub fn update_content_compile_hash(
        &self,
        content_id: &str,
        hash: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE captured_content SET wiki_compile_hash=?1, wiki_assessed_hash=?1 WHERE id=?2",
            params![hash, content_id],
        )?;
        Ok(())
    }

    pub fn update_content_assessed_hash(
        &self,
        content_id: &str,
        hash: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE captured_content SET wiki_assessed_hash=?1 WHERE id=?2",
            params![hash, content_id],
        )?;
        Ok(())
    }

    // ========== Wiki Conversations ==========

    pub fn save_wiki_conversation(
        &self,
        conv: &super::models::WikiConversation,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        conn.execute(
            "INSERT INTO wiki_conversations (id, question, answer, pages_used, saved_as_page, model_used, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![conv.id, conv.question, conv.answer, conv.pages_used, conv.saved_as_page, conv.model_used, conv.created_at],
        )?;
        Ok(())
    }

    pub fn get_wiki_conversations(
        &self,
        limit: i64,
    ) -> Result<Vec<super::models::WikiConversation>, Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, question, answer, pages_used, saved_as_page, model_used, created_at
             FROM wiki_conversations ORDER BY created_at DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit], |row| {
            Ok(super::models::WikiConversation {
                id: row.get(0)?,
                question: row.get(1)?,
                answer: row.get(2)?,
                pages_used: row.get(3)?,
                saved_as_page: row.get(4)?,
                model_used: row.get(5)?,
                created_at: row.get(6)?,
            })
        })?;
        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    pub fn update_conversation_saved_page(
        &self,
        conv_id: &str,
        page_id: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE wiki_conversations SET saved_as_page=?1 WHERE id=?2",
            params![page_id, conv_id],
        )?;
        Ok(())
    }

    // ========== Wiki Lint ==========

    pub fn save_lint_result(
        &self,
        lint_type: &str,
        severity: &str,
        title: &str,
        description: &str,
        page_ids: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        let updated = conn.execute(
            "UPDATE wiki_lint_results
             SET severity=?2, title=?3, description=?4, created_at=datetime('now')
             WHERE id = (
                 SELECT MIN(id) FROM wiki_lint_results
                 WHERE lint_type=?1 AND page_ids=?5 AND status='open'
             )",
            params![lint_type, severity, title, description, page_ids],
        )?;
        if updated == 0 {
            conn.execute(
                "INSERT INTO wiki_lint_results (lint_type, severity, title, description, page_ids, status, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'open', datetime('now'))",
                params![lint_type, severity, title, description, page_ids],
            )?;
        } else {
            conn.execute(
                "UPDATE wiki_lint_results SET status='resolved'
                 WHERE lint_type=?1 AND page_ids=?2 AND status='open'
                   AND id <> (
                       SELECT MIN(id) FROM wiki_lint_results
                       WHERE lint_type=?1 AND page_ids=?2 AND status='open'
                   )",
                params![lint_type, page_ids],
            )?;
        }
        Ok(())
    }

    pub fn get_open_lint_results(
        &self,
    ) -> Result<Vec<super::models::WikiLintResult>, Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, lint_type, severity, title, description, page_ids, status, created_at
             FROM wiki_lint_results WHERE status = 'open' ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(super::models::WikiLintResult {
                id: row.get(0)?,
                lint_type: row.get(1)?,
                severity: row.get(2)?,
                title: row.get(3)?,
                description: row.get(4)?,
                page_ids: row.get(5)?,
                status: row.get(6)?,
                created_at: row.get(7)?,
            })
        })?;
        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    pub fn resolve_lint_result(&self, id: i64) -> Result<(), Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE wiki_lint_results SET status='resolved' WHERE id=?1",
            params![id],
        )?;
        Ok(())
    }

    /// Recalculate confidence for a page based on its source health.
    pub fn recalculate_page_confidence(
        &self,
        page_id: &str,
    ) -> Result<f64, Box<dyn std::error::Error>> {
        let (active, total) = self.count_active_sources(page_id)?;
        let confidence = if total == 0 {
            0.3
        } else {
            active as f64 / total as f64
        };
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE wiki_pages SET confidence=?1, updated_at=datetime('now') WHERE id=?2",
            params![confidence, page_id],
        )?;
        Ok(confidence)
    }

    pub fn get_pages_needing_recompile(
        &self,
    ) -> Result<Vec<super::models::WikiPage>, Box<dyn std::error::Error>> {
        self.get_wiki_pages_by_status("needs_recompile")
    }

    // ========== Wiki Chat Sessions ==========

    pub fn create_chat_session(
        &self,
        id: &str,
        title: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        conn.execute(
            "INSERT INTO wiki_chat_sessions (id, title, created_at, updated_at) VALUES (?1, ?2, datetime('now'), datetime('now'))",
            params![id, title],
        )?;
        Ok(())
    }

    pub fn get_chat_sessions(
        &self,
        limit: i64,
    ) -> Result<Vec<super::models::WikiChatSession>, Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, title, created_at, updated_at FROM wiki_chat_sessions ORDER BY updated_at DESC LIMIT ?1"
        )?;
        let rows = stmt.query_map(params![limit], |row| {
            Ok(super::models::WikiChatSession {
                id: row.get(0)?,
                title: row.get(1)?,
                created_at: row.get(2)?,
                updated_at: row.get(3)?,
            })
        })?;
        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    pub fn update_chat_session_title(
        &self,
        session_id: &str,
        title: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE wiki_chat_sessions SET title=?1, updated_at=datetime('now') WHERE id=?2",
            params![title, session_id],
        )?;
        Ok(())
    }

    pub fn touch_chat_session(&self, session_id: &str) -> Result<(), Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE wiki_chat_sessions SET updated_at=datetime('now') WHERE id=?1",
            params![session_id],
        )?;
        Ok(())
    }

    pub fn delete_chat_session(&self, session_id: &str) -> Result<(), Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        conn.execute(
            "DELETE FROM wiki_chat_sessions WHERE id=?1",
            params![session_id],
        )?;
        Ok(())
    }

    // ========== Wiki Chat Messages ==========

    pub fn add_chat_message(
        &self,
        msg: &super::models::WikiChatMessage,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        conn.execute(
            "INSERT INTO wiki_chat_messages (id, session_id, role, content, pages_used, source_mode, turn_index, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, datetime('now'))",
            params![msg.id, msg.session_id, msg.role, msg.content, msg.pages_used, msg.source_mode, msg.turn_index],
        )?;
        Ok(())
    }

    pub fn get_chat_messages(
        &self,
        session_id: &str,
    ) -> Result<Vec<super::models::WikiChatMessage>, Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, session_id, role, content, pages_used, source_mode, turn_index, created_at
             FROM wiki_chat_messages WHERE session_id=?1 ORDER BY turn_index ASC",
        )?;
        let rows = stmt.query_map(params![session_id], |row| {
            Ok(super::models::WikiChatMessage {
                id: row.get(0)?,
                session_id: row.get(1)?,
                role: row.get(2)?,
                content: row.get(3)?,
                pages_used: row.get(4)?,
                source_mode: row.get(5)?,
                turn_index: row.get(6)?,
                created_at: row.get(7)?,
            })
        })?;
        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    pub fn update_chat_message_sources(
        &self,
        message_id: &str,
        pages_used: &str,
        source_mode: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE wiki_chat_messages
             SET pages_used = ?1, source_mode = ?2
             WHERE id = ?3",
            params![pages_used, source_mode, message_id],
        )?;
        Ok(())
    }

    pub fn get_next_turn_index(&self, session_id: &str) -> Result<i32, Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        let max: Option<i32> = conn.query_row(
            "SELECT MAX(turn_index) FROM wiki_chat_messages WHERE session_id=?1",
            params![session_id],
            |r| r.get(0),
        )?;
        Ok(max.unwrap_or(-1) + 1)
    }

    pub fn get_wiki_page_by_source_message_id(
        &self,
        message_id: &str,
    ) -> Result<Option<super::models::WikiPage>, Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, title, slug, page_type, body_markdown, summary, tags, status, confidence, created_at, updated_at, last_compiled_at, source_message_id
             FROM wiki_pages WHERE source_message_id = ?1"
        )?;
        let mut rows = stmt.query_map(params![message_id], |row| {
            Ok(super::models::WikiPage {
                id: row.get(0)?,
                title: row.get(1)?,
                slug: row.get(2)?,
                page_type: row.get(3)?,
                body_markdown: row.get(4)?,
                summary: row.get(5)?,
                tags: row.get(6)?,
                status: row.get(7)?,
                confidence: row.get(8)?,
                created_at: row.get(9)?,
                updated_at: row.get(10)?,
                last_compiled_at: row.get(11)?,
                source_message_id: row.get(12).unwrap_or(None),
            })
        })?;
        match rows.next() {
            Some(Ok(page)) => Ok(Some(page)),
            Some(Err(e)) => Err(Box::new(e)),
            None => Ok(None),
        }
    }

    pub fn get_active_wiki_page_titles(
        &self,
    ) -> Result<Vec<(String, String)>, Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, title
             FROM wiki_pages
             WHERE status IN ('active', 'needs_recompile') AND page_type != 'qa'",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    /// Get page summaries for Q&A retrieval, excluding qa-type pages.
    pub fn get_wiki_page_summaries_for_qa(
        &self,
    ) -> Result<Vec<(String, String, String)>, Box<dyn std::error::Error>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, title, COALESCE(summary, substr(body_markdown, 1, 100))
             FROM wiki_pages WHERE status = 'active' AND page_type != 'qa' ORDER BY title",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    /// Whether the FTS5 virtual table exists. False means migration 014
    /// failed to apply (older sqlite without FTS5) — callers should fall
    /// back to the full-index APIs above.
    pub fn fts_available(&self) -> bool {
        let conn = match self.db.conn.lock() {
            Ok(c) => c,
            Err(_) => return false,
        };
        conn.prepare("SELECT 1 FROM sqlite_master WHERE type='table' AND name='wiki_pages_fts'")
            .and_then(|mut s| s.query_row([], |row| row.get::<_, i32>(0)))
            .is_ok()
    }

    /// Build an FTS5 MATCH expression from a free-form user query.
    ///
    /// Two transforms applied:
    ///
    /// 1. **CJK segmentation** — same `cjk_segment()` used at index time
    ///    (registered as `cjk_seg()` SQL function and applied by triggers).
    ///    This guarantees query tokens line up with index tokens. Without
    ///    it, querying "设计" would never match "整理设计风格" because
    ///    unicode61 indexes the latter as one big token.
    ///
    /// 2. **Syntax sanitization** — FTS5 reserves a handful of chars
    ///    (`" * : ^ ( ) - +`) that would either be parsed specially or
    ///    raise a syntax error. We strip them.
    ///
    /// Then split on whitespace, quote each token (which becomes an
    /// adjacent-token phrase match for CJK after segmentation), and OR
    /// them together for broad recall — Q&A wants high recall, the AI
    /// does the precision step downstream.
    fn build_fts_match(query: &str) -> String {
        // 1. Strip FTS5 syntax chars first so they don't survive into the
        //    phrase quoting step.
        let cleaned: String = query
            .chars()
            .map(|c| match c {
                '"' | '\'' | '*' | ':' | '^' | '(' | ')' | '-' | '+' => ' ',
                _ => c,
            })
            .collect();
        // 2. Split on the user's original whitespace into words. Each
        //    word becomes a quoted phrase in the OR'd MATCH expression.
        //    For CJK words, cjk_segment turns each character into its
        //    own token, and the quoted phrase becomes an adjacency
        //    match (e.g. "设" followed immediately by "计").
        let phrases: Vec<String> = cleaned
            .split_whitespace()
            .filter(|w| !w.is_empty())
            .map(|w| format!("\"{}\"", super::database::cjk_segment(w)))
            .collect();
        phrases.join(" OR ")
    }

    /// Extract the best clickable link from a wiki page body. Pages
    /// frequently include a "## 项目地址" or "## Links" section with
    /// the actual project URL — that's far more useful to surface than
    /// the source_url (which is often the user's tweet/article they
    /// happened to copy from).
    ///
    /// Priority:
    ///   1. First github.com / gitlab.com / bitbucket.org link
    ///   2. First non-social-media http(s) link
    ///   3. None (caller falls back to source_url)
    fn extract_project_url_from_body(body: &str) -> Option<String> {
        use std::sync::OnceLock;
        static RE: OnceLock<regex::Regex> = OnceLock::new();
        let re = RE.get_or_init(|| {
            // URL stops at whitespace, Markdown bracket close, or CJK punctuation.
            // Inside [...], `)` `]` `>` don't need escaping.
            regex::Regex::new(r"https?://[^\s)\]>，。、；：！？]+").expect("regex compiles")
        });

        let trim_trailing = |s: &str| -> String {
            s.trim_end_matches(|c: char| {
                matches!(
                    c,
                    '.' | ','
                        | ';'
                        | ':'
                        | '?'
                        | '!'
                        | '。'
                        | '，'
                        | '、'
                        | '；'
                        | '：'
                        | '！'
                        | '？'
                )
            })
            .to_string()
        };

        let is_repo = |u: &str| {
            u.contains("github.com")
                || u.contains("gitlab.com")
                || u.contains("bitbucket.org")
                || u.contains("huggingface.co")
        };
        let is_social = |u: &str| {
            u.contains("twitter.com")
                || u.contains("x.com/")
                || u.contains("weibo.com")
                || u.contains("mp.weixin.qq.com")
                || u.contains("xiaohongshu.com")
                || u.contains("douyin.com")
                || u.contains("youtube.com/watch")
                || u.contains("bilibili.com")
        };

        let mut fallback: Option<String> = None;
        for m in re.find_iter(body) {
            let url = trim_trailing(m.as_str());
            if is_repo(&url) {
                return Some(url);
            }
            if fallback.is_none() && !is_social(&url) {
                fallback = Some(url);
            }
        }
        fallback
    }

    /// Returns (id, title, summary, created_at, best_url) candidates
    /// for AI prompts, pre-filtered in SQL.
    ///
    /// `best_url` priority:
    ///   1. GitHub / GitLab / HuggingFace project link extracted from body_markdown
    ///   2. Any non-social-media http link in body_markdown
    ///   3. source_url (the page the user originally captured from)
    ///   4. None
    ///
    /// Why: when a user asks "what was that design skill", they want to
    /// click through to the actual project (a GitHub repo) — not back to
    /// the tweet they happened to save it from.
    ///
    /// - `fts_query`: optional free-form text. None = no FTS filter.
    /// - `date_start`/`date_end`: optional ISO-8601 timestamps. Filters
    ///   `created_at` lexically (works because we store ISO-8601 UTC).
    /// - `exclude_qa`: true for Q&A retrieval (Q&A pages would create a
    ///   feedback loop), false for compile (compile may merge into Q&A
    ///   pages legitimately).
    /// - `limit`: max candidates to return. Recommend 50–100 for AI prompts.
    ///
    /// Falls back to a non-FTS query when FTS is unavailable or when
    /// `fts_query` produces no usable tokens.
    pub fn get_wiki_page_candidates(
        &self,
        fts_query: Option<&str>,
        date_start: Option<&str>,
        date_end: Option<&str>,
        exclude_qa: bool,
        limit: i64,
    ) -> Result<Vec<(String, String, String, String, Option<String>)>, Box<dyn std::error::Error>>
    {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;

        let match_expr: Option<String> = fts_query
            .map(Self::build_fts_match)
            .filter(|s| !s.is_empty());

        let fts_table_exists = conn
            .prepare("SELECT 1 FROM sqlite_master WHERE type='table' AND name='wiki_pages_fts'")
            .and_then(|mut s| s.query_row([], |row| row.get::<_, i32>(0)))
            .is_ok();
        let use_fts = match_expr.is_some() && fts_table_exists;

        // We pull the source URL via a correlated subquery — picks one
        // active source per page (most recently contributed). NULL when
        // a page has no active source (rare but possible for pages built
        // entirely from chat answers).
        let url_subq = "(SELECT cc.source_url FROM wiki_page_sources wps \
            JOIN captured_content cc ON cc.id = wps.content_id \
            WHERE wps.page_id = wp.id AND wps.source_status = 'active' \
            AND cc.source_url IS NOT NULL AND cc.source_url != '' \
            ORDER BY wps.contributed_at DESC LIMIT 1)";

        let mut sql = String::new();
        let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        // We also fetch the first ~3000 chars of body_markdown so we can
        // mine a project link from it in Rust. Project links typically
        // sit in a "## 项目地址" section near the top.
        if use_fts {
            sql.push_str(&format!(
                "SELECT wp.id, wp.title, COALESCE(wp.summary, substr(wp.body_markdown, 1, 100)), wp.created_at, {}, substr(wp.body_markdown, 1, 3000) \
                 FROM wiki_pages_fts fts \
                 JOIN wiki_pages wp ON wp.id = fts.page_id \
                 WHERE wiki_pages_fts MATCH ? AND wp.status = 'active'",
                url_subq
            ));
            params.push(Box::new(match_expr.clone().unwrap()));
        } else {
            sql.push_str(&format!(
                "SELECT wp.id, wp.title, COALESCE(wp.summary, substr(wp.body_markdown, 1, 100)), wp.created_at, {}, substr(wp.body_markdown, 1, 3000) \
                 FROM wiki_pages wp \
                 WHERE wp.status = 'active'",
                url_subq
            ));
        }

        if exclude_qa {
            sql.push_str(" AND wp.page_type != 'qa'");
        }
        if let Some(s) = date_start {
            sql.push_str(" AND wp.created_at >= ?");
            params.push(Box::new(s.to_string()));
        }
        if let Some(e) = date_end {
            sql.push_str(" AND wp.created_at <= ?");
            params.push(Box::new(e.to_string()));
        }

        if use_fts {
            sql.push_str(" ORDER BY rank");
        } else if date_start.is_some() || date_end.is_some() {
            sql.push_str(" ORDER BY wp.created_at DESC");
        } else {
            sql.push_str(" ORDER BY wp.title");
        }
        sql.push_str(" LIMIT ?");
        params.push(Box::new(limit));

        let mut stmt = conn.prepare(&sql)?;
        let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|b| b.as_ref()).collect();
        let rows = stmt.query_map(param_refs.as_slice(), |row| {
            // Tuple shape returned to caller stays at 5 elements; the
            // body excerpt is consumed here to compute best_url and not
            // forwarded.
            let id: String = row.get(0)?;
            let title: String = row.get(1)?;
            let summary: String = row.get(2)?;
            let created_at: String = row.get(3)?;
            let source_url: Option<String> = row.get(4)?;
            let body_excerpt: String = row.get::<_, Option<String>>(5)?.unwrap_or_default();
            let best_url = Self::extract_project_url_from_body(&body_excerpt).or(source_url);
            Ok((id, title, summary, created_at, best_url))
        })?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::database::Database;
    use crate::storage::models::{CapturedContent, ContentType};

    /// Create an in-memory database with all migrations applied.
    fn test_db() -> Arc<Database> {
        let db = Database::new_in_memory().expect("Failed to create test DB");
        Arc::new(db)
    }

    fn make_content(id: &str, captured_at: &str) -> CapturedContent {
        CapturedContent {
            id: id.to_string(),
            content_type: ContentType::Text,
            raw_text: Some(format!("Test content {}", id)),
            image_path: None,
            thumbnail_path: None,
            source_app: "TestApp".to_string(),
            source_bundle_id: None,
            source_url: None,
            user_note: None,
            captured_at: captured_at.to_string(),
            content_hash: format!("hash_{}", id),
            byte_size: 100,
            is_deleted: false,
            created_at: captured_at.to_string(),
            updated_at: captured_at.to_string(),
            digested_at: None,
            digest_action: None,
            summary: None,
            tags: None,
            digest: None,
            wiki_compile_hash: None,
            wiki_assessed_hash: None,
            clean_content: None,
            category: None,
        }
    }

    #[test]
    fn get_content_by_id_finds_items_beyond_first_500() {
        let db = test_db();
        let repo = Repository::new(db);

        for i in 0..=500 {
            let id = format!("item_{i}");
            let content = make_content(&id, &format!("2025-01-01T10:{:02}:00", i % 60));
            repo.save_content(&content).unwrap();
        }

        let item = repo.get_content_by_id("item_500").unwrap().unwrap();
        assert_eq!(item.id, "item_500");
    }

    #[test]
    fn filtered_pagination_is_stable_across_three_hundred_items() {
        let db = test_db();
        let repo = Repository::new(db);
        for index in 0..300 {
            let mut content = make_content(&format!("item-{index:03}"), "2026-07-01T12:00:00Z");
            content.content_type = match index % 3 {
                0 => ContentType::Text,
                1 => ContentType::Image,
                _ => ContentType::Url,
            };
            if index % 10 == 0 {
                content.source_app = "导入内容".to_string();
            }
            repo.save_content(&content).unwrap();
        }

        let (first, total, counts) = repo.query_content("image", None, false, 50, 0).unwrap();
        let (second, _, _) = repo.query_content("image", None, false, 50, 50).unwrap();
        assert_eq!(total, 90);
        assert_eq!(counts["all"], 300);
        assert_eq!(counts["document"], 30);
        assert_eq!(first.len(), 50);
        assert_eq!(second.len(), 40);
        let ids: std::collections::HashSet<_> =
            first.iter().chain(&second).map(|item| &item.id).collect();
        assert_eq!(ids.len(), 90);
        assert_eq!(repo.content_position("item-049", false).unwrap(), Some(250));
    }

    #[test]
    fn query_filters_sensitive_content_and_date_before_counting() {
        let db = test_db();
        let repo = Repository::new(db);
        let normal = make_content("normal", "2026-07-10T00:00:00Z");
        let mut sensitive = make_content("sensitive", "2026-07-11T00:00:00Z");
        sensitive.raw_text = Some("password=mysecretpass".to_string());
        let old = make_content("old", "2026-06-01T00:00:00Z");
        repo.save_content(&normal).unwrap();
        repo.save_content(&sensitive).unwrap();
        repo.save_content(&old).unwrap();

        let (items, total, counts) = repo
            .query_content("all", Some("2026-07-01T00:00:00Z"), true, 50, 0)
            .unwrap();
        assert_eq!(
            items
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            vec!["normal"]
        );
        assert_eq!(total, 1);
        assert_eq!(counts["all"], 1);
        assert_eq!(repo.content_position("sensitive", true).unwrap(), None);
    }

    #[test]
    fn save_lint_result_updates_one_open_record_and_resolves_duplicates() {
        let db = test_db();
        {
            let conn = db.conn.lock().unwrap();
            for title in ["Old title 1", "Old title 2"] {
                conn.execute(
                    "INSERT INTO wiki_lint_results
                     (lint_type, severity, title, description, page_ids, status)
                     VALUES ('orphan', 'critical', ?1, 'Old description', '[\"page-1\"]', 'open')",
                    params![title],
                )
                .unwrap();
            }
        }
        let repo = Repository::new(db);

        repo.save_lint_result(
            "orphan",
            "warning",
            "Updated title",
            "Updated description",
            "[\"page-1\"]",
        )
        .unwrap();

        let open = repo.get_open_lint_results().unwrap();
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].title, "Updated title");
        assert_eq!(open[0].description, "Updated description");
        assert_eq!(open[0].severity, "warning");
    }

    #[test]
    fn test_get_content_stats_counts_total_days_inclusively() {
        let items = vec![
            ContentForAnalysis {
                id: "1".to_string(),
                raw_text: Some("a".to_string()),
                source_url: None,
                captured_at: "2026-03-21T10:00:00Z".to_string(),
                summary: None,
                tags: None,
                user_note: Some("note".to_string()),
                source_app: "WeChat".to_string(),
                content_type: "text".to_string(),
            },
            ContentForAnalysis {
                id: "2".to_string(),
                raw_text: Some("b".to_string()),
                source_url: None,
                captured_at: "2026-04-05T09:00:00Z".to_string(),
                summary: None,
                tags: Some("tag".to_string()),
                user_note: None,
                source_app: "Chrome".to_string(),
                content_type: "url".to_string(),
            },
        ];

        let stats = Repository::get_content_stats(&items);

        assert_eq!(stats["total_days"], 16);
        assert_eq!(stats["source_count"], 2);
        assert_eq!(stats["annotation_rate"], "100%");
    }

    #[test]
    fn test_get_undigested_returns_oldest_first() {
        let db = test_db();
        let repo = Repository::new(db);

        // Insert 5 items with different timestamps
        for i in 1..=5 {
            let content = make_content(
                &format!("item_{}", i),
                &format!("2025-01-{:02}T10:00:00", i),
            );
            repo.save_content(&content).unwrap();
        }

        let items = repo.get_undigested_content(3).unwrap();
        assert_eq!(items.len(), 3);
        // Should be oldest first
        assert_eq!(items[0].id, "item_1");
        assert_eq!(items[1].id, "item_2");
        assert_eq!(items[2].id, "item_3");
    }

    #[test]
    fn test_get_undigested_skips_digested() {
        let db = test_db();
        let repo = Repository::new(db);

        for i in 1..=3 {
            let content = make_content(
                &format!("item_{}", i),
                &format!("2025-01-{:02}T10:00:00", i),
            );
            repo.save_content(&content).unwrap();
        }

        // Digest item_1
        repo.update_digest_action("item_1", "keep").unwrap();

        let items = repo.get_undigested_content(5).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].id, "item_2");
        assert_eq!(items[1].id, "item_3");
    }

    #[test]
    fn test_get_undigested_empty_when_all_digested() {
        let db = test_db();
        let repo = Repository::new(db);

        let content = make_content("item_1", "2025-01-01T10:00:00");
        repo.save_content(&content).unwrap();
        repo.update_digest_action("item_1", "archive").unwrap();

        let items = repo.get_undigested_content(5).unwrap();
        assert!(items.is_empty());
    }

    #[test]
    fn test_update_digest_keep() {
        let db = test_db();
        let repo = Repository::new(db);

        let content = make_content("item_1", "2025-01-01T10:00:00");
        repo.save_content(&content).unwrap();

        repo.update_digest_action("item_1", "keep").unwrap();

        // Verify by fetching — it should no longer be undigested
        let undigested = repo.get_undigested_content(5).unwrap();
        assert!(undigested.is_empty());

        // Verify the action was set correctly
        let item = repo.get_content_by_id("item_1").unwrap().unwrap();
        assert_eq!(item.digest_action.as_deref(), Some("keep"));
        assert!(item.digested_at.is_some());
    }

    #[test]
    fn test_update_digest_archive() {
        let db = test_db();
        let repo = Repository::new(db);

        let content = make_content("item_1", "2025-01-01T10:00:00");
        repo.save_content(&content).unwrap();

        repo.update_digest_action("item_1", "archive").unwrap();

        let item = repo.get_content_by_id("item_1").unwrap().unwrap();
        assert_eq!(item.digest_action.as_deref(), Some("archive"));
        assert!(item.digested_at.is_some());
    }

    #[test]
    fn test_update_digest_pin() {
        let db = test_db();
        let repo = Repository::new(db);

        let content = make_content("item_1", "2025-01-01T10:00:00");
        repo.save_content(&content).unwrap();

        repo.update_digest_action("item_1", "pin").unwrap();

        let item = repo.get_content_by_id("item_1").unwrap().unwrap();
        assert_eq!(item.digest_action.as_deref(), Some("pin"));
        assert!(item.digested_at.is_some());
    }

    #[test]
    fn test_update_digest_invalid_id() {
        let db = test_db();
        let repo = Repository::new(db);

        let result = repo.update_digest_action("nonexistent", "keep");
        assert!(result.is_err());
    }

    #[test]
    fn test_count_undigested() {
        let db = test_db();
        let repo = Repository::new(db);

        for i in 1..=5 {
            let content = make_content(
                &format!("item_{}", i),
                &format!("2025-01-{:02}T10:00:00", i),
            );
            repo.save_content(&content).unwrap();
        }

        assert_eq!(repo.count_undigested().unwrap(), 5);

        repo.update_digest_action("item_1", "archive").unwrap();
        repo.update_digest_action("item_2", "keep").unwrap();

        assert_eq!(repo.count_undigested().unwrap(), 3);
    }

    // ========== FTS / candidate retrieval ==========

    fn make_wiki_page(
        id: &str,
        title: &str,
        summary: &str,
        body: &str,
        created_at: &str,
        page_type: &str,
    ) -> super::super::models::WikiPage {
        super::super::models::WikiPage {
            id: id.to_string(),
            title: title.to_string(),
            slug: format!("slug-{}", id),
            page_type: page_type.to_string(),
            body_markdown: body.to_string(),
            summary: Some(summary.to_string()),
            tags: None,
            status: "active".to_string(),
            confidence: 1.0,
            created_at: created_at.to_string(),
            updated_at: created_at.to_string(),
            last_compiled_at: None,
            source_message_id: None,
        }
    }

    #[test]
    fn wiki_edge_migration_is_idempotent_and_restores_qa_references() {
        use super::super::models::WikiChatMessage;

        let db = test_db();
        let repo = Repository::new(db.clone());
        for id in ["source-1", "source-2", "ordinary"] {
            repo.save_wiki_page(&make_wiki_page(
                id,
                id,
                "summary",
                "body",
                "2026-07-12T10:00:00Z",
                "concept",
            ))
            .unwrap();
        }
        let mut qa_page = make_wiki_page(
            "qa-page",
            "Question",
            "summary",
            "body",
            "2026-07-12T10:00:00Z",
            "qa",
        );
        qa_page.source_message_id = Some("answer-1".to_string());
        repo.save_wiki_page(&qa_page).unwrap();
        repo.create_chat_session("session-1", None).unwrap();
        repo.add_chat_message(&WikiChatMessage {
            id: "answer-1".to_string(),
            session_id: "session-1".to_string(),
            role: "assistant".to_string(),
            content: "answer".to_string(),
            pages_used: Some(
                r#"["source-1",{"id":"source-2","title":"Source 2"},"missing"]"#.to_string(),
            ),
            source_mode: Some("knowledge_base".to_string()),
            turn_index: 0,
            created_at: "2026-07-12T10:00:00Z".to_string(),
        })
        .unwrap();
        repo.save_wiki_edge("qa-page", "source-1", "related", 1.0)
            .unwrap();
        repo.save_wiki_edge("source-1", "ordinary", "related", 0.5)
            .unwrap();

        assert!(repo.migrate_wiki_edge_relations().unwrap() >= 3);
        let first = repo.get_all_wiki_edges().unwrap();
        assert!(first.iter().all(|edge| edge.relation != "related"));
        for source in ["source-1", "source-2"] {
            assert!(first.iter().any(|edge| {
                edge.source_page_id == "qa-page"
                    && edge.target_page_id == source
                    && edge.relation == "qa_reference"
            }));
            assert!(first.iter().any(|edge| {
                edge.source_page_id == source
                    && edge.target_page_id == "qa-page"
                    && edge.relation == "qa_reference"
            }));
        }
        assert!(first.iter().all(|edge| edge.target_page_id != "missing"));

        assert_eq!(repo.migrate_wiki_edge_relations().unwrap(), 0);
        let second = repo.get_all_wiki_edges().unwrap();
        assert_eq!(first.len(), second.len());

        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "UPDATE wiki_pages SET tags='[\"shared\"]' WHERE id IN ('source-1','source-2')",
                [],
            )
            .unwrap();
            conn.execute(
                "UPDATE wiki_pages SET tags='[\"other\"]' WHERE id='ordinary'",
                [],
            )
            .unwrap();
        }
        repo.save_wiki_edge("ordinary", "source-1", "similar", 0.1)
            .unwrap();
        crate::ai::wiki_engine::link_pages_by_shared_tags(db).unwrap();
        let rebuilt = repo.get_all_wiki_edges().unwrap();
        assert!(rebuilt.iter().any(|edge| edge.relation == "similar"));
        assert!(!rebuilt.iter().any(|edge| {
            edge.source_page_id == "ordinary"
                && edge.target_page_id == "source-1"
                && edge.relation == "similar"
        }));
        assert_eq!(
            rebuilt
                .iter()
                .filter(|edge| edge.relation == "qa_reference")
                .count(),
            4
        );
    }

    #[test]
    fn fts_table_is_available_after_migration() {
        let db = test_db();
        let repo = Repository::new(db);
        assert!(
            repo.fts_available(),
            "FTS table should exist after migrations"
        );
    }

    #[test]
    fn fts_indexes_inserted_pages_via_trigger() {
        let db = test_db();
        let repo = Repository::new(db);
        repo.save_wiki_page(&make_wiki_page(
            "p1",
            "RAG technology",
            "Retrieval-augmented generation overview",
            "RAG combines retrieval with LLMs.",
            "2026-04-26T10:00:00Z",
            "concept",
        ))
        .unwrap();

        let candidates = repo
            .get_wiki_page_candidates(Some("RAG"), None, None, true, 10)
            .unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].0, "p1");
    }

    #[test]
    fn fts_delete_trigger_removes_from_index() {
        let db = test_db();
        let repo = Repository::new(db);
        repo.save_wiki_page(&make_wiki_page(
            "p1",
            "DeepSeek model",
            "DeepSeek V4 release notes",
            "DeepSeek is...",
            "2026-04-26T10:00:00Z",
            "concept",
        ))
        .unwrap();
        repo.delete_wiki_page("p1").unwrap();

        let candidates = repo
            .get_wiki_page_candidates(Some("DeepSeek"), None, None, true, 10)
            .unwrap();
        assert_eq!(candidates.len(), 0);
    }

    #[test]
    fn date_range_filter_narrows_results() {
        let db = test_db();
        let repo = Repository::new(db);
        repo.save_wiki_page(&make_wiki_page(
            "old",
            "Old page",
            "old",
            "old body",
            "2026-01-01T10:00:00Z",
            "concept",
        ))
        .unwrap();
        repo.save_wiki_page(&make_wiki_page(
            "new",
            "New page",
            "new",
            "new body",
            "2026-04-25T10:00:00Z",
            "concept",
        ))
        .unwrap();

        let recent = repo
            .get_wiki_page_candidates(None, Some("2026-04-01T00:00:00Z"), None, true, 10)
            .unwrap();
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].0, "new");
    }

    #[test]
    fn excludes_qa_pages_when_requested() {
        let db = test_db();
        let repo = Repository::new(db);
        repo.save_wiki_page(&make_wiki_page(
            "concept-1",
            "Buffett",
            "Investment philosophy",
            "Value investing.",
            "2026-04-25T10:00:00Z",
            "concept",
        ))
        .unwrap();
        repo.save_wiki_page(&make_wiki_page(
            "qa-1",
            "Buffett FAQ",
            "Past Q&A about Buffett",
            "What did Buffett say...",
            "2026-04-25T10:00:00Z",
            "qa",
        ))
        .unwrap();

        let qa_excluded = repo
            .get_wiki_page_candidates(Some("Buffett"), None, None, true, 10)
            .unwrap();
        assert_eq!(qa_excluded.len(), 1);
        assert_eq!(qa_excluded[0].0, "concept-1");

        let qa_included = repo
            .get_wiki_page_candidates(Some("Buffett"), None, None, false, 10)
            .unwrap();
        assert_eq!(qa_included.len(), 2);
    }

    #[test]
    fn empty_fts_query_falls_back_to_no_filter() {
        let db = test_db();
        let repo = Repository::new(db);
        repo.save_wiki_page(&make_wiki_page(
            "p1",
            "Anything",
            "any",
            "body",
            "2026-04-25T10:00:00Z",
            "concept",
        ))
        .unwrap();

        // Query with only FTS-syntax chars resolves to empty match expr
        // → falls back to non-FTS query, returns the page
        let r = repo
            .get_wiki_page_candidates(Some("*-+:"), None, None, true, 10)
            .unwrap();
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn fts_query_sanitizes_special_chars() {
        let db = test_db();
        let repo = Repository::new(db);
        repo.save_wiki_page(&make_wiki_page(
            "p1",
            "Hello world",
            "summary",
            "body",
            "2026-04-25T10:00:00Z",
            "concept",
        ))
        .unwrap();

        // Quotes and other FTS5 syntax chars should not crash — they
        // get stripped before the MATCH expression is built.
        let r = repo
            .get_wiki_page_candidates(Some("\"hello\" -world :extra"), None, None, true, 10)
            .unwrap();
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn fts_finds_cjk_substring_in_continuous_chinese() {
        // The bug this guards against: unicode61 indexed continuous CJK
        // sequences as a single token, so "整理设计风格" was one token
        // and a search for "设计" missed it. Migration 015 + cjk_seg()
        // forces character-level tokenization.
        let db = test_db();
        let repo = Repository::new(db);
        repo.save_wiki_page(&make_wiki_page(
            "design-md",
            "awesome-design-md",
            "整理设计风格并支持开源共建",
            "项目整理各大网站的设计风格用于喂给 AI",
            "2026-04-25T10:00:00Z",
            "source",
        ))
        .unwrap();

        // Query "设计" must hit the page even though "设计" is buried
        // in the middle of a continuous Chinese string.
        let r = repo
            .get_wiki_page_candidates(Some("设计"), None, None, true, 10)
            .unwrap();
        assert_eq!(r.len(), 1, "expected to find page with 设计 in summary");
        assert_eq!(r[0].0, "design-md");
    }

    #[test]
    fn fts_mixed_cjk_and_english_query() {
        // Real-world Q&A query shape: extracted keywords mix Chinese
        // topic words with English technical terms.
        let db = test_db();
        let repo = Repository::new(db);
        repo.save_wiki_page(&make_wiki_page(
            "p1",
            "awesome-design-md",
            "整理设计风格并支持开源共建",
            "body",
            "2026-04-25T10:00:00Z",
            "source",
        ))
        .unwrap();
        repo.save_wiki_page(&make_wiki_page(
            "p2",
            "NanoBanana-PPT-Skills",
            "AI 生成 PPT 的 Skill",
            "body",
            "2026-04-25T10:00:00Z",
            "concept",
        ))
        .unwrap();
        repo.save_wiki_page(&make_wiki_page(
            "p3",
            "无关页面",
            "完全不相关",
            "body",
            "2026-04-25T10:00:00Z",
            "concept",
        ))
        .unwrap();

        // Query "设计 skill" → should pull p1 (matches 设计) AND p2
        // (matches skill) but not p3.
        let r = repo
            .get_wiki_page_candidates(Some("设计 skill"), None, None, true, 10)
            .unwrap();
        let ids: Vec<&str> = r.iter().map(|t| t.0.as_str()).collect();
        assert!(ids.contains(&"p1"), "expected p1 (设计 match)");
        assert!(ids.contains(&"p2"), "expected p2 (skill match)");
        assert!(!ids.contains(&"p3"), "p3 should not match");
    }

    #[test]
    fn cjk_segment_inserts_spaces_between_ideographs() {
        use crate::storage::database::cjk_segment;
        assert_eq!(cjk_segment("整理设计风格"), "整 理 设 计 风 格");
        // English passes through unchanged
        assert_eq!(cjk_segment("hello world"), "hello world");
        // Mixed: only CJK gets spaces, English stays intact
        assert_eq!(cjk_segment("AI 设计 skill"), "AI 设 计 skill");
        // Idempotent
        assert_eq!(cjk_segment("整 理"), "整 理");
        // Empty
        assert_eq!(cjk_segment(""), "");
    }

    #[test]
    fn secret_setting_update_encrypts_sqlite_value() {
        let db = test_db();
        let repo = Repository::new(db);
        let key = "ai_api_key_test_update";

        repo.update_setting(key, "sk-test-secret").unwrap();

        let stored = repo.get_setting_from_db(key).unwrap().unwrap();
        assert!(crate::secure_store::is_encrypted_value(&stored));
        assert_ne!(stored, "sk-test-secret");
        assert_eq!(
            repo.get_setting(key).unwrap(),
            Some("sk-test-secret".to_string())
        );
    }

    #[test]
    fn secret_setting_read_migrates_plaintext_sqlite_value() {
        let db = test_db();
        let repo = Repository::new(db);
        let key = "ai_api_key_test_migration";
        repo.update_setting_db(key, "sk-legacy-secret").unwrap();

        assert_eq!(
            repo.get_setting(key).unwrap(),
            Some("sk-legacy-secret".to_string())
        );
        let stored = repo.get_setting_from_db(key).unwrap().unwrap();
        assert!(crate::secure_store::is_encrypted_value(&stored));
        assert_ne!(stored, "sk-legacy-secret");
    }

    #[test]
    fn secret_setting_placeholder_update_preserves_existing_secret() {
        let db = test_db();
        let repo = Repository::new(db);
        let key = "ai_api_key_test_placeholder";

        repo.update_setting(key, "sk-test-secret").unwrap();
        let stored_before = repo.get_setting_from_db(key).unwrap().unwrap();

        repo.update_setting(key, crate::secure_store::SECRET_SETTING_PRESENT)
            .unwrap();

        assert_eq!(
            repo.get_setting(key).unwrap(),
            Some("sk-test-secret".to_string())
        );
        assert_eq!(repo.get_setting_from_db(key).unwrap(), Some(stored_before));
    }

    #[test]
    fn get_all_settings_masks_secret_values() {
        let db = test_db();
        let repo = Repository::new(db);
        repo.update_setting_db("ai_api_key_openai", "sk-legacy-secret")
            .unwrap();
        repo.update_setting("ai_provider", "openai").unwrap();

        let settings = repo.get_all_settings().unwrap();
        assert_eq!(
            settings.get("ai_api_key_openai"),
            Some(&crate::secure_store::SECRET_SETTING_PRESENT.to_string())
        );
        assert_eq!(
            repo.get_setting_from_db("ai_api_key_openai").unwrap(),
            Some("sk-legacy-secret".to_string())
        );
        assert_eq!(settings.get("ai_provider"), Some(&"openai".to_string()));
    }

    #[test]
    fn extract_project_url_prefers_github() {
        let body = "这个项目的源码：\n\n## 项目地址\n- GitHub: https://github.com/foo/bar\n";
        let url = Repository::extract_project_url_from_body(body).unwrap();
        assert_eq!(url, "https://github.com/foo/bar");
    }

    #[test]
    fn extract_project_url_skips_social_in_favor_of_github() {
        // Even if a tweet appears earlier in the body, GitHub wins.
        let body =
            "原文 https://x.com/some/status/123\n更多介绍\n## 项目地址\nhttps://github.com/foo/bar";
        let url = Repository::extract_project_url_from_body(body).unwrap();
        assert_eq!(url, "https://github.com/foo/bar");
    }

    #[test]
    fn extract_project_url_falls_back_to_non_social_link() {
        // No GitHub but has a non-social URL → use that.
        let body = "项目主页 https://example.com/project\n推文 https://x.com/some/status/123";
        let url = Repository::extract_project_url_from_body(body).unwrap();
        assert_eq!(url, "https://example.com/project");
    }

    #[test]
    fn extract_project_url_returns_none_when_only_social_links() {
        // Only social links and no project page → None (caller falls back to source_url)
        let body =
            "看到这个推文 https://x.com/foo/status/1 还有微信文章 https://mp.weixin.qq.com/s/abc";
        assert!(Repository::extract_project_url_from_body(body).is_none());
    }

    #[test]
    fn extract_project_url_strips_trailing_punctuation() {
        let body = "看 https://github.com/foo/bar，挺好用的";
        let url = Repository::extract_project_url_from_body(body).unwrap();
        assert_eq!(url, "https://github.com/foo/bar");
    }

    #[test]
    fn extract_project_url_returns_none_for_empty_body() {
        assert!(Repository::extract_project_url_from_body("").is_none());
    }
}
