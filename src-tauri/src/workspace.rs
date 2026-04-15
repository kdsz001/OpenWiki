use crate::storage::models::{CapturedContent, ConceptCandidate, WikiPage};
use crate::storage::repository::Repository;
use chrono::{DateTime, Local, Utc};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

const WORKSPACE_FOLDER_NAME: &str = "OpenWiki Workspace";
const LOCAL_WIKI_SOURCE_PREFIX: &str = "local-wiki:";

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

fn sanitize_workspace_segment(input: &str) -> String {
    let mut out = String::new();
    let mut last_was_dash = false;
    for ch in input.chars() {
        let keep = ch.is_alphanumeric()
            || ch == '-'
            || ch == '_'
            || ('\u{4E00}'..='\u{9FFF}').contains(&ch);
        if keep {
            out.push(ch);
            last_was_dash = false;
        } else if !last_was_dash {
            out.push('-');
            last_was_dash = true;
        }
    }

    let cleaned = out.trim_matches('-').trim().to_string();
    if cleaned.is_empty() {
        "untitled".to_string()
    } else {
        cleaned
    }
}

fn parse_page_tags(raw: Option<&str>) -> Vec<String> {
    let Some(raw) = raw.map(str::trim).filter(|raw| !raw.is_empty()) else {
        return Vec::new();
    };

    if raw.starts_with('[') {
        if let Ok(parsed) = serde_json::from_str::<Vec<String>>(raw) {
            return parsed
                .into_iter()
                .map(|tag| tag.trim().to_string())
                .filter(|tag| !tag.is_empty())
                .collect();
        }
    }

    raw.split([',', '\n', ';', '|'])
        .map(str::trim)
        .filter(|tag| !tag.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn workspace_draft_subdir(page_type: &str) -> &'static str {
    match page_type {
        "case" => "cases",
        "concept" => "concepts",
        "theme" => "themes",
        "dashboard" => "dashboards",
        "qa" => "qa",
        "source" => "sources",
        _ => "misc",
    }
}

fn workspace_wiki_draft_note_path(paths: &WorkspaceProtocolPaths, page: &WikiPage) -> PathBuf {
    PathBuf::from(&paths.wiki_drafts)
        .join(workspace_draft_subdir(&page.page_type))
        .join(format!("{}.md", sanitize_workspace_segment(&page.slug)))
}

fn workspace_candidate_concepts_dir(paths: &WorkspaceProtocolPaths) -> PathBuf {
    PathBuf::from(&paths.wiki_candidates).join("concepts")
}

fn workspace_candidate_file_prefix(content: &CapturedContent) -> String {
    format!("{}--", content.id)
}

fn workspace_candidate_note_path(
    paths: &WorkspaceProtocolPaths,
    content: &CapturedContent,
    candidate: &ConceptCandidate,
) -> PathBuf {
    workspace_candidate_concepts_dir(paths).join(format!(
        "{}{}.md",
        workspace_candidate_file_prefix(content),
        sanitize_workspace_segment(&candidate.normalized_name)
    ))
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
            .map_err(|e| format!("Failed to create asset directory: {}", e))?;
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

fn render_workspace_inbox_markdown(
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

fn render_workspace_wiki_draft_markdown(repo: &Repository, page: &WikiPage) -> String {
    let mut out = String::new();
    let page_tags = parse_page_tags(page.tags.as_deref());
    let (active_sources, total_sources) = repo.count_active_sources(&page.id).unwrap_or((0, 0));

    out.push_str("---\n");
    out.push_str(&format!("openwiki_page_id: {}\n", page.id));
    out.push_str(&format!("page_type: {}\n", page.page_type));
    out.push_str(&format!("slug: {}\n", page.slug));
    out.push_str("managed_by: openwiki\n");
    out.push_str("workspace_layer: wiki_draft\n");
    out.push_str(&format!("status: {}\n", page.status));
    out.push_str(&format!("confidence: {:.2}\n", page.confidence));
    out.push_str(&format!("created_at: {}\n", page.created_at));
    out.push_str(&format!("updated_at: {}\n", page.updated_at));
    if let Some(last_compiled_at) = page.last_compiled_at.as_deref() {
        out.push_str(&format!("last_compiled_at: {}\n", last_compiled_at));
    }
    if let Some(source_message_id) = page.source_message_id.as_deref() {
        out.push_str(&format!("source_message_id: {}\n", source_message_id));
    }
    out.push_str("---\n\n");

    out.push_str(&format!("# {}\n\n", page.title));
    out.push_str(&format!("- Page type: {}\n", page.page_type));
    out.push_str(&format!("- Active sources: {}\n", active_sources));
    out.push_str(&format!("- Total linked sources: {}\n", total_sources));
    out.push_str(&format!("- Confidence: {:.2}\n", page.confidence));
    out.push('\n');

    if let Some(summary) = page
        .summary
        .as_deref()
        .filter(|summary| !summary.trim().is_empty())
    {
        out.push_str("## Summary\n\n");
        out.push_str(summary.trim());
        out.push_str("\n\n");
    }

    if !page_tags.is_empty() {
        out.push_str("## Tags\n\n");
        for tag in page_tags {
            out.push_str(&format!("- {}\n", tag));
        }
        out.push('\n');
    }

    out.push_str("## Body\n\n");
    out.push_str(page.body_markdown.trim());
    out.push('\n');

    out
}

fn render_workspace_candidate_markdown(
    content: &CapturedContent,
    candidate: &ConceptCandidate,
    source_count: i64,
    day_count: i64,
    avg_importance: f64,
) -> String {
    let mut out = String::new();
    out.push_str("---\n");
    out.push_str(&format!("openwiki_content_id: {}\n", content.id));
    out.push_str(&format!("candidate_name: {}\n", candidate.name));
    out.push_str(&format!("normalized_name: {}\n", candidate.normalized_name));
    out.push_str(&format!("importance: {:.2}\n", candidate.importance));
    out.push_str(&format!("temporality: {}\n", candidate.temporality));
    out.push_str("managed_by: openwiki\n");
    out.push_str("workspace_layer: wiki_candidate\n");
    out.push_str(&format!("captured_at: {}\n", content.captured_at));
    out.push_str("---\n\n");

    out.push_str(&format!("# {}\n\n", candidate.name));
    out.push_str(&format!(
        "- Candidate source: {}\n",
        markdown_title(content)
    ));
    out.push_str(&format!("- Captured at: {}\n", content.captured_at));
    out.push_str(&format!("- Source app: {}\n", content.source_app));
    out.push_str(&format!("- Importance: {:.2}\n", candidate.importance));
    out.push_str(&format!("- Temporality: {}\n", candidate.temporality));
    out.push_str(&format!("- Current supporting sources: {}\n", source_count));
    out.push_str(&format!("- Current supporting days: {}\n", day_count));
    out.push_str(&format!("- Average importance: {:.2}\n", avg_importance));
    out.push('\n');

    if let Some(rationale) = candidate
        .rationale
        .as_deref()
        .filter(|rationale| !rationale.trim().is_empty())
    {
        out.push_str("## Why it matters\n\n");
        out.push_str(rationale.trim());
        out.push_str("\n\n");
    }

    if let Some(summary) = content
        .summary
        .as_deref()
        .filter(|summary| !summary.trim().is_empty())
    {
        out.push_str("## Source summary\n\n");
        out.push_str(summary.trim());
        out.push_str("\n\n");
    }

    if let Some(raw_text) = content
        .clean_content
        .as_deref()
        .or(content.raw_text.as_deref())
        .filter(|text| !text.trim().is_empty())
    {
        out.push_str("## Source excerpt\n\n");
        out.push_str(raw_text.trim().chars().take(800).collect::<String>().trim());
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

fn sync_wiki_page_to_workspace_draft_with_paths(
    repo: &Repository,
    paths: &WorkspaceProtocolPaths,
    page: &WikiPage,
) -> Result<Option<PathBuf>, String> {
    if page
        .source_message_id
        .as_deref()
        .is_some_and(|source| source.starts_with(LOCAL_WIKI_SOURCE_PREFIX))
    {
        return Ok(None);
    }

    let note_path = workspace_wiki_draft_note_path(paths, page);
    if let Some(parent) = note_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create workspace draft directory: {}", e))?;
    }

    let note_markdown = render_workspace_wiki_draft_markdown(repo, page);
    fs::write(&note_path, note_markdown)
        .map_err(|e| format!("Failed to write workspace wiki draft: {}", e))?;

    Ok(Some(note_path))
}

pub fn sync_wiki_page_to_workspace_draft(
    repo: &Repository,
    page: &WikiPage,
) -> Result<Option<PathBuf>, String> {
    let Some(paths) = load_workspace_paths(repo)? else {
        return Ok(None);
    };
    sync_wiki_page_to_workspace_draft_with_paths(repo, &paths, page)
}

pub fn sync_wiki_pages_to_workspace_drafts(
    repo: &Repository,
    page_ids: &[String],
) -> Result<usize, String> {
    let Some(paths) = load_workspace_paths(repo)? else {
        return Ok(0);
    };

    let mut written = 0usize;
    for page_id in page_ids {
        let Some(page) = repo
            .get_wiki_page_by_id(page_id)
            .map_err(|e| format!("Failed to load wiki page {}: {}", page_id, e))?
        else {
            continue;
        };
        if sync_wiki_page_to_workspace_draft_with_paths(repo, &paths, &page)?.is_some() {
            written += 1;
        }
    }

    Ok(written)
}

pub fn remove_wiki_page_workspace_draft(repo: &Repository, page: &WikiPage) -> Result<(), String> {
    if page
        .source_message_id
        .as_deref()
        .is_some_and(|source| source.starts_with(LOCAL_WIKI_SOURCE_PREFIX))
    {
        return Ok(());
    }

    let Some(paths) = load_workspace_paths(repo)? else {
        return Ok(());
    };
    let note_path = workspace_wiki_draft_note_path(&paths, page);
    if note_path.exists() {
        fs::remove_file(&note_path)
            .map_err(|e| format!("Failed to remove workspace wiki draft: {}", e))?;
    }
    Ok(())
}

pub fn sync_content_candidates_to_workspace(
    repo: &Repository,
    content: &CapturedContent,
    candidates: &[ConceptCandidate],
) -> Result<usize, String> {
    let Some(paths) = load_workspace_paths(repo)? else {
        return Ok(0);
    };

    let candidates_dir = workspace_candidate_concepts_dir(&paths);
    fs::create_dir_all(&candidates_dir)
        .map_err(|e| format!("Failed to create candidate concept directory: {}", e))?;

    let prefix = workspace_candidate_file_prefix(content);
    for entry in fs::read_dir(&candidates_dir)
        .map_err(|e| format!("Failed to scan candidate concept directory: {}", e))?
    {
        let entry = entry.map_err(|e| format!("Failed to read candidate entry: {}", e))?;
        let file_name = entry.file_name();
        if file_name.to_string_lossy().starts_with(&prefix) {
            fs::remove_file(entry.path())
                .map_err(|e| format!("Failed to clear stale candidate note: {}", e))?;
        }
    }

    let mut written = 0usize;
    for candidate in candidates {
        let (_, source_count, day_count, avg_importance, _) = repo
            .get_promotable_candidate_support(&candidate.normalized_name, 0.0)
            .map_err(|e| format!("Failed to inspect candidate support: {}", e))?;
        let note_path = workspace_candidate_note_path(&paths, content, candidate);
        let note_markdown = render_workspace_candidate_markdown(
            content,
            candidate,
            source_count,
            day_count,
            avg_importance,
        );
        fs::write(&note_path, note_markdown)
            .map_err(|e| format!("Failed to write workspace candidate concept note: {}", e))?;
        written += 1;
    }

    Ok(written)
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
    let note_markdown =
        render_workspace_inbox_markdown(content, asset_relative_path.as_deref(), "inbox");
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
    let note_markdown =
        render_workspace_inbox_markdown(content, raw_asset_relative.as_deref(), "raw");
    fs::write(&note_path, note_markdown)
        .map_err(|e| format!("Failed to write workspace raw note: {}", e))?;

    Ok(Some(note_path))
}
