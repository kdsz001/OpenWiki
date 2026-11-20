use crate::commands::capture::AppState;
use crate::export::{markdown, report};
use crate::storage::models::CapturedContent;
use crate::storage::repository::Repository;
use std::path::{Path, PathBuf};
use tauri::State;

/// Get the default export directory (~/Downloads/OpenWiki导出/).
fn default_export_dir() -> PathBuf {
    dirs::download_dir().unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join("Downloads"))
}

/// Resolve the export directory from settings or use the default.
fn resolve_export_dir(repo: &Repository) -> PathBuf {
    match repo.get_setting("export_dir") {
        Ok(Some(dir)) if !dir.is_empty() => PathBuf::from(dir),
        _ => default_export_dir(),
    }
}

fn open_path(path: &Path, label: &str) -> Result<(), String> {
    open::that_detached(path).map_err(|e| format!("Failed to open {}: {}", label, e))
}

#[cfg(target_os = "macos")]
pub(crate) fn reveal_path(path: &Path, label: &str) -> Result<(), String> {
    std::process::Command::new("open")
        .arg("-R")
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Failed to reveal {}: {}", label, e))
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn reveal_path(path: &Path, label: &str) -> Result<(), String> {
    let target = if path.is_file() {
        path.parent().unwrap_or(path)
    } else {
        path
    };
    open_path(target, label)
}

#[tauri::command]
pub async fn search_content(
    query: String,
    hide_sensitive: Option<bool>,
    state: State<'_, AppState>,
) -> Result<Vec<CapturedContent>, String> {
    let repo = Repository::new(state.db.clone());
    repo.search_content(&query, 50, hide_sensitive.unwrap_or(false))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_dates_with_content(
    state: State<'_, AppState>,
) -> Result<Vec<(String, i64)>, String> {
    let repo = Repository::new(state.db.clone());
    repo.get_dates_with_content().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_content_for_date(
    date: String,
    state: State<'_, AppState>,
) -> Result<Vec<CapturedContent>, String> {
    let repo = Repository::new(state.db.clone());
    repo.get_content_for_date(&date).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn export_day_markdown(
    date: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let repo = Repository::new(state.db.clone());
    let export_dir = resolve_export_dir(&repo);

    let path = markdown::export_day(&date, &repo, &export_dir).map_err(|e| e.to_string())?;

    Ok(path.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn export_all_markdown(state: State<'_, AppState>) -> Result<usize, String> {
    let repo = Repository::new(state.db.clone());
    let export_dir = resolve_export_dir(&repo);

    markdown::export_all(&repo, &export_dir).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn export_date_range_markdown(
    start: String,
    end: String,
    state: State<'_, AppState>,
) -> Result<usize, String> {
    let repo = Repository::new(state.db.clone());
    let export_dir = resolve_export_dir(&repo);

    markdown::export_date_range(&start, &end, &repo, &export_dir).map_err(|e| e.to_string())
}

/// Export all content into a single markdown file.
/// Returns the file path so frontend can reveal it in Finder.
#[tauri::command]
pub async fn export_all_single(state: State<'_, AppState>) -> Result<String, String> {
    let repo = Repository::new(state.db.clone());
    let export_dir = resolve_export_dir(&repo);

    let (path, _count) =
        markdown::export_all_single_file(&repo, &export_dir).map_err(|e| e.to_string())?;

    let _ = reveal_path(&path, "exported file");

    Ok(path.to_string_lossy().to_string())
}

/// Same as export_all_single but does NOT pop Finder.
/// Used when the user just wants the file path on the clipboard
/// (e.g. to paste into an AI tool like Claude Desktop / Code that
/// can read local files). Popping Finder here would steal focus
/// and feel like an unwanted side effect.
#[tauri::command]
pub async fn export_all_single_quiet(state: State<'_, AppState>) -> Result<String, String> {
    let repo = Repository::new(state.db.clone());
    let export_dir = resolve_export_dir(&repo);

    let (path, _count) =
        markdown::export_all_single_file(&repo, &export_dir).map_err(|e| e.to_string())?;

    Ok(path.to_string_lossy().to_string())
}

/// Export a date range into a single markdown file.
/// Returns the file path so frontend can reveal it in Finder.
#[tauri::command]
pub async fn export_range_single(
    start: String,
    end: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let repo = Repository::new(state.db.clone());
    let export_dir = resolve_export_dir(&repo);

    let (path, _count) = markdown::export_range_single_file(&start, &end, &repo, &export_dir)
        .map_err(|e| e.to_string())?;

    let _ = reveal_path(&path, "exported file");

    Ok(path.to_string_lossy().to_string())
}

/// Export the latest completed radar insight report into a single Markdown file.
/// Returns the file path so frontend can reveal it in Finder.
#[tauri::command]
pub async fn export_current_radar_report(state: State<'_, AppState>) -> Result<String, String> {
    let repo = Repository::new(state.db.clone());
    let export_dir = resolve_export_dir(&repo);

    let path =
        report::export_current_radar_report(&repo, &export_dir).map_err(|e| e.to_string())?;

    let _ = reveal_path(&path, "exported report");

    Ok(path.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn get_export_dir(state: State<'_, AppState>) -> Result<String, String> {
    let repo = Repository::new(state.db.clone());
    let dir = resolve_export_dir(&repo);
    Ok(dir.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn set_export_dir(path: String, state: State<'_, AppState>) -> Result<(), String> {
    let repo = Repository::new(state.db.clone());
    repo.update_setting("export_dir", &path)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn open_export_dir(state: State<'_, AppState>) -> Result<(), String> {
    let repo = Repository::new(state.db.clone());
    let dir = resolve_export_dir(&repo);

    // Ensure directory exists before opening
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    open_path(&dir, "export directory")?;

    Ok(())
}

#[tauri::command]
pub async fn open_data_folder() -> Result<(), String> {
    let data_dir = crate::storage::data_location::data_dir()?;
    std::fs::create_dir_all(&data_dir).map_err(|e| e.to_string())?;

    // On macOS reveal the db file in Finder, targeting a file inside the
    // directory because ".app" suffixes can be treated as bundles.
    // macOS treats ".app" directories as application bundles,
    // so "open com.openwiki.app/" fails. Revealing a file inside works.
    let target = data_dir.join("openwiki.db");
    let reveal_target = if target.exists() { target } else { data_dir };

    reveal_path(&reveal_target, "data folder")?;

    Ok(())
}

#[tauri::command]
pub async fn get_storage_info(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let conn = state.db.conn.lock().map_err(|e| e.to_string())?;

    // Count non-deleted items
    let total_items: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM captured_content WHERE is_deleted = 0",
            [],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;

    // Get database file size
    let db_path = crate::storage::data_location::data_dir()?
        .join(crate::storage::data_location::DB_FILE);
    let disk_bytes = std::fs::metadata(&db_path).map(|m| m.len()).unwrap_or(0);
    let disk_mb = disk_bytes as f64 / (1024.0 * 1024.0);

    Ok(serde_json::json!({
        "total_items": total_items,
        "disk_usage_mb": disk_mb,
    }))
}
