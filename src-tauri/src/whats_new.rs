//! One-time "what's new" card in the main window, shown after an update.

use crate::commands::capture::AppState;
use crate::storage::repository::Repository;
use std::sync::Mutex;
use tauri::Manager;

/// The announcement this build carries. A new feature gets a new id to be announced once more.
const CURRENT: &str = "football-2026-09";
/// Id of the announcement the user has already seen.
const SEEN_KEY: &str = "whats_new_seen";
/// Version of the previous launch, so an update can be told apart from a fresh install.
const LAST_VERSION_KEY: &str = "last_run_version";

/// The announcement waiting for the main window in this launch, if any.
pub struct WhatsNewState(pub Mutex<Option<String>>);

impl WhatsNewState {
    pub fn new() -> Self {
        Self(Mutex::new(None))
    }
}

/// Decides at startup whether this launch shows the card; returns true when the main window
/// should open for it. Must run before anything writes first-run markers: only people who used
/// an earlier version get the card. A fresh install just marks it as seen, and so does someone
/// who already uses the football bubble.
pub fn prepare(app: &tauri::App, first_run_marker: &str) -> bool {
    let state: tauri::State<'_, AppState> = app.state();
    let repo = Repository::new(state.db.clone());
    let setting = |key: &str| repo.get_setting(key).ok().flatten();

    let used_before = setting(LAST_VERSION_KEY).is_some() || setting(first_run_marker).is_some();
    let version = app.package_info().version.to_string();
    if let Err(e) = repo.update_setting(LAST_VERSION_KEY, &version) {
        log::warn!("[whats-new] failed to record the running version: {}", e);
    }
    if setting(SEEN_KEY).as_deref() == Some(CURRENT) {
        return false;
    }
    // The football bubble is Mac-only for now, so other platforms skip its announcement. It is
    // not marked as seen, so a later release that brings football there can still show it.
    if !cfg!(target_os = "macos") {
        return false;
    }
    if !used_before || setting("bubble_style").as_deref() == Some("football") {
        if let Err(e) = repo.update_setting(SEEN_KEY, CURRENT) {
            log::warn!("[whats-new] failed to skip the announcement: {}", e);
        }
        return false;
    }
    if let Ok(mut pending) = app.state::<WhatsNewState>().0.lock() {
        *pending = Some(CURRENT.to_string());
    }
    log::info!("[whats-new] showing '{}' after updating to {}", CURRENT, version);
    true
}

/// The announcement the main window should show now, unless it has been seen already.
#[tauri::command]
pub fn get_whats_new(
    state: tauri::State<'_, AppState>,
    whats_new: tauri::State<'_, WhatsNewState>,
) -> Option<String> {
    let pending = whats_new.0.lock().ok()?.clone()?;
    let seen = Repository::new(state.db.clone()).get_setting(SEEN_KEY).ok().flatten();
    (seen.as_deref() != Some(pending.as_str())).then_some(pending)
}

/// The card is on screen: never bring this announcement back.
#[tauri::command]
pub fn mark_whats_new_seen(id: String, state: tauri::State<'_, AppState>) -> Result<(), String> {
    Repository::new(state.db.clone())
        .update_setting(SEEN_KEY, &id)
        .map_err(|e| e.to_string())
}
