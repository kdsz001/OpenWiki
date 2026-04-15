use crate::storage::repository::Repository;
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
