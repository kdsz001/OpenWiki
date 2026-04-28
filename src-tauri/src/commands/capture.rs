use crate::capture::content::{compute_hash, detect_url};
use crate::storage::database::Database;
use crate::storage::models::{CaptureEvent, CapturedContent, ContentType};
use chrono::Utc;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tauri::{Emitter, State};

/// The application data directory name for storing captured images.
const APP_DATA_DIR: &str = "com.openwiki.app";
const CAPTURES_SUBDIR: &str = "captures";
const THUMBNAILS_SUBDIR: &str = "thumbnails";
const THUMBNAIL_WIDTH: u32 = 200;

pub struct AppState {
    pub db: Arc<Database>,
    /// Stores the latest pending capture for the bubble window to retrieve.
    pub pending_capture: Arc<Mutex<Option<serde_json::Value>>>,
    /// Temporarily suppresses macOS Reopen from pulling the main window forward.
    pub suppress_reopen_until: Arc<Mutex<Option<Instant>>>,
}

/// Get the captures directory, creating it if necessary.
fn get_captures_dir() -> Result<PathBuf, String> {
    let base = dirs::data_dir()
        .or_else(|| dirs::home_dir().map(|h| h.join("Library").join("Application Support")))
        .ok_or_else(|| "Cannot determine application data directory".to_string())?;

    let captures_dir = base.join(APP_DATA_DIR).join(CAPTURES_SUBDIR);
    std::fs::create_dir_all(&captures_dir)
        .map_err(|e| format!("Failed to create captures directory: {}", e))?;

    Ok(captures_dir)
}

/// Get the thumbnails directory, creating it if necessary.
fn get_thumbnails_dir() -> Result<PathBuf, String> {
    let base = dirs::data_dir()
        .or_else(|| dirs::home_dir().map(|h| h.join("Library").join("Application Support")))
        .ok_or_else(|| "Cannot determine application data directory".to_string())?;

    let thumbnails_dir = base.join(APP_DATA_DIR).join(THUMBNAILS_SUBDIR);
    std::fs::create_dir_all(&thumbnails_dir)
        .map_err(|e| format!("Failed to create thumbnails directory: {}", e))?;

    Ok(thumbnails_dir)
}

/// Copy a source image to the captures directory and return the new path.
fn copy_image_to_captures(source_path: &str, id: &str) -> Result<String, String> {
    let source = Path::new(source_path);
    if !source.exists() {
        return Err(format!("Source image does not exist: {}", source_path));
    }

    let extension = source
        .extension()
        .map(|e| e.to_string_lossy().to_string())
        .unwrap_or_else(|| "png".to_string());

    let captures_dir = get_captures_dir()?;
    let dest_filename = format!("{}.{}", id, extension);
    let dest_path = captures_dir.join(&dest_filename);

    std::fs::copy(source, &dest_path)
        .map_err(|e| format!("Failed to copy image to captures: {}", e))?;

    let dest_str = dest_path.to_string_lossy().to_string();
    log::info!("Image copied to captures: {}", dest_str);
    Ok(dest_str)
}

/// Generate a thumbnail (200px wide, preserving aspect ratio) and save it.
/// Returns the thumbnail path if successful.
fn generate_thumbnail(source_path: &str, id: &str) -> Result<String, String> {
    let img = image::open(source_path)
        .map_err(|e| format!("Failed to open image for thumbnail: {}", e))?;

    let (orig_width, orig_height) = (img.width(), img.height());
    if orig_width == 0 || orig_height == 0 {
        return Err("Image has zero dimensions".to_string());
    }

    // Calculate new height preserving aspect ratio
    let new_width = THUMBNAIL_WIDTH.min(orig_width);
    let new_height = (orig_height as f64 * new_width as f64 / orig_width as f64) as u32;

    let thumbnail = img.thumbnail(new_width, new_height);

    let thumbnails_dir = get_thumbnails_dir()?;
    let thumb_filename = format!("{}_thumb.png", id);
    let thumb_path = thumbnails_dir.join(&thumb_filename);

    thumbnail
        .save(&thumb_path)
        .map_err(|e| format!("Failed to save thumbnail: {}", e))?;

    let thumb_str = thumb_path.to_string_lossy().to_string();
    log::info!(
        "Thumbnail generated: {} ({}x{} -> {}x{})",
        thumb_str,
        orig_width,
        orig_height,
        new_width,
        new_height
    );
    Ok(thumb_str)
}

/// Internal auto-save function called directly from CaptureDetector.
/// Does not require Tauri State — takes a Database reference directly.
pub fn save_content_auto(
    db: &Arc<Database>,
    event: CaptureEvent,
) -> Result<CapturedContent, String> {
    let now = Utc::now().to_rfc3339();
    let id = uuid::Uuid::new_v4().to_string();

    // Detect content type and extract URL if applicable
    let (content_type, raw_text, image_path, detected_url) = match event.content_type.as_str() {
        "image" => (ContentType::Image, None, event.image_path, None),
        "url" => {
            let url = event.raw_text.as_deref().and_then(detect_url);
            (ContentType::Url, event.raw_text.clone(), None, url)
        }
        _ => {
            if let Some(ref text) = event.raw_text {
                if let Some(url) = detect_url(text) {
                    (ContentType::Url, event.raw_text.clone(), None, Some(url))
                } else {
                    (ContentType::Text, event.raw_text.clone(), None, None)
                }
            } else {
                (ContentType::Text, None, None, None)
            }
        }
    };

    let (final_image_path, thumbnail_path) = if content_type.as_str() == "image" {
        if let Some(ref src_path) = image_path {
            let copied_path = match copy_image_to_captures(src_path, &id) {
                Ok(p) => Some(p),
                Err(e) => {
                    log::error!("Failed to copy image: {}", e);
                    image_path.clone()
                }
            };

            let thumb_source = copied_path.as_deref().unwrap_or(src_path.as_str());
            let thumb_path = match generate_thumbnail(thumb_source, &id) {
                Ok(p) => Some(p),
                Err(e) => {
                    log::error!("Failed to generate thumbnail: {}", e);
                    None
                }
            };

            (copied_path, thumb_path)
        } else {
            (None, None)
        }
    } else {
        (image_path, None)
    };

    // For hash computation, use detected_url (trimmed) for URL content to ensure consistent dedup
    let hash_data = if let Some(ref path) = final_image_path {
        let file_size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        format!("img:{}:{}", path, file_size)
    } else if let Some(ref url) = detected_url {
        url.clone()
    } else {
        raw_text.as_deref().unwrap_or("").to_string()
    };
    let content_hash = compute_hash(hash_data.as_bytes());

    let byte_size = if let Some(ref path) = final_image_path {
        std::fs::metadata(path).map(|m| m.len() as i64).unwrap_or(0)
    } else {
        raw_text.as_ref().map(|t| t.len() as i64).unwrap_or(0)
    };

    // For URL content, use the clean detected URL (trimmed) as source_url
    let source_url = detected_url.clone();

    // Check for duplicate content — if found, move it to the top by updating captured_at
    let repo = crate::storage::repository::Repository::new(db.clone());
    if let Ok(Some(existing)) = repo.find_content_by_hash(&content_hash) {
        let _ = repo.touch_captured_at(&existing.id);
        log::info!(
            "Duplicate content detected (hash={}), moved to top: {}",
            &content_hash[..16],
            existing.id
        );
        return Err("Duplicate content".to_string());
    }

    let content = CapturedContent {
        id: id.clone(),
        content_type,
        raw_text,
        image_path: final_image_path,
        thumbnail_path,
        source_app: event.source_app,
        source_bundle_id: None,
        source_url,
        user_note: None,
        captured_at: now.clone(),
        content_hash,
        byte_size,
        is_deleted: false,
        created_at: now.clone(),
        updated_at: now,
        digested_at: None,
        digest_action: None,
        summary: None,
        tags: None,
        digest: None,
        wiki_compile_hash: None,
        wiki_assessed_hash: None,
        clean_content: None,
    };

    repo.save_content(&content).map_err(|e| e.to_string())?;

    log::info!(
        "Content auto-saved: {} (type={}, size={} bytes)",
        id,
        content.content_type.as_str(),
        content.byte_size
    );

    // Trigger auto-sync: export today's markdown if enabled
    {
        let db_clone = db.clone();
        let captured_date = content.captured_at[..10].to_string(); // "YYYY-MM-DD"
        std::thread::spawn(move || {
            let repo = crate::storage::repository::Repository::new(db_clone);
            // Check if auto-sync is enabled
            let enabled = repo
                .get_setting("datahub_export_enabled")
                .ok()
                .flatten()
                .unwrap_or_default()
                == "true";
            let auto_sync = repo
                .get_setting("datahub_auto_sync")
                .ok()
                .flatten()
                .unwrap_or_else(|| "true".to_string())
                == "true";
            if enabled && auto_sync {
                let export_dir = repo
                    .get_setting("datahub_export_dir")
                    .ok()
                    .flatten()
                    .unwrap_or_else(|| {
                        dirs::document_dir()
                            .unwrap_or_else(|| std::path::PathBuf::from("~/Documents"))
                            .join("Xiaoyun")
                            .to_string_lossy()
                            .to_string()
                    });
                let export_path = std::path::Path::new(&export_dir);
                match crate::export::markdown::export_day(&captured_date, &repo, export_path) {
                    Ok(p) => log::info!("Auto-synced markdown: {}", p.display()),
                    Err(e) => log::error!("Auto-sync failed: {}", e),
                }
            }
        });
    }

    Ok(content)
}

#[tauri::command]
pub fn save_captured_content(
    state: State<'_, AppState>,
    event: CaptureEvent,
) -> Result<CapturedContent, String> {
    save_content_auto(&state.db, event)
}

/// Save content from the Spotlight window with a user note.
/// Called when user presses Enter in the Spotlight input.
///
/// Handles the race condition where the clipboard watcher may have already
/// saved the same content. In that case, we find the existing record and
/// just attach the user_note to it.
#[tauri::command]
pub fn save_spotlight_content(
    state: State<'_, AppState>,
    content_type: String,
    raw_text: Option<String>,
    image_path: Option<String>,
    source_app: String,
    user_note: String,
) -> Result<CapturedContent, String> {
    let repo = crate::storage::repository::Repository::new(state.db.clone());

    let event = CaptureEvent {
        content_type,
        preview: raw_text
            .as_deref()
            .map(|t| t.chars().take(100).collect::<String>())
            .unwrap_or_default(),
        source_app,
        raw_text,
        image_path,
    };

    // Try saving — if duplicate, find the existing record instead
    let mut content = match save_content_auto(&state.db, event) {
        Ok(c) => c,
        Err(e) if e.contains("Duplicate content") => {
            // The clipboard watcher already saved this content.
            // Recompute the hash to find it.
            find_existing_content(&state.db)
                .ok_or_else(|| "Content was deduplicated but could not be found".to_string())?
        }
        Err(e) => return Err(e),
    };

    // Attach user_note if provided
    let note = if user_note.trim().is_empty() {
        None
    } else {
        Some(user_note.trim().to_string())
    };

    if let Some(ref note_text) = note {
        repo.update_user_note(&content.id, note_text)
            .map_err(|e| format!("Failed to save user note: {}", e))?;
        content.user_note = Some(note_text.clone());
    }

    Ok(content)
}

/// Find the most recently captured content item (used as fallback when
/// spotlight save hits a duplicate from the clipboard watcher).
fn find_existing_content(db: &Arc<Database>) -> Option<CapturedContent> {
    let repo = crate::storage::repository::Repository::new(db.clone());
    // Get the most recent item — it's almost certainly the one just auto-saved
    repo.get_all_content(1, 0)
        .ok()
        .and_then(|v| v.into_iter().next())
}

/// Called by the floating bubble when user confirms saving the captured content.
/// Receives the same JSON data that was originally sent as `capture:pending`.
#[tauri::command]
pub fn confirm_capture(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    content_type: String,
    preview: String,
    source_app: String,
    raw_text: Option<String>,
    image_path: Option<String>,
    user_note: Option<String>,
) -> Result<CapturedContent, String> {
    // NOTE: Do NOT close the bubble window here.
    // The frontend shows a green checkmark animation for 1.5s before closing itself.

    let event = CaptureEvent {
        content_type,
        preview,
        source_app,
        raw_text,
        image_path,
    };
    let mut content = match save_content_auto(&state.db, event) {
        Ok(c) => c,
        Err(e) if e.contains("Duplicate content") => {
            // Content was moved to top, emit refresh event
            let _ = app.emit(
                "content:url-fetched",
                serde_json::json!({"id": "", "reorder": true}),
            );
            return Err("Moved to top".to_string());
        }
        Err(e) => return Err(e),
    };

    // Attach user note if provided
    if let Some(ref note) = user_note {
        let note = note.trim();
        if !note.is_empty() {
            let repo = crate::storage::repository::Repository::new(state.db.clone());
            if let Err(e) = repo.update_user_note(&content.id, note) {
                log::error!("Failed to save user note: {}", e);
            } else {
                content.user_note = Some(note.to_string());
                log::info!("User note saved for {}: {}", content.id, note);
            }
        }
    }

    // Auto-OCR for image content
    spawn_auto_ocr(&app, &state.db, &content);

    // Auto-fetch for URL content
    spawn_auto_url_fetch(&app, &state.db, &content);

    // AI summary for text content (images get it after OCR, URLs after fetch)
    if content.content_type.as_str() == "text" {
        if let Some(ref text) = content.raw_text {
            spawn_summary_task(
                state.db.clone(),
                app.clone(),
                content.id.clone(),
                text.clone(),
            );
        }
    }

    Ok(content)
}

/// Get multiple content items by their IDs. Used by radar detail view.
#[tauri::command]
pub fn get_contents_by_ids(
    state: State<'_, AppState>,
    ids: Vec<String>,
) -> Result<Vec<CapturedContent>, String> {
    let repo = crate::storage::repository::Repository::new(state.db.clone());
    let mut results = Vec::new();
    for id in &ids {
        match repo.get_content_by_id(id) {
            Ok(Some(content)) => results.push(content),
            Ok(None) => {} // skip missing
            Err(e) => log::warn!("Failed to get content {}: {}", id, e),
        }
    }
    Ok(results)
}

/// Called by the bubble window to retrieve the latest pending capture.
/// Returns the pending data and clears it from state.
#[tauri::command]
pub fn get_pending_capture(
    state: State<'_, AppState>,
) -> Result<Option<serde_json::Value>, String> {
    let data = state
        .pending_capture
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?
        .take();
    Ok(data)
}

/// Called by the floating bubble when countdown expires (user didn't confirm).
/// Cleans up temporary image file if one was created.
#[tauri::command]
pub fn dismiss_capture(app: tauri::AppHandle, image_path: Option<String>) -> Result<(), String> {
    // Hide bubble window from Rust side (backup)
    hide_bubble_window(&app);

    if let Some(ref path) = image_path {
        let p = std::path::Path::new(path);
        if p.exists() {
            if let Err(e) = std::fs::remove_file(p) {
                log::warn!("Failed to cleanup temp image {}: {}", path, e);
            } else {
                log::info!("Cleaned up dismissed capture image: {}", path);
            }
        }
    }
    Ok(())
}

/// Retry fetching URL content for a given content ID.
/// Called from frontend when a URL read has failed.
#[tauri::command]
pub async fn retry_url_fetch(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    content_id: String,
) -> Result<(), String> {
    let db = state.db.clone();
    let repo = crate::storage::repository::Repository::new(db.clone());

    // Find the content record and get its source_url
    let content = repo
        .get_all_content(500, 0)
        .map_err(|e| format!("DB error: {}", e))?
        .into_iter()
        .find(|c| c.id == content_id)
        .ok_or_else(|| "Content not found".to_string())?;

    let url = content
        .source_url
        .ok_or_else(|| "No source URL for this content".to_string())?;

    log::info!("Retrying URL fetch for {} (url={})", content_id, url);

    // Spawn async fetch task
    tauri::async_runtime::spawn(async move {
        let reader = crate::capture::url_reader::UrlReader::new();
        let locale = crate::locale::resolve_locale(&db);
        match reader.fetch_content(&url, &locale).await {
            Ok(result) => {
                let db_for_summary = db.clone();
                let repo = crate::storage::repository::Repository::new(db);
                if let Err(e) = repo.update_content_for_url(&content_id, &result.content, &url) {
                    log::error!("Failed to update URL content on retry: {}", e);
                } else {
                    log::info!(
                        "URL retry succeeded for {}: {} chars",
                        content_id,
                        result.content.len()
                    );
                    spawn_summary_task(
                        db_for_summary.clone(),
                        app.clone(),
                        content_id.clone(),
                        result.content.clone(),
                    );
                    spawn_clean_content_task(
                        db_for_summary,
                        app.clone(),
                        content_id.clone(),
                        result.content.clone(),
                    );
                    let _ = app.emit(
                        "content:url-fetched",
                        serde_json::json!({
                            "id": content_id,
                            "title": result.title,
                            "content_length": result.content.len(),
                        }),
                    );
                }
            }
            Err(e) => {
                log::error!("URL retry failed for {}: {}", content_id, e);
                let repo = crate::storage::repository::Repository::new(db);
                let fail_msg = format!("[读取失败] {}\n\n原始链接: {}", e, url);
                let _ = repo.update_content_for_url(&content_id, &fail_msg, &url);
                let _ = app.emit(
                    "content:url-fetched",
                    serde_json::json!({ "id": content_id, "failed": true }),
                );
            }
        }
    });

    Ok(())
}

/// Run OCR on an image content item using macOS Vision framework.
/// Saves the recognized text to raw_text and returns it.
#[tauri::command]
pub async fn ocr_image(state: State<'_, AppState>, content_id: String) -> Result<String, String> {
    let db = state.db.clone();
    let repo = crate::storage::repository::Repository::new(db.clone());

    // Find the content record
    let content = repo
        .get_all_content(500, 0)
        .map_err(|e| format!("DB error: {}", e))?
        .into_iter()
        .find(|c| c.id == content_id)
        .ok_or_else(|| "Content not found".to_string())?;

    let image_path = content
        .image_path
        .ok_or_else(|| "No image path for this content".to_string())?;

    log::info!(
        "[OCR] Starting OCR for {} (path={})",
        content_id,
        image_path
    );

    // Run OCR in a blocking thread (Swift process is synchronous)
    let path_clone = image_path.clone();
    let text =
        tokio::task::spawn_blocking(move || crate::capture::ocr::recognize_text(&path_clone))
            .await
            .map_err(|e| format!("OCR task error: {}", e))?
            .map_err(|e| format!("OCR failed: {}", e))?;

    // Save OCR text to database
    repo.update_raw_text(&content_id, &text)
        .map_err(|e| format!("Failed to save OCR text: {}", e))?;

    log::info!("[OCR] Saved {} chars for {}", text.len(), content_id);
    Ok(text)
}

/// Spawn auto-OCR for image content in the background.
fn spawn_auto_ocr(app: &tauri::AppHandle, db: &Arc<Database>, content: &CapturedContent) {
    if content.content_type.as_str() != "image" {
        return;
    }
    // Skip if already has text (OCR already done)
    if content
        .raw_text
        .as_ref()
        .map(|t| !t.is_empty())
        .unwrap_or(false)
    {
        return;
    }
    let image_path = match &content.image_path {
        Some(p) => p.clone(),
        None => return,
    };

    let content_id = content.id.clone();
    let db_clone = db.clone();
    let app_clone = app.clone();

    tauri::async_runtime::spawn(async move {
        log::info!("[OCR] Auto-OCR starting for {}", content_id);
        match tokio::task::spawn_blocking({
            let path = image_path.clone();
            move || crate::capture::ocr::recognize_text(&path)
        })
        .await
        {
            Ok(Ok(text)) => {
                let db_for_summary = db_clone.clone();
                let repo = crate::storage::repository::Repository::new(db_clone);
                if let Err(e) = repo.update_raw_text(&content_id, &text) {
                    log::error!("[OCR] Failed to save: {}", e);
                } else {
                    log::info!(
                        "[OCR] Auto-OCR done for {}: {} chars",
                        content_id,
                        text.len()
                    );
                    spawn_summary_task(
                        db_for_summary,
                        app_clone.clone(),
                        content_id.clone(),
                        text.clone(),
                    );
                    let _ = app_clone.emit(
                        "content:ocr-done",
                        serde_json::json!({
                            "id": content_id,
                            "text_length": text.len(),
                        }),
                    );
                }
            }
            Ok(Err(e)) => {
                log::info!("[OCR] No text found in {}: {}", content_id, e);
            }
            Err(e) => {
                log::error!("[OCR] Task failed for {}: {}", content_id, e);
            }
        }
    });
}

/// Spawn auto URL fetch for URL content in the background.
fn spawn_auto_url_fetch(app: &tauri::AppHandle, db: &Arc<Database>, content: &CapturedContent) {
    if content.content_type.as_str() != "url" {
        return;
    }
    let url = match &content.source_url {
        Some(u) => u.clone(),
        None => return,
    };
    // Skip if already fetched
    let needs_fetch = content
        .raw_text
        .as_ref()
        .map(|text| text.is_empty() || text.as_str() == url)
        .unwrap_or(true);
    if !needs_fetch {
        return;
    }

    let content_id = content.id.clone();
    let db_clone = db.clone();
    let app_clone = app.clone();

    log::info!("Spawning URL fetch for {} (url={})", content_id, url);
    tauri::async_runtime::spawn(async move {
        let reader = crate::capture::url_reader::UrlReader::new();
        let locale = crate::locale::resolve_locale(&db_clone);
        match reader.fetch_content(&url, &locale).await {
            Ok(result) => {
                let db_for_summary = db_clone.clone();
                let repo = crate::storage::repository::Repository::new(db_clone);
                if let Err(e) = repo.update_content_for_url(&content_id, &result.content, &url) {
                    log::error!("Failed to update URL content: {}", e);
                } else {
                    log::info!(
                        "URL fetched for {}: {} chars",
                        content_id,
                        result.content.len()
                    );
                    spawn_summary_task(
                        db_for_summary.clone(),
                        app_clone.clone(),
                        content_id.clone(),
                        result.content.clone(),
                    );
                    // Trigger AI content cleaning (independent from summary)
                    spawn_clean_content_task(
                        db_for_summary,
                        app_clone.clone(),
                        content_id.clone(),
                        result.content.clone(),
                    );
                    let _ = app_clone.emit(
                        "content:url-fetched",
                        serde_json::json!({
                            "id": content_id,
                            "title": result.title,
                            "content_length": result.content.len(),
                        }),
                    );
                }
            }
            Err(e) => {
                log::error!("URL fetch failed for {}: {}", content_id, e);
                let repo = crate::storage::repository::Repository::new(db_clone);
                let fail_msg = format!("[读取失败] {}\n\n原始链接: {}", e, url);
                let _ = repo.update_content_for_url(&content_id, &fail_msg, &url);
                let _ = app_clone.emit(
                    "content:url-fetched",
                    serde_json::json!({ "id": content_id, "failed": true }),
                );
            }
        }
    });
}

/// Spawn an async task to generate an AI summary for a content item.
/// Silently skips if no API key configured or text too short.
pub fn spawn_summary_task(
    db: Arc<Database>,
    app: tauri::AppHandle,
    content_id: String,
    text: String,
) {
    // At least 50 characters to be worth summarizing — very short text
    // causes AI to summarize the prompt itself instead of the content
    if text.trim().len() < 50 {
        log::info!("[SUMMARY] skip {} — text too short ({} chars)", content_id, text.trim().len());
        return;
    }
    log::info!("[SUMMARY] spawn for {} ({} chars)", content_id, text.len());
    tauri::async_runtime::spawn(async move {
        log::info!("[SUMMARY] task started for {}", content_id);
        let repo = crate::storage::repository::Repository::new(db.clone());

        // Helper: trigger wiki auto-compile after summary is saved
        let maybe_wiki_compile = |db_ref: std::sync::Arc<crate::storage::database::Database>, cid: String| {
            let wiki_auto = crate::storage::repository::Repository::new(db_ref.clone())
                .get_setting("wiki_auto_compile")
                .ok()
                .flatten()
                .unwrap_or_else(|| "true".to_string())
                == "true";
            if wiki_auto {
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    if let Err(e) = crate::ai::wiki_engine::auto_compile(db_ref, &cid).await {
                        log::warn!("Wiki auto-compile failed for {}: {}", cid, e);
                    }
                });
            }
        };

        let provider_str = repo
            .get_setting("ai_provider")
            .ok()
            .flatten()
            .unwrap_or_else(|| "anthropic".to_string());

        // Load per-provider API key, fall back to legacy key
        let provider_key = format!("ai_api_key_{}", provider_str);
        let api_key = repo
            .get_setting(&provider_key)
            .ok()
            .flatten()
            .or_else(|| repo.get_setting("ai_api_key").ok().flatten())
            .unwrap_or_default();

        let model = repo
            .get_setting("ai_model")
            .ok()
            .flatten()
            .unwrap_or_else(|| "claude-sonnet-4-6".to_string());

        // Resolve locale for summary language
        let locale = crate::locale::resolve_locale(&db);

        // Send full content to AI (up to 5000 chars, covers most articles)
        let content_for_ai: String = text.chars().take(5000).collect();
        let prompt = if crate::locale::is_english(&locale) {
            format!(
                "Read the following content and return JSON with three fields:\n\
                 1. \"tags\": 2-3 specific tags. Each tag MUST contain concrete nouns from the text (names of people, companies, products, methods, technical terms, etc.).\n\
                    Format: \"Concrete noun + core point\", so the reader instantly knows what the content is about.\n\
                    Good tags: \"Musk first-principles rockets\", \"Stripe developer experience flywheel\", \"RAG retrieval-augmented generation\", \"Bridgewater all-weather hedge\"\n\
                    Bad tags: \"startup mindset\", \"product design\", \"AI apps\", \"investment methods\" (no concrete nouns, too generic)\n\
                    Each tag 2-6 words in English (keep proper nouns in original form)\n\
                 2. \"summary\": Plain-English explanation of what this content is about (English, under 40 words).\n\
                    Like a one-line pitch a friend would send when sharing an article, so the reader knows whether to click.\n\
                    Avoid formal or academic language — write like you're talking to a friend.\n\
                 3. \"digest\": Core takeaways from the content (English, 80-120 words).\n\
                    Like a smart friend telling you the key points after reading it for you.\n\
                    Structure it: core point first, then key evidence or examples, then conclusion.\n\
                    Don't use phrases like \"the article\" or \"the author\" — speak about the content directly.\n\
                 Regardless of the source language, write tags/summary/digest in English (keep proper nouns in original form). Return JSON only.\n\
                 Example: {{\"tags\":[\"Dalio all-weather hedge\",\"Shannon rebalancing arbitrage\"],\"summary\":\"How to buy the dip when markets crash — the key is keeping enough cash on hand\",\"digest\":\"The core tension in investing is wanting high returns without losing money. Dalio's all-weather strategy uses four buckets (stocks, long bonds, commodities, inflation-protected bonds) to diversify risk, surviving any economic regime. Key data: max drawdown over 30 years was only 3.9%, vs over 50% for pure stock portfolios. But the strategy sacrifices upside, averaging 9% annually. Works well for people who don't want to stress and accept moderate returns.\"}}\n\n{}",
                content_for_ai
            )
        } else {
            format!(
                "通读以下全文，返回JSON格式，包含三个字段：\n\
                 1. \"tags\": 2-3个具体标签，必须包含文中的具体名词（人名、公司名、产品名、方法名、术语等）。\n\
                    标签格式：\"具体名词+核心观点\"，让人一看就知道这篇讲了什么。\n\
                    好的标签：\"Musk第一性原理造火箭\"、\"Stripe的开发者体验飞轮\"、\"RAG检索增强生成\"、\"桥水全天候策略对冲\"\n\
                    差的标签：\"创业思维\"、\"产品设计\"、\"AI应用\"、\"投资方法\"（没有具体名词，太泛）\n\
                    每个标签4-12个字，用中文简体（专有名词保留原文）\n\
                 2. \"summary\": 用大白话说这篇内容讲了什么（中文简体，不超过80字）。\n\
                    像朋友转发文章时附的一句话，让人一看就知道要不要点开。\n\
                    不要用书面语、不要用\"探讨\"\"阐述\"\"倡导\"这类词，就正常说话。\n\
                 3. \"digest\": 这篇内容的核心要点总结（中文简体，150-200字）。\n\
                    像一个聪明的朋友帮你读完后告诉你重点。\n\
                    要有结构感：先说核心观点，再说关键论据或例子，最后说结论。\n\
                    不要用\"本文\"\"作者\"这种书面词，直接说内容本身。\n\
                 无论原文是什么语言，都必须用中文简体（专有名词保留原文）。只返回JSON。\n\
                 示例：{{\"tags\":[\"Dalio全天候策略对冲\",\"Shannon再平衡套利\"],\"summary\":\"教你怎么在股市暴跌时抄底，关键是平时得留够现金\",\"digest\":\"投资的核心矛盾是想要高收益又怕亏钱。Dalio的全天候策略用四个桶（股票、长期债、商品、通胀保护债）来分散风险，不管经济好坏都能活着。关键数据：过去30年回撤最大只有3.9%，而纯股票组合最大回撤超过50%。但这个策略牺牲了上涨空间，年化只有9%左右。适合不想操心、愿意接受中等回报的人。\"}}\n\n{}",
                content_for_ai
            )
        };

        // Try Codex OAuth first if provider is openai
        if provider_str == "openai" {
            if let Some(result) = crate::ai::attention_analyzer::try_codex_call(
                db.clone(),
                "You are an AI assistant that analyzes content and returns JSON.",
                &prompt,
                0.5,
                false, // summary, not deep analysis
            )
            .await
            {
                match result {
                    Ok(raw) => {
                        log::info!("Codex OAuth summary generated for {}", content_id);
                        let (summary, tags, digest) = extract_summary_tags_digest(&raw);
                        if !summary.is_empty() {
                            let tags_str = tags.join(",");
                            let _ = repo.update_summary_and_tags(
                                &content_id,
                                &summary,
                                &tags_str,
                                &digest,
                            );
                            let _ = app.emit("content-summary-ready", &content_id);
                            maybe_wiki_compile(db.clone(), content_id.clone());
                            log::info!(
                                "Summary generated for {}: [{}] {}",
                                content_id,
                                tags_str,
                                summary
                            );
                        }
                        return;
                    }
                    Err(e) => {
                        log::warn!("Codex OAuth failed, falling back to API Key: {}", e);
                        // Fall through to API key path below
                    }
                }
            }
        }

        // Try Gemini OAuth if provider is google
        if provider_str == "google" {
            if let Some(result) = crate::ai::attention_analyzer::try_gemini_call(
                db.clone(), "", &prompt, 0.5, false, // summary, not deep analysis
            ).await {
                match result {
                    Ok(raw) => {
                        log::info!("Gemini OAuth summary generated for {}", content_id);
                        let (summary, tags, digest) = extract_summary_tags_digest(&raw);
                        if !summary.is_empty() {
                            let tags_str = tags.join(",");
                            let _ = repo.update_summary_and_tags(&content_id, &summary, &tags_str, &digest);
                            let _ = app.emit("content-summary-ready", &content_id);
                            maybe_wiki_compile(db.clone(), content_id.clone());
                            log::info!("Summary generated for {}: [{}] {}", content_id, tags_str, summary);
                        }
                        return;
                    }
                    Err(e) => {
                        log::warn!("Gemini OAuth failed, falling back to API Key: {}", e);
                    }
                }
            }
        }

        // API key path — skip if no key configured (except for local/custom providers)
        let is_local_or_custom = provider_str == "custom" || provider_str == "ollama" || provider_str == "lmstudio";
        if api_key.is_empty() && !is_local_or_custom {
            log::warn!("[SUMMARY] {} — no API key and not local provider (provider={})", content_id, provider_str);
            return;
        }
        let base_url = repo.get_setting("ai_custom_base_url").ok().flatten().unwrap_or_default();

        log::info!(
            "[SUMMARY] {} — calling provider={} model={} base_url='{}' prompt_len={}",
            content_id, provider_str, model, base_url, prompt.len()
        );

        let provider = crate::ai::attention_analyzer::AnalysisProvider::from_str_with_base(
            &provider_str, &base_url,
        );
        match crate::ai::attention_analyzer::call_analysis_api(
            &provider, &api_key, &model, "", &prompt, 1024, true,
        )
        .await
        {
            Ok(raw) => {
                log::info!("[SUMMARY] {} — response received ({} chars): {}", content_id, raw.len(), raw.chars().take(200).collect::<String>());
                let (summary, tags, digest) = extract_summary_tags_digest(&raw);
                if !summary.is_empty() {
                    let tags_str = tags.join(",");
                    let _ = repo.update_summary_and_tags(&content_id, &summary, &tags_str, &digest);
                    let _ = app.emit("content-summary-ready", &content_id);
                    maybe_wiki_compile(db.clone(), content_id.clone());
                    log::info!(
                        "Summary generated for {}: [{}] {}",
                        content_id,
                        tags_str,
                        summary
                    );
                } else {
                    log::warn!("[SUMMARY] {} — empty summary extracted from raw response", content_id);
                }
            }
            Err(e) => {
                log::warn!("Summary generation failed for {}: {}", content_id, e);
            }
        }
    });
}

/// Extract summary, tags, and digest from AI response.
/// Expected format: {"tags":["标签1","标签2"],"summary":"一句话","digest":"段落总结"}
fn extract_summary_tags_digest(raw: &str) -> (String, Vec<String>, String) {
    let trimmed = raw.trim();
    // Strip markdown code block wrappers (```json ... ``` or ``` ... ```)
    let cleaned = if trimmed.starts_with("```") {
        let without_prefix = if let Some(rest) = trimmed.strip_prefix("```json") {
            rest
        } else {
            &trimmed[3..]
        };
        without_prefix
            .strip_suffix("```")
            .unwrap_or(without_prefix)
            .trim()
    } else {
        trimmed
    };
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(cleaned) {
        let summary = v
            .get("summary")
            .and_then(|v| v.as_str())
            .or_else(|| v.get("text").and_then(|v| v.as_str()))
            .unwrap_or("")
            .trim()
            .to_string();

        let tags = v
            .get("tags")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.trim().to_string()))
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let digest = v
            .get("digest")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();

        if !summary.is_empty() {
            return (summary, tags, digest);
        }
    }
    // Not JSON — treat as plain text summary
    let stripped = trimmed
        .trim_matches('"')
        .trim_matches('「')
        .trim_matches('」');
    (stripped.trim().to_string(), vec![], String::new())
}

/// Spawn an async task to clean URL article content via AI.
/// Only called for URL content AFTER the article body has been fetched.
/// Completely independent from the summary task.
pub fn spawn_clean_content_task(
    db: Arc<crate::storage::database::Database>,
    app: tauri::AppHandle,
    content_id: String,
    raw_text: String,
) {
    use tauri::Emitter;
    // Need substantial text to clean (bare URLs are too short)
    if raw_text.trim().len() < 200 {
        return;
    }

    // Check if feature is enabled
    let repo_check = crate::storage::repository::Repository::new(db.clone());
    let enabled = repo_check
        .get_setting("clean_content_enabled")
        .ok()
        .flatten()
        .unwrap_or_else(|| "true".to_string())
        == "true";
    if !enabled {
        return;
    }

    tauri::async_runtime::spawn(async move {
        let repo = crate::storage::repository::Repository::new(db.clone());

        let locale = crate::locale::resolve_locale(&db);

        let provider_str = repo
            .get_setting("ai_provider")
            .ok()
            .flatten()
            .unwrap_or_else(|| "anthropic".to_string());

        let provider_key = format!("ai_api_key_{}", provider_str);
        let api_key = repo
            .get_setting(&provider_key)
            .ok()
            .flatten()
            .or_else(|| repo.get_setting("ai_api_key").ok().flatten())
            .unwrap_or_default();

        let is_local_or_custom = provider_str == "custom" || provider_str == "ollama" || provider_str == "lmstudio";
        if api_key.is_empty() && !is_local_or_custom {
            // Try OAuth paths
            if provider_str == "openai" {
                if let Some(result) = crate::ai::attention_analyzer::try_codex_call(
                    db.clone(),
                    "You extract article body from noisy webpage text. Output clean Markdown only.",
                    &build_clean_prompt(&raw_text, &locale),
                    0.3,
                    false,
                )
                .await
                {
                    if let Ok(cleaned) = result {
                        save_clean_content(db.clone(), &repo, &app, &content_id, &cleaned);
                    }
                }
                return;
            }
            if provider_str == "google" {
                if let Some(result) = crate::ai::attention_analyzer::try_gemini_call(
                    db.clone(),
                    "You extract article body from noisy webpage text. Output clean Markdown only.",
                    &build_clean_prompt(&raw_text, &locale),
                    0.3,
                    false,
                )
                .await
                {
                    if let Ok(cleaned) = result {
                        save_clean_content(db.clone(), &repo, &app, &content_id, &cleaned);
                    }
                }
                return;
            }
            return;
        }

        let model = repo
            .get_setting("ai_model")
            .ok()
            .flatten()
            .unwrap_or_else(|| "claude-sonnet-4-6".to_string());
        let base_url = repo.get_setting("ai_custom_base_url").ok().flatten().unwrap_or_default();

        let provider = crate::ai::attention_analyzer::AnalysisProvider::from_str_with_base(
            &provider_str, &base_url,
        );
        match crate::ai::attention_analyzer::call_analysis_api(
            &provider,
            &api_key,
            &model,
            "You extract article body from noisy webpage text. Output clean Markdown only.",
            &build_clean_prompt(&raw_text, &locale),
            4096,
            false,
        )
        .await
        {
            Ok(cleaned) => {
                save_clean_content(db.clone(), &repo, &app, &content_id, &cleaned);
            }
            Err(e) => {
                log::warn!("Clean content generation failed for {}: {}", content_id, e);
            }
        }
    });
}

fn build_clean_prompt(raw_text: &str, locale: &str) -> String {
    let content_for_ai: String = raw_text.chars().take(15000).collect();
    if crate::locale::is_english(locale) {
        format!(
            "The following is text scraped from a webpage, which contains navigation, menus, footers, and other irrelevant content.\n\
             Please extract the article body and output it as clean Markdown.\n\n\
             Requirements:\n\
             - Keep only the article body content (headings, paragraphs, lists, quotes, etc.)\n\
             - Remove navigation menus, headers, footers, cookie notices, ads, recommended links\n\
             - Preserve all languages in the content (don't translate)\n\
             - Format with Markdown: # headings, paragraph breaks, lists, > quotes\n\
             - Preserve the full article, don't abbreviate or summarize\n\
             - Output Markdown only, no explanations\n\n\
             Webpage text:\n{}",
            content_for_ai
        )
    } else {
        format!(
            "以下是从网页抓取的文本，包含导航栏、菜单、页脚等无关内容。\n\
             请提取文章正文，输出干净的 Markdown 格式。\n\n\
             要求：\n\
             - 只保留文章正文内容（标题、段落、列表、引用等）\n\
             - 删除导航菜单、页头页脚、Cookie提示、广告、推荐链接等\n\
             - 保留文章中所有语言（不要翻译，如果有中英双语就保留双语）\n\
             - 用 Markdown 格式组织：# 标题、段落分隔、列表、> 引用等\n\
             - 保留文章全文，不要缩写或总结\n\
             - 只输出 Markdown 正文，不要加任何解释\n\n\
             网页文本：\n{}",
            content_for_ai
        )
    }
}

fn save_clean_content(
    db: std::sync::Arc<crate::storage::database::Database>,
    repo: &crate::storage::repository::Repository,
    app: &tauri::AppHandle,
    content_id: &str,
    cleaned: &str,
) {
    use tauri::Emitter;
    // Strip markdown code block wrappers that AI sometimes adds
    let trimmed = cleaned.trim();
    let stripped = if trimmed.starts_with("```") {
        let without_prefix = if let Some(rest) = trimmed.strip_prefix("```markdown") {
            rest
        } else if let Some(rest) = trimmed.strip_prefix("```md") {
            rest
        } else {
            &trimmed[3..]
        };
        without_prefix
            .strip_suffix("```")
            .unwrap_or(without_prefix)
            .trim()
    } else {
        trimmed
    };
    if stripped.len() < 50 {
        log::warn!("Clean content too short for {}, skipping", content_id);
        return;
    }
    match repo.update_clean_content(content_id, stripped) {
        Ok(()) => {
            let _ = app.emit("content:clean-ready", content_id);
            log::info!("Clean content saved for {} ({} chars)", content_id, stripped.len());
            // Trigger wiki recompile with the now-clean content
            let cid = content_id.to_string();
            let db_ref = db;
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                if let Err(e) = crate::ai::wiki_engine::auto_compile(db_ref, &cid).await {
                    log::warn!("Wiki recompile after clean failed for {}: {}", cid, e);
                }
            });
        }
        Err(e) => {
            log::warn!("Failed to save clean content for {}: {}", content_id, e);
        }
    }
}

/// Close (destroy) the bubble window completely.
fn hide_bubble_window(app: &tauri::AppHandle) {
    use tauri::Manager;
    if let Some(win) = app.get_webview_window("bubble") {
        let _ = win.close();
        log::info!("Bubble window closed/destroyed");
    }
}

/// Debug logging command — writes to a local file so we can see what happens at runtime.
#[tauri::command]
pub fn debug_log(message: String) {
    let path = std::env::temp_dir().join("openwiki_debug.log");
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let now = chrono::Local::now().format("%H:%M:%S%.3f");
        let _ = writeln!(f, "[{}] {}", now, message);
    }
    log::info!("[BUBBLE_DEBUG] {}", message);
}

/// Test AI API connection with the given provider, model, and key.
/// Returns Ok(model_response) on success, Err(error_message) on failure.
#[tauri::command]
pub async fn test_ai_connection(
    provider: String,
    model: String,
    api_key: String,
    base_url: Option<String>,
) -> Result<String, String> {
    let base = base_url.unwrap_or_default();
    let p = crate::ai::attention_analyzer::AnalysisProvider::from_str_with_base(&provider, &base);
    crate::ai::attention_analyzer::call_analysis_api(
        &p,
        &api_key,
        &model,
        "",
        "Reply with this exact json object and nothing else: {\"status\":\"ok\"}",
        64,
        false,
    )
    .await
}
