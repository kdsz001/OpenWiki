use crate::capture::content::{compute_hash, detect_url};
use crate::commands::capture::AppState;
use crate::export::markdown;
use crate::storage::models::{CapturedContent, ContentType};
use crate::storage::repository::Repository;
use chrono::{DateTime, Utc};
use std::fs;
use std::path::{Path, PathBuf};
use tauri::State;
use uuid::Uuid;

const LOCAL_RAW_SOURCE_PREFIX: &str = "local-raw:";

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

fn resolve_preview_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join("Library/Caches"))
        .join("com.openwiki.app")
        .join("previews")
}

fn is_supported_import_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| matches!(ext.to_ascii_lowercase().as_str(), "md" | "markdown" | "txt"))
        .unwrap_or(false)
}

fn collect_import_files(dir: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    for entry in fs::read_dir(dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if path.is_dir() {
            collect_import_files(&path, files)?;
        } else if is_supported_import_file(&path) {
            files.push(path);
        }
    }
    Ok(())
}

fn capture_timestamp_for_path(path: &Path) -> String {
    fs::metadata(path)
        .ok()
        .and_then(|meta| meta.modified().ok())
        .map(|modified| DateTime::<Utc>::from(modified).to_rfc3339())
        .unwrap_or_else(|| Utc::now().to_rfc3339())
}

fn build_imported_content(path: &Path) -> Result<CapturedContent, String> {
    let raw_text = fs::read_to_string(path)
        .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
    let trimmed = raw_text.trim();
    if trimmed.is_empty() {
        return Err(format!("Empty file: {}", path.display()));
    }

    let detected_url = detect_url(trimmed);
    let content_type = if detected_url.is_some() {
        ContentType::Url
    } else {
        ContentType::Text
    };
    let content_hash = compute_hash(
        detected_url
            .clone()
            .unwrap_or_else(|| trimmed.to_string())
            .as_bytes(),
    );
    let captured_at = capture_timestamp_for_path(path);
    let source_app = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Imported Note")
        .to_string();
    let now = Utc::now().to_rfc3339();

    Ok(CapturedContent {
        id: Uuid::new_v4().to_string(),
        content_type,
        raw_text: Some(trimmed.to_string()),
        image_path: None,
        thumbnail_path: None,
        source_app,
        source_bundle_id: None,
        source_url: detected_url,
        user_note: None,
        captured_at: captured_at.clone(),
        content_hash,
        byte_size: trimmed.as_bytes().len() as i64,
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
    })
}

fn import_path_list(paths: Vec<PathBuf>, repo: &Repository) -> Result<serde_json::Value, String> {
    let mut imported = 0usize;
    let mut skipped = 0usize;
    let mut failed = 0usize;
    let mut imported_ids: Vec<String> = Vec::new();

    for path in paths {
        if !path.is_file() || !is_supported_import_file(&path) {
            skipped += 1;
            continue;
        }

        let content = match build_imported_content(&path) {
            Ok(content) => content,
            Err(e) => {
                log::warn!("Import skipped for {}: {}", path.display(), e);
                failed += 1;
                continue;
            }
        };

        match repo.content_exists_by_hash(&content.content_hash) {
            Ok(true) => {
                skipped += 1;
                continue;
            }
            Ok(false) => {}
            Err(e) => {
                log::warn!("Import dedup check failed for {}: {}", path.display(), e);
                failed += 1;
                continue;
            }
        }

        if let Err(e) = repo.save_content(&content) {
            log::warn!("Import save failed for {}: {}", path.display(), e);
            failed += 1;
        } else {
            imported += 1;
            imported_ids.push(content.id);
        }
    }

    Ok(serde_json::json!({
        "imported": imported,
        "skipped": skipped,
        "failed": failed,
        "imported_ids": imported_ids,
    }))
}

fn classify_raw_bucket(root: &Path, path: &Path) -> &'static str {
    let relative = path.strip_prefix(root).unwrap_or(path);
    let parts: Vec<String> = relative
        .components()
        .filter_map(|component| component.as_os_str().to_str())
        .map(|part| part.to_string())
        .collect();

    match parts.first().map(|part| part.as_str()) {
        Some("inbox") => match parts.get(1).map(|part| part.as_str()) {
            Some("闪念笔记") => "fleeting_note",
            Some("外部素材") => "external_material",
            _ => "inbox",
        },
        Some("01_Diaries") => "diary",
        Some("02_Articles") => "article",
        Some("03_Tradingnotes") => "trading_note",
        Some("个人档案") => match parts.get(1).map(|part| part.as_str()) {
            Some("01_Diaries") => "diary",
            Some("02_Articles") => "article",
            Some("03_Tradingnotes") => "trading_note",
            _ => "personal_archive",
        },
        Some("佛教相关内容学习") => "study_note",
        Some("常用决策原则") => "decision_principle",
        _ => "material",
    }
}

fn should_skip_local_raw_path(root: &Path, path: &Path) -> bool {
    let relative = path.strip_prefix(root).unwrap_or(path);
    let path_text = relative.to_string_lossy();
    if path_text.contains("历史日记汇总") || path_text.contains("历史文章") {
        return true;
    }

    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    name.contains("季度合集")
        || name.contains("年度合集")
        || name.contains("_QN_")
        || name.contains("全集")
}

fn collect_local_raw_files(root: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    for entry in fs::read_dir(root).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if should_skip_local_raw_path(root, &path) {
            continue;
        }
        if path.is_dir() {
            collect_local_raw_files(&path, files)?;
        } else if is_supported_import_file(&path) {
            files.push(path);
        }
    }
    Ok(())
}

fn build_local_raw_content(root: &Path, path: &Path) -> Result<CapturedContent, String> {
    let mut content = build_imported_content(path)?;
    let relative = path
        .strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");
    let bucket = classify_raw_bucket(root, path);
    let source_url = format!("{}{}", LOCAL_RAW_SOURCE_PREFIX, path.to_string_lossy());

    content.source_app = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Imported RAW")
        .to_string();
    content.source_bundle_id = Some(format!("local_raw:{}", bucket));
    content.source_url = Some(source_url);
    content.user_note = Some(format!("RAW/{}", relative));
    content.raw_text = content.raw_text.map(|raw| raw.trim().to_string());

    Ok(content)
}

fn sync_local_raw_files(root: &Path, repo: &Repository) -> Result<serde_json::Value, String> {
    let mut files = Vec::new();
    collect_local_raw_files(root, &mut files)?;
    files.sort();

    let mut created = 0usize;
    let mut updated = 0usize;
    let mut skipped = 0usize;
    let mut failed = 0usize;
    let mut removed = 0usize;
    let mut imported_ids = Vec::new();
    let mut seen_sources = std::collections::HashSet::new();
    let mut bucket_counts: std::collections::HashMap<&'static str, usize> =
        std::collections::HashMap::new();

    for path in files {
        let bucket = classify_raw_bucket(root, &path);
        *bucket_counts.entry(bucket).or_insert(0) += 1;

        let content = match build_local_raw_content(root, &path) {
            Ok(content) => content,
            Err(error) => {
                log::warn!("Local RAW sync skipped for {}: {}", path.display(), error);
                failed += 1;
                continue;
            }
        };

        let Some(source_url) = content.source_url.clone() else {
            failed += 1;
            continue;
        };
        seen_sources.insert(source_url.clone());

        match repo
            .get_content_by_source_url(&source_url)
            .map_err(|e| e.to_string())?
        {
            Some(existing) => {
                if existing.content_hash == content.content_hash
                    && existing.user_note == content.user_note
                    && existing.source_bundle_id == content.source_bundle_id
                    && existing.captured_at == content.captured_at
                {
                    skipped += 1;
                    imported_ids.push(existing.id);
                    continue;
                }

                let updated_content = CapturedContent {
                    id: existing.id.clone(),
                    created_at: existing.created_at.clone(),
                    updated_at: Utc::now().to_rfc3339(),
                    ..content
                };
                repo.update_local_synced_content(&updated_content)
                    .map_err(|e| e.to_string())?;
                updated += 1;
                imported_ids.push(updated_content.id);
            }
            None => {
                repo.save_content(&content).map_err(|e| e.to_string())?;
                created += 1;
                imported_ids.push(content.id);
            }
        }
    }

    for (content_id, source_url) in repo
        .get_local_synced_contents(LOCAL_RAW_SOURCE_PREFIX)
        .map_err(|e| e.to_string())?
    {
        if !seen_sources.contains(&source_url) {
            repo.delete_content(&content_id)
                .map_err(|e| e.to_string())?;
            removed += 1;
        }
    }

    Ok(serde_json::json!({
        "root": root.to_string_lossy().to_string(),
        "files_found": seen_sources.len(),
        "created": created,
        "updated": updated,
        "skipped": skipped,
        "failed": failed,
        "removed": removed,
        "imported_ids": imported_ids,
        "counts": bucket_counts,
    }))
}

fn run_applescript(script: &str) -> Result<String, String> {
    let output = std::process::Command::new("osascript")
        .arg("-e")
        .arg(script)
        .output()
        .map_err(|e| format!("Failed to run picker: {}", e))?;

    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
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

    // Reveal the file in Finder
    let _ = std::process::Command::new("open")
        .arg("-R")
        .arg(&path)
        .spawn();

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

    // Reveal the file in Finder
    let _ = std::process::Command::new("open")
        .arg("-R")
        .arg(&path)
        .spawn();

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

    std::process::Command::new("open")
        .arg(dir.to_string_lossy().to_string())
        .spawn()
        .map_err(|e| format!("Failed to open directory: {}", e))?;

    Ok(())
}

#[tauri::command]
pub async fn open_data_folder() -> Result<(), String> {
    let data_dir = dirs::data_dir()
        .unwrap_or_default()
        .join("com.openwiki.app");

    // Use "open -R" to reveal in Finder, targeting the db file.
    // macOS treats ".app" directories as application bundles,
    // so "open com.openwiki.app/" fails. Revealing a file inside works.
    let target = data_dir.join("openwiki.db");
    let reveal_target = if target.exists() { target } else { data_dir };

    std::process::Command::new("open")
        .arg("-R")
        .arg(reveal_target.to_string_lossy().to_string())
        .spawn()
        .map_err(|e| format!("Failed to open data folder: {}", e))?;

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

#[tauri::command]
pub async fn render_selected_markdown(
    ids: Vec<String>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let repo = Repository::new(state.db.clone());
    let export_dir = resolve_export_dir(&repo);
    markdown::render_selected_markdown(&ids, &repo, &export_dir).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn export_selected_single(
    ids: Vec<String>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let repo = Repository::new(state.db.clone());
    let export_dir = resolve_export_dir(&repo);
    let (path, _count) = markdown::export_selected_single_file(&ids, &repo, &export_dir)
        .map_err(|e| e.to_string())?;

    let _ = std::process::Command::new("open")
        .arg("-R")
        .arg(&path)
        .spawn();

    Ok(path.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn open_content_file(
    content_id: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let repo = Repository::new(state.db.clone());
    let content = repo
        .get_content_by_id(&content_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Content not found".to_string())?;

    if let Some(local_source_path) = content
        .source_url
        .as_deref()
        .and_then(|source| source.strip_prefix(LOCAL_RAW_SOURCE_PREFIX))
        .map(PathBuf::from)
        .filter(|path| path.exists())
    {
        std::process::Command::new("open")
            .arg(&local_source_path)
            .spawn()
            .map_err(|e| format!("Failed to open local source: {}", e))?;
        return Ok(local_source_path.to_string_lossy().to_string());
    }

    let workspace_paths = crate::workspace::resolve_workspace_paths_from_repo(&repo);
    let workspace_note_path = crate::workspace::workspace_raw_note_path(&workspace_paths, &content);
    if workspace_note_path.exists() {
        std::process::Command::new("open")
            .arg(&workspace_note_path)
            .spawn()
            .map_err(|e| format!("Failed to open workspace note: {}", e))?;
        return Ok(workspace_note_path.to_string_lossy().to_string());
    }

    let workspace_note_path =
        crate::workspace::workspace_inbox_note_path(&workspace_paths, &content);
    if workspace_note_path.exists() {
        std::process::Command::new("open")
            .arg(&workspace_note_path)
            .spawn()
            .map_err(|e| format!("Failed to open workspace note: {}", e))?;
        return Ok(workspace_note_path.to_string_lossy().to_string());
    }

    let target_path = if let Some(path) = content
        .image_path
        .clone()
        .filter(|path| Path::new(path).exists())
        .or_else(|| {
            content
                .thumbnail_path
                .clone()
                .filter(|path| Path::new(path).exists())
        }) {
        PathBuf::from(path)
    } else {
        let preview_dir = resolve_preview_dir();
        fs::create_dir_all(&preview_dir).map_err(|e| e.to_string())?;
        let preview_path = preview_dir.join(format!("openwiki-preview-{}.md", content.id));
        let preview_markdown = markdown::generate_selection_markdown(
            "OpenWiki Preview",
            &[content.clone()],
            &preview_dir,
            false,
        )
        .map_err(|e| e.to_string())?;
        fs::write(&preview_path, preview_markdown).map_err(|e| e.to_string())?;
        preview_path
    };

    std::process::Command::new("open")
        .arg(&target_path)
        .spawn()
        .map_err(|e| format!("Failed to open content: {}", e))?;

    Ok(target_path.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn pick_import_files() -> Result<Vec<String>, String> {
    let script = r#"
try
  set chosenFiles to choose file with prompt "选择要导入的 Markdown / 文本文件" with multiple selections allowed
  if class of chosenFiles is list then
    set outputLines to {}
    repeat with oneFile in chosenFiles
      set end of outputLines to POSIX path of oneFile
    end repeat
    set AppleScript's text item delimiters to linefeed
    return outputLines as text
  else
    return POSIX path of chosenFiles
  end if
on error number -128
  return ""
end try
"#;

    let output = run_applescript(script)?;
    Ok(output
        .lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .map(|line| line.to_string())
        .collect())
}

#[tauri::command]
pub async fn pick_import_directory() -> Result<String, String> {
    let script = r#"
try
  set chosenFolder to choose folder with prompt "选择要导入的知识库文件夹"
  return POSIX path of chosenFolder
on error number -128
  return ""
end try
"#;

    run_applescript(script)
}

#[tauri::command]
pub async fn import_files(
    paths: Vec<String>,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let repo = Repository::new(state.db.clone());
    let normalized: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
    import_path_list(normalized, &repo)
}

#[tauri::command]
pub async fn import_directory(
    path: String,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    if path.trim().is_empty() {
        return Ok(serde_json::json!({
            "imported": 0,
            "skipped": 0,
            "failed": 0,
        }));
    }

    let repo = Repository::new(state.db.clone());
    let dir = PathBuf::from(path);
    if !dir.is_dir() {
        return Err("Selected path is not a folder".to_string());
    }

    let mut files = Vec::new();
    collect_import_files(&dir, &mut files)?;
    import_path_list(files, &repo)
}

#[tauri::command]
pub async fn sync_local_raw_directory(
    path: String,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    if path.trim().is_empty() {
        return Err("RAW path is empty".to_string());
    }

    let repo = Repository::new(state.db.clone());
    let dir = PathBuf::from(path);
    if !dir.is_dir() {
        return Err("Selected RAW path is not a folder".to_string());
    }

    sync_local_raw_files(&dir, &repo)
}
