use crate::commands::capture::AppState;
use crate::export::markdown;
use crate::storage::models::CapturedContent;
use crate::storage::repository::Repository;
use std::path::PathBuf;
use tauri::State;

/// Open a path in the system file manager.
/// `reveal` = true selects/highlights the item; false opens the directory itself.
fn open_in_file_manager(path: &std::path::Path, reveal: bool) {
    #[cfg(target_os = "macos")]
    {
        let mut cmd = std::process::Command::new("open");
        if reveal {
            cmd.arg("-R");
        }
        let _ = cmd.arg(path).spawn();
    }
    #[cfg(target_os = "windows")]
    {
        let mut cmd = std::process::Command::new("explorer.exe");
        if reveal {
            cmd.arg("/select,");
        }
        let _ = cmd.arg(path).spawn();
    }
    #[cfg(target_os = "linux")]
    {
        // xdg-open can't reveal individual files; open the parent dir instead.
        let target = if reveal {
            path.parent().unwrap_or(path)
        } else {
            path
        };
        let _ = std::process::Command::new("xdg-open").arg(target).spawn();
    }
}

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

#[tauri::command]
pub async fn search_content(
    query: String,
    state: State<'_, AppState>,
) -> Result<Vec<CapturedContent>, String> {
    let repo = Repository::new(state.db.clone());
    repo.search_content(&query, 50).map_err(|e| e.to_string())
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

    open_in_file_manager(&path, true);

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

    open_in_file_manager(&path, true);

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

    open_in_file_manager(&dir, false);

    Ok(())
}

#[tauri::command]
pub async fn open_data_folder() -> Result<(), String> {
    let data_dir = dirs::data_dir()
        .unwrap_or_default()
        .join("com.openwiki.app");

    let target = data_dir.join("openwiki.db");
    let reveal_target = if target.exists() { target } else { data_dir };

    open_in_file_manager(&reveal_target, true);

    Ok(())
}

#[tauri::command]
pub async fn get_storage_info(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let repo = Repository::new(state.db.clone());
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
    let db_path = dirs::data_dir()
        .unwrap_or_default()
        .join("com.openwiki.app")
        .join("openwiki.db");
    let disk_bytes = std::fs::metadata(&db_path).map(|m| m.len()).unwrap_or(0);
    let disk_mb = disk_bytes as f64 / (1024.0 * 1024.0);

    Ok(serde_json::json!({
        "total_items": total_items,
        "disk_usage_mb": disk_mb,
    }))
}
