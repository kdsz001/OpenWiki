//! In-app update notifier.
//!
//! Polls the GitHub Releases API on startup (and on demand) to detect when a
//! newer version of OpenWiki is available. When one is found, we emit an
//! `update-available` Tauri event that the frontend renders as a banner.
//!
//! This is intentionally a *notification only* feature. The user still
//! downloads and installs the new DMG manually — we just surface the fact
//! that one exists and deep-link into the GitHub release page. If we ever
//! upgrade to real auto-updates we'll switch to `tauri-plugin-updater`.

use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

use crate::commands::capture::AppState;
use crate::storage::database::Database;
use crate::storage::repository::Repository;

const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
const GITHUB_API_URL: &str = "https://api.github.com/repos/kdsz001/OpenWiki/releases/latest";
const RELEASES_PAGE_URL: &str = "https://github.com/kdsz001/OpenWiki/releases";

const SETTING_CHECK_ENABLED: &str = "update.check_enabled";
const SETTING_DISMISSED_VERSION: &str = "update.dismissed_version";

const REQUEST_TIMEOUT_SECS: u64 = 10;
const STARTUP_DELAY_SECS: u64 = 3;

/// Raw shape of the GitHub `/releases/latest` response — only the fields we need.
#[derive(Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    body: String,
    html_url: String,
    #[serde(default)]
    published_at: String,
}

/// Payload emitted to the frontend as the `update-available` event and
/// returned from the manual check command.
#[derive(Debug, Clone, Serialize)]
pub struct UpdateInfo {
    /// Stripped latest version, e.g. `"0.1.3"` (no leading `v`).
    pub version: String,
    /// Current running version from `CARGO_PKG_VERSION`.
    pub current_version: String,
    /// Human-readable release title (falls back to tag if empty).
    pub name: String,
    /// Release notes (Markdown).
    pub body: String,
    /// Link to the GitHub release page — opened in browser on user click.
    pub url: String,
    /// ISO-8601 publish timestamp.
    pub published_at: String,
}

/// Update check settings surfaced to the Settings UI.
#[derive(Debug, Serialize)]
pub struct UpdateSettings {
    pub check_enabled: bool,
    pub current_version: String,
    pub releases_url: String,
}

// ========== public entry points ==========

/// Spawn a background task that checks GitHub after the main window has had
/// time to render (3s) and emits `update-available` if there's something new.
///
/// Failures at every step are logged at warn-level and then swallowed — the
/// update check must never interrupt the user or surface errors.
pub fn spawn_background_check(app: AppHandle, db: Arc<Database>) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(STARTUP_DELAY_SECS)).await;

        // Respect the user's "auto-check" toggle.
        let enabled = read_check_enabled(&db).unwrap_or(true);
        if !enabled {
            log::info!("[update] auto-check disabled by user, skipping");
            return;
        }

        let release = match fetch_latest_release().await {
            Ok(r) => r,
            Err(e) => {
                log::warn!("[update] fetch failed: {}", e);
                return;
            }
        };

        let info = match build_update_info(&release) {
            Some(info) => info,
            None => return, // already up to date or unparseable tag
        };

        // Skip if the user already said "later" for this exact version.
        let dismissed = read_dismissed_version(&db).unwrap_or_default();
        if dismissed == info.version {
            log::info!(
                "[update] v{} dismissed by user, not notifying",
                info.version
            );
            return;
        }

        log::info!(
            "[update] new version v{} available (current v{})",
            info.version,
            info.current_version
        );
        if let Err(e) = app.emit("update-available", info) {
            log::warn!("[update] failed to emit event: {}", e);
        }
    });
}

// ========== Tauri commands (invoked from the frontend) ==========

/// Force a check and return the result immediately.
/// Bypasses `dismissed_version` so "Check now" always yields feedback.
#[tauri::command]
pub async fn check_for_update_manual(
    _state: State<'_, AppState>,
) -> Result<Option<UpdateInfo>, String> {
    let release = fetch_latest_release().await.map_err(|e| format!("{}", e))?;
    Ok(build_update_info(&release))
}

/// Persist the "later" click for a specific version so we don't re-nag
/// the user until an even newer version ships.
#[tauri::command]
pub fn dismiss_update_version(state: State<'_, AppState>, version: String) -> Result<(), String> {
    let repo = Repository::new(state.db.clone());
    repo.update_setting(SETTING_DISMISSED_VERSION, &version)
        .map_err(|e| format!("Failed to save dismissed version: {}", e))
}

/// Toggle the auto-check feature from the Settings page.
#[tauri::command]
pub fn set_update_check_enabled(state: State<'_, AppState>, enabled: bool) -> Result<(), String> {
    let repo = Repository::new(state.db.clone());
    let val = if enabled { "true" } else { "false" };
    repo.update_setting(SETTING_CHECK_ENABLED, val)
        .map_err(|e| format!("Failed to save update setting: {}", e))
}

/// Return enough state for the Settings UI to render the update panel.
#[tauri::command]
pub fn get_update_settings(state: State<'_, AppState>) -> Result<UpdateSettings, String> {
    let repo = Repository::new(state.db.clone());
    let enabled = repo
        .get_setting(SETTING_CHECK_ENABLED)
        .map_err(|e| format!("Failed to read update setting: {}", e))?
        .map(|v| v == "true")
        .unwrap_or(true); // default on

    Ok(UpdateSettings {
        check_enabled: enabled,
        current_version: CURRENT_VERSION.to_string(),
        releases_url: RELEASES_PAGE_URL.to_string(),
    })
}

// ========== internals ==========

fn read_check_enabled(db: &Arc<Database>) -> Option<bool> {
    let repo = Repository::new(db.clone());
    repo.get_setting(SETTING_CHECK_ENABLED)
        .ok()
        .flatten()
        .map(|v| v == "true")
}

fn read_dismissed_version(db: &Arc<Database>) -> Option<String> {
    let repo = Repository::new(db.clone());
    repo.get_setting(SETTING_DISMISSED_VERSION).ok().flatten()
}

async fn fetch_latest_release() -> Result<GithubRelease, String> {
    let user_agent = format!(
        "OpenWiki/{} (+https://github.com/kdsz001/OpenWiki)",
        CURRENT_VERSION
    );

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .user_agent(user_agent)
        .build()
        .map_err(|e| format!("build client: {}", e))?;

    let resp = client
        .get(GITHUB_API_URL)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .await
        .map_err(|e| format!("request failed: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("GitHub returned HTTP {}", resp.status()));
    }

    resp.json::<GithubRelease>()
        .await
        .map_err(|e| format!("parse json: {}", e))
}

/// Compare the release against the current build. Returns `Some(UpdateInfo)`
/// only if `release.tag_name` parses as a version that's strictly newer than
/// `CARGO_PKG_VERSION`. Returns `None` otherwise (up to date / unparseable).
fn build_update_info(release: &GithubRelease) -> Option<UpdateInfo> {
    let latest = strip_version_prefix(&release.tag_name);
    if !is_newer(&latest, CURRENT_VERSION) {
        return None;
    }

    let display_name = if release.name.is_empty() {
        release.tag_name.clone()
    } else {
        release.name.clone()
    };

    Some(UpdateInfo {
        version: latest,
        current_version: CURRENT_VERSION.to_string(),
        name: display_name,
        body: release.body.clone(),
        url: release.html_url.clone(),
        published_at: release.published_at.clone(),
    })
}

fn strip_version_prefix(tag: &str) -> String {
    tag.trim()
        .trim_start_matches(|c: char| c == 'v' || c == 'V')
        .to_string()
}

fn is_newer(latest: &str, current: &str) -> bool {
    match (parse_version(latest), parse_version(current)) {
        (Some(l), Some(c)) => l > c,
        _ => {
            log::warn!(
                "[update] failed to parse version pair: latest='{}' current='{}'",
                latest,
                current
            );
            false
        }
    }
}

/// Parse a semver-ish string into `(major, minor, patch)`. Anything after
/// the first non-digit-non-dot char (pre-release or build metadata) is
/// discarded. Returns `None` if fewer than two numeric segments exist.
fn parse_version(s: &str) -> Option<(u32, u32, u32)> {
    let s = s.trim().trim_start_matches(|c: char| c == 'v' || c == 'V');
    let numeric_prefix: String = s
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();

    let parts: Vec<&str> = numeric_prefix.split('.').collect();
    if parts.len() < 2 {
        return None;
    }

    let major = parts.first()?.parse::<u32>().ok()?;
    let minor = parts.get(1)?.parse::<u32>().ok()?;
    let patch = parts
        .get(2)
        .and_then(|p| p.parse::<u32>().ok())
        .unwrap_or(0);

    Some((major, minor, patch))
}

// ========== tests ==========

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_semver() {
        assert_eq!(parse_version("0.1.2"), Some((0, 1, 2)));
    }

    #[test]
    fn parses_v_prefix() {
        assert_eq!(parse_version("v0.1.2"), Some((0, 1, 2)));
    }

    #[test]
    fn parses_missing_patch() {
        assert_eq!(parse_version("v1.2"), Some((1, 2, 0)));
    }

    #[test]
    fn parses_prerelease() {
        assert_eq!(parse_version("v0.1.3-beta"), Some((0, 1, 3)));
    }

    #[test]
    fn parses_build_metadata() {
        assert_eq!(parse_version("v0.1.3+build.1"), Some((0, 1, 3)));
    }

    #[test]
    fn rejects_non_numeric() {
        assert_eq!(parse_version("nightly"), None);
    }

    #[test]
    fn rejects_single_component() {
        assert_eq!(parse_version("v1"), None);
    }

    #[test]
    fn newer_detects_patch_bump() {
        assert!(is_newer("0.1.3", "0.1.2"));
    }

    #[test]
    fn newer_detects_minor_bump() {
        assert!(is_newer("0.2.0", "0.1.9"));
    }

    #[test]
    fn newer_detects_major_bump() {
        assert!(is_newer("1.0.0", "0.99.99"));
    }

    #[test]
    fn newer_rejects_same() {
        assert!(!is_newer("0.1.2", "0.1.2"));
    }

    #[test]
    fn newer_rejects_older() {
        assert!(!is_newer("0.1.1", "0.1.2"));
    }

    #[test]
    fn newer_handles_v_prefix_on_either_side() {
        assert!(is_newer("v0.1.3", "v0.1.2"));
    }

    #[test]
    fn newer_rejects_unparseable_latest() {
        assert!(!is_newer("nightly", "0.1.2"));
    }
}
