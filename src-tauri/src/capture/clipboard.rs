use once_cell::sync::Lazy;
use sha2::{Digest, Sha256};
use std::sync::Mutex;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;
use std::time::Instant;
use tauri::{AppHandle, Emitter};

static SELF_WRITTEN_TEXT: Lazy<Mutex<Option<(String, Instant)>>> = Lazy::new(|| Mutex::new(None));

pub fn write_text_suppressed(text: &str) -> Result<(), String> {
    let hash = compute_text_hash(text);
    *SELF_WRITTEN_TEXT.lock().map_err(|e| e.to_string())? = Some((hash, Instant::now()));
    let mut clipboard =
        arboard::Clipboard::new().map_err(|e| format!("Cannot access clipboard: {}", e))?;
    if let Err(error) = clipboard.set_text(text) {
        *SELF_WRITTEN_TEXT.lock().map_err(|e| e.to_string())? = None;
        return Err(format!("Failed to write to clipboard: {}", error));
    }
    Ok(())
}

fn consume_self_written(hash: &str) -> bool {
    let Ok(mut entry) = SELF_WRITTEN_TEXT.lock() else {
        return false;
    };
    let matches = entry
        .as_ref()
        .map(|(saved, at)| saved == hash && at.elapsed() < Duration::from_secs(3))
        .unwrap_or(false);
    if matches
        || entry
            .as_ref()
            .map(|(_, at)| at.elapsed() >= Duration::from_secs(3))
            .unwrap_or(false)
    {
        *entry = None;
    }
    matches
}

/// Save arboard::ImageData (raw RGBA pixels) to a PNG file on disk.
/// Returns the file path if successful.
fn save_clipboard_image_to_disk(img: &arboard::ImageData) -> Option<String> {
    let captures_dir = super::image_lifecycle::pending_images_dir().ok()?;

    let id = uuid::Uuid::new_v4().to_string();
    let file_path = captures_dir.join(format!("{}.png", id));

    // arboard gives us RGBA pixel data
    let rgba_buf =
        image::RgbaImage::from_raw(img.width as u32, img.height as u32, img.bytes.to_vec());

    match rgba_buf {
        Some(buffer) => {
            if let Err(e) = buffer.save(&file_path) {
                log::error!("Failed to save clipboard image to disk: {}", e);
                let _ = std::fs::remove_file(&file_path);
                return None;
            }
            let path_str = file_path.to_string_lossy().to_string();
            log::info!("Clipboard image saved as pending: {}", path_str);
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
                            if consume_self_written(&hash) {
                                last_content_hash = Some(hash);
                                std::thread::sleep(Duration::from_millis(interval));
                                continue;
                            }

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

/// Detect the frontmost application on macOS using osascript.
fn detect_frontmost_app() -> String {
    #[cfg(target_os = "macos")]
    {
        return detect_frontmost_app_macos();
    }

    #[cfg(target_os = "windows")]
    {
        return detect_frontmost_app_windows();
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        "Unknown".to_string()
    }
}

#[cfg(target_os = "macos")]
fn detect_frontmost_app_macos() -> String {
    match std::process::Command::new("osascript")
        .args([
            "-e",
            "tell application \"System Events\" to get name of first application process whose frontmost is true",
        ])
        .output()
    {
        Ok(output) if output.status.success() => {
            let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if name.is_empty() {
                "Unknown".to_string()
            } else {
                name
            }
        }
        Ok(_) => "Unknown".to_string(),
        Err(e) => {
            log::error!("Failed to detect frontmost app: {}", e);
            "Unknown".to_string()
        }
    }
}

#[cfg(target_os = "windows")]
fn detect_frontmost_app_windows() -> String {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowTextLengthW, GetWindowTextW,
    };

    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_null() {
            return "Unknown".to_string();
        }

        let len = GetWindowTextLengthW(hwnd);
        if len <= 0 {
            return "Unknown".to_string();
        }

        let mut buf = vec![0u16; (len + 1) as usize];
        let copied = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
        if copied <= 0 {
            return "Unknown".to_string();
        }

        let title = String::from_utf16_lossy(&buf[..copied as usize])
            .trim()
            .to_string();
        if title.is_empty() {
            "Unknown".to_string()
        } else {
            title
        }
    }
}

/// Compute a SHA-256 hash for text content (prefixed to distinguish from image hashes).
fn compute_text_hash(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"text:");
    hasher.update(text.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Fingerprint of a clipboard image over all of its pixels, so two same-size screenshots that only
/// differ further down are not taken for the same copy. SipHash takes about 8 ms for a 4K image
/// where SHA-256 takes about 75 ms, and this runs every poll; the fingerprint is only compared
/// within this run, so it does not need to be stable or cryptographic.
fn compute_image_hash(img: &arboard::ImageData) -> String {
    use std::hash::{DefaultHasher, Hasher};

    let mut hasher = DefaultHasher::new();
    hasher.write_usize(img.width);
    hasher.write_usize(img.height);
    hasher.write(&img.bytes);
    format!("img:{:016x}", hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::compute_image_hash;
    use std::borrow::Cow;

    fn image(width: usize, height: usize, bytes: Vec<u8>) -> arboard::ImageData<'static> {
        arboard::ImageData {
            width,
            height,
            bytes: Cow::Owned(bytes),
        }
    }

    #[test]
    fn same_size_images_that_differ_only_at_the_end_get_different_hashes() {
        let first = vec![7u8; 1920 * 1080 * 4];
        let mut second = first.clone();
        *second.last_mut().unwrap() = 8;

        assert_ne!(
            compute_image_hash(&image(1920, 1080, first)),
            compute_image_hash(&image(1920, 1080, second))
        );
    }

    #[test]
    fn identical_images_get_the_same_hash() {
        let pixels = vec![3u8; 64 * 64 * 4];

        assert_eq!(
            compute_image_hash(&image(64, 64, pixels.clone())),
            compute_image_hash(&image(64, 64, pixels))
        );
    }
}
