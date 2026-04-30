use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;
use tauri::{AppHandle, Emitter};

// NOTE: Clipboard image detection (Cmd+Ctrl+Shift screenshots) is handled by
// ClipboardWatcher in clipboard.rs. This module ONLY watches the file system
// for screenshot files saved to disk. This avoids concurrent NSPasteboard access
// which causes crashes on macOS.

/// Delay in milliseconds to wait after detecting a new screenshot file,
/// ensuring the file is fully written to disk before emitting the event.
const FILE_SETTLE_DELAY_MS: u64 = 200;

pub struct ScreenshotWatcher {
    running: Arc<AtomicBool>,
}

impl ScreenshotWatcher {
    pub fn new() -> Self {
        ScreenshotWatcher {
            running: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn get_screenshot_dir() -> PathBuf {
        #[cfg(target_os = "macos")]
        {
            // Try to read macOS screenshot location preference
            if let Ok(output) = std::process::Command::new("defaults")
                .args(["read", "com.apple.screencapture", "location"])
                .output()
            {
                if output.status.success() {
                    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    if !path.is_empty() {
                        let p = PathBuf::from(&path);
                        if p.exists() {
                            return p;
                        }
                    }
                }
            }
        }

        // Fall back to Desktop (or Pictures on Linux where Desktop may not exist)
        dirs::desktop_dir()
            .or_else(|| dirs::picture_dir())
            .unwrap_or_else(|| PathBuf::from("/tmp"))
    }

    pub fn start(&self, app: AppHandle) {
        let running = self.running.clone();
        running.store(true, Ordering::SeqCst);

        // Spawn the file-system watcher thread
        let app_clone = app;
        let running_clone = running;
        std::thread::spawn(move || {
            let screenshot_dir = Self::get_screenshot_dir();
            log::info!("Watching screenshot directory: {:?}", screenshot_dir);

            let (tx, rx) = mpsc::channel();

            let mut watcher = match RecommendedWatcher::new(
                move |res: Result<Event, notify::Error>| {
                    if let Ok(event) = res {
                        let _ = tx.send(event);
                    }
                },
                Config::default(),
            ) {
                Ok(w) => w,
                Err(e) => {
                    log::error!("Failed to create file watcher: {}", e);
                    return;
                }
            };

            if let Err(e) = watcher.watch(&screenshot_dir, RecursiveMode::NonRecursive) {
                log::error!("Failed to watch screenshot directory: {}", e);
                return;
            }

            while running_clone.load(Ordering::SeqCst) {
                if let Ok(event) = rx.recv_timeout(Duration::from_secs(1)) {
                    if matches!(event.kind, EventKind::Create(_)) {
                        for path in &event.paths {
                            if Self::is_screenshot(path) {
                                // Wait a short time to ensure the file is fully written
                                std::thread::sleep(Duration::from_millis(FILE_SETTLE_DELAY_MS));

                                let path_str = path.to_string_lossy().to_string();
                                let filename = path
                                    .file_name()
                                    .map(|n| n.to_string_lossy().to_string())
                                    .unwrap_or_default();

                                // Get the file size for logging
                                let file_size =
                                    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);

                                let event_data = serde_json::json!({
                                    "content_type": "image",
                                    "preview": filename,
                                    "source_app": "Screenshot",
                                    "raw_text": null,
                                    "image_path": path_str,
                                    "file_size": file_size,
                                    "from_clipboard": false
                                });

                                log::info!(
                                    "Screenshot file detected: {} ({} bytes)",
                                    path_str,
                                    file_size
                                );

                                if let Err(e) = app_clone.emit("capture:screenshot", event_data) {
                                    log::error!("Failed to emit screenshot event: {}", e);
                                }
                            }
                        }
                    }
                }
            }

            log::info!("Screenshot file watcher stopped");
        });
    }

    fn is_screenshot(path: &std::path::Path) -> bool {
        let filename = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();

        let is_image = matches!(ext.as_str(), "png" | "jpg" | "jpeg");
        let is_screenshot_name = filename.starts_with("Screenshot")
            || filename.starts_with("Screen Shot")
            || filename.starts_with("截屏");

        is_image && is_screenshot_name
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }
}
