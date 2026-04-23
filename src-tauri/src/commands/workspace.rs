use crate::commands::capture::AppState;
use crate::storage::repository::Repository;
use tauri::State;

#[tauri::command]
pub fn get_workspace_paths(
    state: State<'_, AppState>,
) -> Result<crate::workspace::WorkspaceProtocolPaths, String> {
    let repo = Repository::new(state.db.clone());
    Ok(crate::workspace::resolve_workspace_paths_from_repo(&repo))
}

#[tauri::command]
pub fn initialize_workspace_root(
    state: State<'_, AppState>,
    path: Option<String>,
) -> Result<crate::workspace::WorkspaceProtocolPaths, String> {
    let repo = Repository::new(state.db.clone());
    crate::workspace::ensure_workspace_root(&repo, path)
}

#[tauri::command]
pub fn open_workspace_root(state: State<'_, AppState>) -> Result<String, String> {
    let repo = Repository::new(state.db.clone());
    let paths = crate::workspace::resolve_workspace_paths_from_repo(&repo);
    std::fs::create_dir_all(&paths.root)
        .map_err(|e| format!("Failed to create workspace root: {}", e))?;

    std::process::Command::new("open")
        .arg(&paths.root)
        .spawn()
        .map_err(|e| format!("Failed to open workspace root: {}", e))?;

    Ok(paths.root)
}
