use crate::commands::capture::AppState;
use crate::storage::repository::Repository;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use tauri::State;

/// Supported platforms with built-in readers (no external tools needed).
const BUILTIN_PLATFORMS: &[&str] = &[
    "mp.weixin.qq.com (WeChat)",
    "x.com / twitter.com (X/Twitter)",
    "Other web pages (via Jina Reader)",
];

#[derive(Serialize, Deserialize)]
pub struct XReaderStatus {
    pub installed: bool,
    pub supported_platforms: Vec<String>,
    pub install_command: String,
}

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Result<HashMap<String, String>, String> {
    let repo = Repository::new(state.db.clone());
    repo.get_all_settings()
        .map_err(|e| format!("Failed to get settings: {}", e))
}

#[tauri::command]
pub fn update_setting(
    state: State<'_, AppState>,
    key: String,
    value: String,
) -> Result<(), String> {
    let repo = Repository::new(state.db.clone());
    repo.update_setting(&key, &value)
        .map_err(|e| format!("Failed to update setting: {}", e))
}

/// Returns true/false if the system dark-mode preference can be determined natively,
/// or null (None) to let the frontend fall back to window.matchMedia.
#[tauri::command]
pub fn get_system_dark_mode() -> Option<bool> {
    #[cfg(target_os = "linux")]
    {
        // GNOME 42+: org.gnome.desktop.interface color-scheme
        if let Ok(out) = std::process::Command::new("gsettings")
            .args(["get", "org.gnome.desktop.interface", "color-scheme"])
            .output()
        {
            if out.status.success() {
                let s = String::from_utf8_lossy(&out.stdout);
                if s.contains("prefer-dark") {
                    return Some(true);
                }
                if s.contains("prefer-light") || s.contains("default") {
                    return Some(false);
                }
            }
        }
        // Older GNOME / KDE fallback: check GTK theme name
        if let Ok(out) = std::process::Command::new("gsettings")
            .args(["get", "org.gnome.desktop.interface", "gtk-theme"])
            .output()
        {
            if out.status.success() {
                let s = String::from_utf8_lossy(&out.stdout).to_lowercase();
                return Some(s.contains("dark"));
            }
        }
        Some(false)
    }
    #[cfg(not(target_os = "linux"))]
    {
        None // matchMedia is reliable on macOS and Windows
    }
}

#[tauri::command]
pub fn get_default_screenshot_dir() -> String {
    let path: PathBuf = {
        #[cfg(target_os = "linux")]
        {
            dirs::picture_dir()
                .map(|d| d.join("openwiki-screenshots"))
                .or_else(|| dirs::home_dir().map(|h| h.join("Pictures").join("openwiki-screenshots")))
                .unwrap_or_else(|| PathBuf::from("/tmp/openwiki-screenshots"))
        }
        #[cfg(not(target_os = "linux"))]
        {
            dirs::data_dir()
                .map(|d| d.join("com.openwiki.app").join("screenshots"))
                .unwrap_or_else(|| PathBuf::from("com.openwiki.app/screenshots"))
        }
    };
    path.to_string_lossy().to_string()
}

#[tauri::command]
pub fn check_xreader_status() -> Result<XReaderStatus, String> {
    // Built-in readers — no external Python dependencies needed
    let supported_platforms: Vec<String> =
        BUILTIN_PLATFORMS.iter().map(|s| s.to_string()).collect();

    Ok(XReaderStatus {
        installed: true, // Built-in, always available
        supported_platforms,
        install_command: String::new(),
    })
}
