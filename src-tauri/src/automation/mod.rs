//! macOS Automation (Apple Events) permission guard.
//!
//! OpenWiki uses `osascript` + `tell application "System Events"` to identify
//! the frontmost app when the user copies something. This requires the macOS
//! "Automation" permission, which is granted via a system dialog the first
//! time the call happens.
//!
//! This module owns the permission lifecycle so users aren't surprised by
//! the raw system dialog. On first launch we emit an event that triggers a
//! friendly pre-auth modal in the frontend; the user then clicks "Start"
//! which invokes `request_automation_permission`, which runs the real
//! osascript call — *that's* what pops the system dialog. After the user
//! responds, we persist the outcome to SQLite so subsequent launches can
//! act accordingly (quiet success, or a red denial banner + repair flow).

use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

use crate::commands::capture::AppState;
use crate::storage::database::Database;
use crate::storage::repository::Repository;

// ========== constants ==========

const SETTING_INITIAL_PROMPT_SHOWN: &str = "automation.initial_prompt_shown";
const SETTING_LAST_STATUS: &str = "automation.last_status";

/// Minimal AppleScript that needs Automation permission but does nothing
/// observable. If this succeeds, we have permission; if it errors, we don't.
const PROBE_SCRIPT: &str = r#"tell application "System Events" to return 1"#;

/// How long to wait before the startup detection runs — gives the UI time
/// to render before a modal pops in the user's face.
const STARTUP_DELAY_SECS: u64 = 2;

// ========== public types ==========

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AutomationStatus {
    /// App has never asked for permission — show the pre-auth modal.
    Unknown,
    /// User previously clicked the primary "start" button in our modal
    /// and has been granted by the system.
    Granted,
    /// User previously hit the system's "Don't Allow" — future calls will
    /// silently fail until they toggle it back in System Settings.
    Denied,
    /// User clicked our secondary "later" button; we haven't bothered the
    /// system yet. They can re-trigger from Settings.
    Dismissed,
}

impl AutomationStatus {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Granted => "granted",
            Self::Denied => "denied",
            Self::Dismissed => "dismissed",
        }
    }

    fn from_str(s: &str) -> Self {
        match s {
            "granted" => Self::Granted,
            "denied" => Self::Denied,
            "dismissed" => Self::Dismissed,
            _ => Self::Unknown,
        }
    }
}

/// Snapshot returned to the frontend — includes both the cached decision
/// from SQLite and the live probe result for freshness.
#[derive(Debug, Clone, Serialize)]
pub struct AutomationSnapshot {
    pub status: AutomationStatus,
    /// Whether we've ever shown the user our pre-auth modal. Frontend uses
    /// this to decide whether to show the first-time welcome flow.
    pub initial_prompt_shown: bool,
}

// ========== public entry points ==========

/// Background task spawned from the Tauri setup hook. Checks the persisted
/// status after a short delay and emits the appropriate event for the
/// frontend to react to.
pub fn spawn_startup_check(app: AppHandle, db: Arc<Database>) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(STARTUP_DELAY_SECS)).await;

        let snapshot = read_snapshot(&db);

        match (snapshot.initial_prompt_shown, snapshot.status) {
            // First launch — show the pre-auth modal. We do NOT probe yet
            // because probing would immediately pop the system dialog and
            // bypass our own explanation.
            (false, _) => {
                log::info!("[automation] first launch, asking frontend to show pre-auth modal");
                if let Err(e) = app.emit("automation-needed", ()) {
                    log::warn!("[automation] failed to emit automation-needed: {}", e);
                }
            }

            // User previously granted — verify it's still true with a safe probe.
            // If it's still granted, silent success. If revoked, show the banner.
            (true, AutomationStatus::Granted) => {
                let live = probe_status();
                if live == AutomationStatus::Granted {
                    log::info!("[automation] permission still granted");
                } else {
                    log::warn!("[automation] permission revoked since last launch, showing banner");
                    persist_status(&db, live);
                    let _ = app.emit("automation-denied", ());
                }
            }

            // User previously denied — still denied unless they fixed it in
            // System Settings. Probe and emit accordingly.
            (true, AutomationStatus::Denied) => {
                let live = probe_status();
                if live == AutomationStatus::Granted {
                    log::info!("[automation] user enabled permission manually, clearing banner");
                    persist_status(&db, live);
                    let _ = app.emit("automation-granted", ());
                } else {
                    let _ = app.emit("automation-denied", ());
                }
            }

            // User dismissed the modal without interacting with the system
            // dialog. Don't nag — settings has the re-trigger button.
            (true, AutomationStatus::Dismissed) => {
                log::info!("[automation] user previously dismissed the pre-auth modal");
            }

            (true, AutomationStatus::Unknown) => {
                // Weird state, just act like first launch.
                log::warn!("[automation] stale 'unknown' status with initial_prompt_shown=true");
                let _ = app.emit("automation-needed", ());
            }
        }
    });
}

// ========== Tauri commands ==========

/// Read the persisted snapshot. Frontend uses this when rendering the
/// Settings → Diagnostics pane.
#[tauri::command]
pub fn get_automation_status(state: State<'_, AppState>) -> Result<AutomationSnapshot, String> {
    Ok(read_snapshot(&state.db))
}

/// The user clicked "开始授权" in the pre-auth modal. We run the real
/// osascript call, which will:
///   1. pop the system dialog (first ever call), OR
///   2. succeed silently (previously granted), OR
///   3. fail immediately (previously denied at OS level).
///
/// Then we persist the outcome and return it to the frontend.
#[tauri::command]
pub fn request_automation_permission(
    state: State<'_, AppState>,
) -> Result<AutomationSnapshot, String> {
    log::info!("[automation] running probe (may show system dialog)");
    let live = probe_status();
    persist_initial_shown(&state.db);
    persist_status(&state.db, live);
    log::info!("[automation] probe result: {:?}", live);
    Ok(read_snapshot(&state.db))
}

/// User clicked "稍后再说". We remember they saw the modal so we don't
/// re-pop it automatically, but we don't touch the system.
#[tauri::command]
pub fn dismiss_automation_prompt(state: State<'_, AppState>) -> Result<(), String> {
    persist_initial_shown(&state.db);
    persist_status(&state.db, AutomationStatus::Dismissed);
    log::info!("[automation] user dismissed the pre-auth modal");
    Ok(())
}

/// Jump straight to the macOS "Privacy & Security → Automation" pane so
/// users who previously denied can fix it in one click.
#[tauri::command]
pub fn open_automation_settings() -> Result<(), String> {
    let url = "x-apple.systempreferences:com.apple.preference.security?Privacy_Automation";
    std::process::Command::new("open")
        .arg(url)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Failed to open Automation settings: {}", e))
}

// ========== internals ==========

fn read_snapshot(db: &Arc<Database>) -> AutomationSnapshot {
    let repo = Repository::new(db.clone());

    let initial_prompt_shown = repo
        .get_setting(SETTING_INITIAL_PROMPT_SHOWN)
        .ok()
        .flatten()
        .map(|v| v == "true")
        .unwrap_or(false);

    let status = repo
        .get_setting(SETTING_LAST_STATUS)
        .ok()
        .flatten()
        .map(|v| AutomationStatus::from_str(&v))
        .unwrap_or(AutomationStatus::Unknown);

    AutomationSnapshot {
        status,
        initial_prompt_shown,
    }
}

fn persist_initial_shown(db: &Arc<Database>) {
    let repo = Repository::new(db.clone());
    if let Err(e) = repo.update_setting(SETTING_INITIAL_PROMPT_SHOWN, "true") {
        log::warn!("[automation] failed to persist initial_prompt_shown: {}", e);
    }
}

fn persist_status(db: &Arc<Database>, status: AutomationStatus) {
    let repo = Repository::new(db.clone());
    if let Err(e) = repo.update_setting(SETTING_LAST_STATUS, status.as_str()) {
        log::warn!("[automation] failed to persist status: {}", e);
    }
}

/// Run the probe script. This blocks the calling thread until either the
/// system dialog closes (first call) or osascript finishes (subsequent).
/// Returns `Granted` on success, `Denied` on any failure.
fn probe_status() -> AutomationStatus {
    let result = std::process::Command::new("osascript")
        .args(["-e", PROBE_SCRIPT])
        .output();

    match result {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if stdout.trim() == "1" {
                AutomationStatus::Granted
            } else {
                log::warn!(
                    "[automation] osascript returned unexpected stdout: {:?}",
                    stdout
                );
                AutomationStatus::Denied
            }
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            log::warn!("[automation] osascript failed: {}", stderr.trim());
            AutomationStatus::Denied
        }
        Err(e) => {
            log::warn!("[automation] osascript invocation failed: {}", e);
            AutomationStatus::Denied
        }
    }
}
