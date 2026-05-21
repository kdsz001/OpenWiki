use sha2::{Digest, Sha256};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;
use tauri::{AppHandle, Emitter};

/// Save arboard::ImageData (raw RGBA pixels) to a PNG file on disk.
/// Returns the file path if successful.
fn save_clipboard_image_to_disk(img: &arboard::ImageData) -> Option<String> {
    let base = dirs::data_dir()
        .or_else(|| dirs::home_dir().map(|h| h.join("Library").join("Application Support")))?;

    let captures_dir = base.join("com.openwiki.app").join("captures");
    if let Err(e) = std::fs::create_dir_all(&captures_dir) {
        log::error!("Failed to create captures directory: {}", e);
        return None;
    }

    let id = uuid::Uuid::new_v4().to_string();
    let file_path = captures_dir.join(format!("{}.png", id));

    // arboard gives us RGBA pixel data
    let rgba_buf =
        image::RgbaImage::from_raw(img.width as u32, img.height as u32, img.bytes.to_vec());

    match rgba_buf {
        Some(buffer) => {
            if let Err(e) = buffer.save(&file_path) {
                log::error!("Failed to save clipboard image to disk: {}", e);
                return None;
            }
            let path_str = file_path.to_string_lossy().to_string();
            log::info!("Clipboard image saved to disk: {}", path_str);
            Some(path_str)
        }
        None => {
            log::error!(
                "Failed to create image buffer from clipboard data ({}x{}, {} bytes)",
                img.width,
                img.height,
                img.bytes.len()
            );
            None
        }
    }
}

/// Default polling interval in milliseconds.
const DEFAULT_POLL_INTERVAL_MS: u64 = 500;

pub struct ClipboardWatcher {
    running: Arc<AtomicBool>,
    poll_interval_ms: u64,
}

impl ClipboardWatcher {
    pub fn new() -> Self {
        ClipboardWatcher {
            running: Arc::new(AtomicBool::new(false)),
            poll_interval_ms: DEFAULT_POLL_INTERVAL_MS,
        }
    }

    /// Create a watcher with a custom polling interval (in milliseconds).
    pub fn with_interval(interval_ms: u64) -> Self {
        ClipboardWatcher {
            running: Arc::new(AtomicBool::new(false)),
            poll_interval_ms: if interval_ms > 0 {
                interval_ms
            } else {
                DEFAULT_POLL_INTERVAL_MS
            },
        }
    }

    pub fn start(&self, app: AppHandle) {
        let running = self.running.clone();
        running.store(true, Ordering::SeqCst);
        let interval = self.poll_interval_ms;

        std::thread::spawn(move || {
            let mut last_content_hash: Option<String> = None;

            eprintln!(
                "[openwiki] Clipboard watcher thread started ({}ms)",
                interval
            );
            log::info!(
                "Clipboard watcher started with {}ms polling interval",
                interval
            );

            while running.load(Ordering::SeqCst) {
                if let Ok(mut clipboard) = arboard::Clipboard::new() {
                    // Try to detect image clipboard content first
                    if let Ok(img) = clipboard.get_image() {
                        let hash = compute_image_hash(&img);

                        let is_new = match &last_content_hash {
                            Some(prev) => prev != &hash,
                            None => true,
                        };

                        if is_new {
                            last_content_hash = Some(hash);
                            let source_app = detect_frontmost_app();

                            let preview =
                                format!("Image {}x{} from clipboard", img.width, img.height);

                            // Save clipboard image pixels to disk as PNG
                            let image_path = save_clipboard_image_to_disk(&img);

                            if image_path.is_none() {
                                log::warn!("Clipboard image detected but could not be saved to disk, skipping");
                            } else {
                                let event = serde_json::json!({
                                    "content_type": "image",
                                    "preview": preview,
                                    "source_app": source_app,
                                    "raw_text": null,
                                    "image_path": image_path,
                                    "image_width": img.width,
                                    "image_height": img.height,
                                    "from_clipboard": true
                                });

                                log::info!(
                                    "Clipboard image detected: {}x{} from {}",
                                    img.width,
                                    img.height,
                                    source_app
                                );

                                if let Err(e) = app.emit("capture:clipboard", event) {
                                    log::error!("Failed to emit clipboard image event: {}", e);
                                }
                            }
                        }
                    }
                    // Then try text clipboard content
                    else if let Ok(text) = clipboard.get_text() {
                        if !text.is_empty() {
                            let hash = compute_text_hash(&text);

                            let is_new = match &last_content_hash {
                                Some(prev) => prev != &hash,
                                None => true,
                            };

                            if is_new {
                                eprintln!(
                                    "[openwiki] New clipboard text detected: {} chars",
                                    text.len()
                                );
                                last_content_hash = Some(hash);
                                let source_app = detect_frontmost_app();

                                let preview = if text.chars().count() > 100 {
                                    let truncated: String = text.chars().take(100).collect();
                                    format!("{}...", truncated)
                                } else {
                                    text.clone()
                                };

                                // Cap raw_text sent via IPC to avoid crashes with very large clipboard content.
                                // The full text is still used for hashing/dedup; storage will use this capped version.
                                const MAX_RAW_TEXT_CHARS: usize = 50_000;
                                let raw_text_for_event =
                                    if text.chars().count() > MAX_RAW_TEXT_CHARS {
                                        let truncated: String =
                                            text.chars().take(MAX_RAW_TEXT_CHARS).collect();
                                        truncated
                                    } else {
                                        text.clone()
                                    };

                                let event = serde_json::json!({
                                    "content_type": "text",
                                    "preview": preview,
                                    "source_app": source_app,
                                    "raw_text": raw_text_for_event,
                                    "image_path": null,
                                    "from_clipboard": true
                                });

                                log::info!(
                                    "Clipboard text detected: {} chars from {}",
                                    text.len(),
                                    source_app
                                );

                                if let Err(e) = app.emit("capture:clipboard", event) {
                                    log::error!("Failed to emit clipboard text event: {}", e);
                                }
                            }
                        }
                    }
                }

                std::thread::sleep(Duration::from_millis(interval));
            }

            log::info!("Clipboard watcher stopped");
        });
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }
}

/// Detect the frontmost application name.
fn detect_frontmost_app() -> String {
    #[cfg(target_os = "macos")]
    {
        match std::process::Command::new("osascript")
            .args([
                "-e",
                "tell application \"System Events\" to get name of first application process whose frontmost is true",
            ])
            .output()
        {
            Ok(output) if output.status.success() => {
                let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if name.is_empty() { "Unknown".to_string() } else { name }
            }
            Ok(_) => "Unknown".to_string(),
            Err(e) => {
                log::error!("Failed to detect frontmost app: {}", e);
                "Unknown".to_string()
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        "Unknown".to_string()
    }
}

/// Compute a SHA-256 hash for text content (prefixed to distinguish from image hashes).
fn compute_text_hash(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"text:");
    hasher.update(text.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Compute a SHA-256 hash from image metadata (dimensions + first bytes of pixel data).
/// We sample the data to avoid hashing potentially large image buffers every poll cycle.
fn compute_image_hash(img: &arboard::ImageData) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"img:");
    hasher.update(img.width.to_le_bytes());
    hasher.update(img.height.to_le_bytes());
    // Sample up to 4096 bytes from the image data for a fast fingerprint
    let sample_len = img.bytes.len().min(4096);
    hasher.update(&img.bytes[..sample_len]);
    format!("{:x}", hasher.finalize())
}
