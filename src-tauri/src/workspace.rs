use crate::storage::models::CapturedContent;
use crate::storage::repository::Repository;
use chrono::{DateTime, Local, Utc};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

const WORKSPACE_FOLDER_NAME: &str = "OpenWiki Workspace";

#[derive(Clone, Debug, Serialize)]
pub struct WorkspaceProtocolPaths {
    pub initialized: bool,
    pub root: String,
    pub inbox: String,
    pub raw: String,
    pub wiki_root: String,
    pub wiki_cases: String,
    pub wiki_concepts: String,
    pub wiki_themes: String,
    pub wiki_dashboards: String,
    pub wiki_drafts: String,
    pub wiki_candidates: String,
    pub insights: String,
}

fn expand_user_path(input: &str) -> PathBuf {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return default_workspace_root();
    }

    if trimmed == "~" {
        return dirs::home_dir().unwrap_or_else(|| PathBuf::from(trimmed));
    }

    if let Some(rest) = trimmed.strip_prefix("~/") {
        return dirs::home_dir().unwrap_or_default().join(rest);
    }

    PathBuf::from(trimmed)
}

pub fn default_workspace_root() -> PathBuf {
    dirs::document_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(WORKSPACE_FOLDER_NAME)
}

fn path_set(root: &Path, initialized: bool) -> WorkspaceProtocolPaths {
    let inbox = root.join("inbox");
    let raw = root.join("raw");
    let wiki_root = root.join("wiki");
    let insights = root.join("insights");

    WorkspaceProtocolPaths {
        initialized,
        root: root.to_string_lossy().to_string(),
        inbox: inbox.to_string_lossy().to_string(),
        raw: raw.to_string_lossy().to_string(),
        wiki_root: wiki_root.to_string_lossy().to_string(),
        wiki_cases: wiki_root.join("cases").to_string_lossy().to_string(),
        wiki_concepts: wiki_root.join("concepts").to_string_lossy().to_string(),
        wiki_themes: wiki_root.join("themes").to_string_lossy().to_string(),
        wiki_dashboards: wiki_root.join("dashboards").to_string_lossy().to_string(),
        wiki_drafts: wiki_root.join("_drafts").to_string_lossy().to_string(),
        wiki_candidates: wiki_root.join("_candidates").to_string_lossy().to_string(),
        insights: insights.to_string_lossy().to_string(),
    }
}

pub fn resolve_workspace_paths_from_repo(repo: &Repository) -> WorkspaceProtocolPaths {
    let root = repo
        .get_setting("workspace_root")
        .ok()
        .flatten()
        .filter(|value| !value.trim().is_empty())
        .map(|value| expand_user_path(&value))
        .unwrap_or_else(default_workspace_root);
    let initialized = root.exists();
    path_set(&root, initialized)
}

pub fn ensure_workspace_root(
    repo: &Repository,
    requested_path: Option<String>,
) -> Result<WorkspaceProtocolPaths, String> {
    let requested = requested_path
        .as_deref()
        .map(expand_user_path)
        .unwrap_or_else(default_workspace_root);

    let paths = path_set(&requested, true);
    for dir in [
        &paths.root,
        &paths.inbox,
        &paths.raw,
        &paths.wiki_root,
        &paths.wiki_cases,
        &paths.wiki_concepts,
        &paths.wiki_themes,
        &paths.wiki_dashboards,
        &paths.wiki_drafts,
        &paths.wiki_candidates,
        &paths.insights,
    ] {
        fs::create_dir_all(dir)
            .map_err(|e| format!("Failed to create workspace directory {}: {}", dir, e))?;
    }

    repo.update_setting("workspace_root", &paths.root)
        .map_err(|e| format!("Failed to save workspace root: {}", e))?;

    Ok(paths)
}

fn workspace_note_file_name(content: &CapturedContent) -> String {
    let stamp = DateTime::parse_from_rfc3339(&content.captured_at)
        .map(|dt| dt.with_timezone(&Local).format("%Y%m%d-%H%M%S").to_string())
        .unwrap_or_else(|_| {
            Utc::now()
                .with_timezone(&Local)
                .format("%Y%m%d-%H%M%S")
                .to_string()
        });
    let kind = content.content_type.as_str();
    let short_id = content.id.chars().take(8).collect::<String>();
    format!("{}-{}-{}.md", stamp, kind, short_id)
}

pub fn workspace_inbox_note_path(
    paths: &WorkspaceProtocolPaths,
    content: &CapturedContent,
) -> PathBuf {
    PathBuf::from(&paths.inbox).join(workspace_note_file_name(content))
}

pub fn workspace_raw_note_path(
    paths: &WorkspaceProtocolPaths,
    content: &CapturedContent,
) -> PathBuf {
    PathBuf::from(&paths.raw).join(workspace_note_file_name(content))
}

fn workspace_asset_path(
    paths: &WorkspaceProtocolPaths,
    content: &CapturedContent,
    source_path: &Path,
) -> PathBuf {
    let ext = source_path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("png");
    PathBuf::from(&paths.inbox)
        .join("assets")
        .join(format!("{}.{}", content.id, ext))
}

fn write_workspace_assets(
    paths: &WorkspaceProtocolPaths,
    content: &CapturedContent,
) -> Result<Option<String>, String> {
    let Some(image_path) = content.image_path.as_deref() else {
        return Ok(None);
    };
    let source_path = Path::new(image_path);
    if !source_path.exists() {
        return Ok(None);
    }

    let asset_path = workspace_asset_path(paths, content, source_path);
    if let Some(parent) = asset_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create workspace asset directory: {}", e))?;
    }
    fs::copy(source_path, &asset_path)
        .map_err(|e| format!("Failed to copy asset into workspace inbox: {}", e))?;

    Ok(asset_path
        .strip_prefix(&paths.inbox)
        .ok()
        .map(|relative| relative.to_string_lossy().replace('\\', "/")))
}

fn markdown_title(content: &CapturedContent) -> String {
    if let Some(raw) = content.raw_text.as_deref() {
        if let Some(first_line) = raw.lines().map(str::trim).find(|line| !line.is_empty()) {
            let first_line = first_line.trim_start_matches('#').trim();
            if !first_line.is_empty() {
                return first_line.chars().take(60).collect();
            }
        }
    }

    let kind = match content.content_type.as_str() {
        "url" => "Captured link",
        "image" => "Captured image",
        _ => "Captured note",
    };
    format!(
        "{} {}",
        kind,
        &content.id.chars().take(8).collect::<String>()
    )
}

fn render_workspace_markdown(
    content: &CapturedContent,
    asset_relative_path: Option<&str>,
    workspace_layer: &str,
) -> String {
    let mut out = String::new();
    out.push_str("---\n");
    out.push_str(&format!("openwiki_id: {}\n", content.id));
    out.push_str(&format!("captured_at: {}\n", content.captured_at));
    out.push_str(&format!(
        "content_type: {}\n",
        content.content_type.as_str()
    ));
    out.push_str(&format!("source_app: {}\n", content.source_app));
    out.push_str("managed_by: openwiki\n");
    out.push_str(&format!("workspace_layer: {}\n", workspace_layer));
    if let Some(source_url) = content.source_url.as_deref() {
        out.push_str(&format!("source_url: {}\n", source_url));
    }
    out.push_str("---\n\n");

    out.push_str(&format!("# {}\n\n", markdown_title(content)));
    out.push_str(&format!("- Captured at: {}\n", content.captured_at));
    out.push_str(&format!("- Source app: {}\n", content.source_app));
    out.push_str(&format!(
        "- Content type: {}\n",
        content.content_type.as_str()
    ));
    if let Some(source_url) = content.source_url.as_deref() {
        out.push_str(&format!("- Source URL: {}\n", source_url));
    }
    out.push('\n');

    if let Some(note) = content
        .user_note
        .as_deref()
        .filter(|note| !note.trim().is_empty())
    {
        out.push_str("## Note\n\n");
        out.push_str(note.trim());
        out.push_str("\n\n");
    }

    if let Some(summary) = content
        .summary
        .as_deref()
        .filter(|summary| !summary.trim().is_empty())
    {
        out.push_str("## Summary\n\n");
        out.push_str(summary.trim());
        out.push_str("\n\n");
    }

    if let Some(digest) = content
        .digest
        .as_deref()
        .filter(|digest| !digest.trim().is_empty())
    {
        out.push_str("## Digest\n\n");
        out.push_str(digest.trim());
        out.push_str("\n\n");
    }

    if let Some(tags) = content
        .tags
        .as_deref()
        .filter(|tags| !tags.trim().is_empty())
    {
        out.push_str("## Tags\n\n");
        for tag in tags.split(',').map(str::trim).filter(|tag| !tag.is_empty()) {
            out.push_str(&format!("- {}\n", tag));
        }
        out.push('\n');
    }

    if let Some(asset_relative_path) = asset_relative_path {
        out.push_str("## Asset\n\n");
        out.push_str(&format!("![Captured image]({})\n\n", asset_relative_path));
    }

    if let Some(clean_content) = content
        .clean_content
        .as_deref()
        .filter(|text| !text.trim().is_empty())
    {
        out.push_str("## Clean content\n\n");
        out.push_str(clean_content.trim());
        out.push_str("\n\n");
    }

    if let Some(raw_text) = content
        .raw_text
        .as_deref()
        .filter(|text| !text.trim().is_empty())
    {
        out.push_str("## Content\n\n");
        out.push_str(raw_text.trim());
        out.push('\n');
    }

    out
}

fn load_workspace_paths(repo: &Repository) -> Result<Option<WorkspaceProtocolPaths>, String> {
    let root_value = repo
        .get_setting("workspace_root")
        .map_err(|e| format!("Failed to load workspace root: {}", e))?;
    let Some(root_value) = root_value.filter(|value| !value.trim().is_empty()) else {
        return Ok(None);
    };

    ensure_workspace_root(repo, Some(root_value)).map(Some)
}

pub fn sync_content_to_workspace_inbox(
    repo: &Repository,
    content: &CapturedContent,
) -> Result<Option<PathBuf>, String> {
    let Some(paths) = load_workspace_paths(repo)? else {
        return Ok(None);
    };
    let asset_relative_path = write_workspace_assets(&paths, content)?;
    let note_path = workspace_inbox_note_path(&paths, content);
    let note_markdown = render_workspace_markdown(content, asset_relative_path.as_deref(), "inbox");
    fs::write(&note_path, note_markdown)
        .map_err(|e| format!("Failed to write workspace inbox note: {}", e))?;

    Ok(Some(note_path))
}

pub fn sync_content_to_workspace_raw(
    repo: &Repository,
    content: &CapturedContent,
) -> Result<Option<PathBuf>, String> {
    let Some(paths) = load_workspace_paths(repo)? else {
        return Ok(None);
    };

    let has_processed_signal = content
        .summary
        .as_deref()
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
        || content
            .digest
            .as_deref()
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false)
        || content
            .tags
            .as_deref()
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false)
        || content
            .clean_content
            .as_deref()
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false);
    if !has_processed_signal {
        return Ok(None);
    }

    let inbox_asset_relative = write_workspace_assets(&paths, content)?;
    let raw_asset_relative = inbox_asset_relative
        .as_deref()
        .map(|relative| format!("../inbox/{}", relative));
    let note_path = workspace_raw_note_path(&paths, content);
    let note_markdown = render_workspace_markdown(content, raw_asset_relative.as_deref(), "raw");
    fs::write(&note_path, note_markdown)
        .map_err(|e| format!("Failed to write workspace raw note: {}", e))?;

    Ok(Some(note_path))
}
