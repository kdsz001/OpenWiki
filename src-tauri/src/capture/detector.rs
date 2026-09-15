use super::clipboard::ClipboardWatcher;
use super::screenshot::ScreenshotWatcher;
use super::sensitive_filter::contains_sensitive_data;
use super::url_reader::UrlReader;
use crate::commands::capture::{save_clipboard_content, AppState};
use crate::storage::database::Database;
use crate::storage::models::CaptureEvent;
use crate::storage::repository::Repository;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Listener, Manager};

/// Time window in milliseconds for deduplication.
/// If two capture events (e.g., screenshot file + clipboard image) arrive within
/// this window, only the first one is forwarded to the frontend.
const DEDUP_WINDOW_MS: u128 = 3000;

/// Tracks recent events to deduplicate overlapping captures.
struct DeduplicationState {
    /// Maps a dedup key to the time it was last emitted.
    recent_events: HashMap<String, Instant>,
}

impl DeduplicationState {
    fn new() -> Self {
        DeduplicationState {
            recent_events: HashMap::new(),
        }
    }

    /// Check if an event with the given keys should be emitted.
    /// Returns true if NONE of the keys have been seen within the dedup window.
    /// If any key matches a recently-seen key, the event is suppressed.
    fn should_emit(&mut self, keys: &[String]) -> bool {
        let now = Instant::now();

        // Clean up old entries to prevent unbounded growth
        self.recent_events
            .retain(|_, time| now.duration_since(*time).as_millis() < DEDUP_WINDOW_MS * 5);

        // Check if any key was recently seen
        for key in keys {
            if let Some(last_time) = self.recent_events.get(key) {
                if now.duration_since(*last_time).as_millis() < DEDUP_WINDOW_MS {
                    log::info!("Dedup: suppressing duplicate event for key: {}", key);
                    return false;
                }
            }
        }

        // Record all keys as seen
        for key in keys {
            self.recent_events.insert(key.clone(), now);
        }
        true
    }
}

/// Compute deduplication keys from an event's content.
/// Returns a list of keys. An event is considered a duplicate if ANY of its keys
/// match a recently seen key. This allows cross-source deduplication:
/// e.g., a clipboard image event (with dimensions) and a screenshot file event
/// (with a file path) both produce a dimension-based key, so the second is suppressed.
fn compute_dedup_keys(event: &serde_json::Value) -> Vec<String> {
    let content_type = event
        .get("content_type")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    match content_type {
        "image" => {
            let mut keys = Vec::new();

            // Key based on image path (if available)
            if let Some(path) = event.get("image_path").and_then(|v| v.as_str()) {
                if !path.is_empty() {
                    keys.push(format!("img:path:{}", path));
                }
            }

            // Key based on image dimensions (for cross-source dedup between
            // clipboard images and screenshot files of the same capture)
            let w = event
                .get("image_width")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let h = event
                .get("image_height")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            if w > 0 && h > 0 {
                keys.push(format!("img:dims:{}x{}", w, h));
            }

            // Fallback: generic image key if no other keys produced
            if keys.is_empty() {
                keys.push("img:unknown".to_string());
            }
            keys
        }
        "text" => {
            if let Some(text) = event.get("raw_text").and_then(|v| v.as_str()) {
                // Use first 64 chars as dedup key for text
                let key_text: String = text.chars().take(64).collect();
                vec![format!("text:{}", key_text)]
            } else {
                vec!["text:empty".to_string()]
            }
        }
        _ => vec![format!("other:{}", content_type)],
    }
}

fn cleanup_event_pending_image(event: &serde_json::Value) {
    if let Some(path) = event.get("image_path").and_then(|value| value.as_str()) {
        if let Err(error) = super::image_lifecycle::cleanup_pending_image(path) {
            log::debug!("Pending image was not owned by OpenWiki: {}", error);
        }
    }
}

fn is_xiaoyun_source_app(source_app: &str) -> bool {
    source_app.eq_ignore_ascii_case("xiaoyun")
}

fn should_show_confirmation_bubble(event: &serde_json::Value) -> bool {
    let content_type = event
        .get("content_type")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    if content_type != "text" {
        return true;
    }

    let from_clipboard = event
        .get("from_clipboard")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let source_app = event
        .get("source_app")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    !from_clipboard || is_xiaoyun_source_app(source_app)
}

/// Async function to fetch URL content.
/// Uses smart routing: WeChat → direct HTML, Twitter → fxtwitter API, others → Jina.
async fn fetch_url_content(content_id: String, url: String, db: Arc<Database>, app: AppHandle) {
    log::info!("Starting URL fetch task for {} (url={})", content_id, url);

    let reader = UrlReader::with_app(app.clone());
    let locale = crate::locale::resolve_locale(&db);
    let result = reader.fetch_content(&url, &locale).await;

    match result {
        Ok(result) => {
            let db_for_summary = db.clone();
            let repo = Repository::new(db);
            if let Err(e) = repo.update_content_for_url(&content_id, &result.content, &url) {
                log::error!("Failed to update URL content: {}", e);
            } else {
                log::info!(
                    "URL content fetched for {}: {} chars (title={:?})",
                    content_id,
                    result.content.len(),
                    result.title
                );
                // Generate AI summary + tags after URL content is ready
                crate::commands::capture::spawn_summary_task(
                    db_for_summary,
                    app.clone(),
                    content_id.clone(),
                    result.content.clone(),
                );
                let _ = app.emit(
                    "content:url-fetched",
                    serde_json::json!({
                        "id": content_id,
                        "title": result.title,
                        "content_length": result.content.len(),
                    }),
                );
            }
        }
        Err(e) => {
            log::error!(
                "Failed to fetch URL content for {} (url={}): {}",
                content_id,
                url,
                e
            );
            // Mark as failed so UI stops showing "读取中" spinner
            let repo = Repository::new(db);
            let fail_msg = format!("[读取失败] {}\n\n原始链接: {}", e, url);
            let _ = repo.update_content_for_url(&content_id, &fail_msg, &url);
            let _ = app.emit(
                "content:url-fetched",
                serde_json::json!({
                    "id": content_id,
                    "failed": true,
                }),
            );
        }
    }
}

/// Parse JSON data into CaptureEvent and auto-save to database.
/// This is a module-level function (not nested) for better async task spawning.
fn handle_auto_save(app: &AppHandle, data: serde_json::Value) {
    let pending_image_path = data
        .get("image_path")
        .and_then(|value| value.as_str())
        .map(str::to_string);
    let db = {
        let state = app.state::<AppState>();
        state.db.clone()
    };

    // Check if sensitive data filtering is enabled
    let repo = Repository::new(db.clone());
    let sensitive_filter_enabled = repo
        .get_setting("sensitive_filter_enabled")
        .ok()
        .flatten()
        .map(|v| v == "true")
        .unwrap_or(false);

    // If filter is enabled, check text content for sensitive data
    if sensitive_filter_enabled {
        if let Some(text) = data.get("raw_text").and_then(|v| v.as_str()) {
            if contains_sensitive_data(text) {
                log::info!(
                    "Sensitive data detected, skipping capture (source_app={}, preview={}...)",
                    data.get("source_app")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Unknown"),
                    &text.chars().take(20).collect::<String>()
                );
                return;
            }
        }
    }

    let event = CaptureEvent {
        content_type: data
            .get("content_type")
            .and_then(|v| v.as_str())
            .unwrap_or("text")
            .to_string(),
        preview: data
            .get("preview")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        source_app: data
            .get("source_app")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown")
            .to_string(),
        raw_text: data
            .get("raw_text")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        image_path: data
            .get("image_path")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
    };

    match save_clipboard_content(app, &db, event) {
        Ok(content) => {
            log::info!(
                "Auto-saved: {} (type={}, source_url={:?})",
                content.id,
                content.content_type.as_str(),
                content.source_url
            );

            // For pure text content, generate AI summary immediately
            if content.content_type.as_str() == "text" {
                if let Some(ref text) = content.raw_text {
                    crate::commands::capture::spawn_summary_task(
                        db.clone(),
                        app.clone(),
                        content.id.clone(),
                        text.clone(),
                    );
                }
            }

            // For URL content, spawn background fetch via Jina Reader
            // Only fetch if raw_text is empty or equals source_url (not fetched yet)
            if content.content_type.as_str() == "url" {
                if let Some(url) = content.source_url {
                    // Check if content has been fetched already
                    let needs_fetch = content
                        .raw_text
                        .as_ref()
                        .map(|text| text.is_empty() || text.as_str() == url)
                        .unwrap_or(true);

                    if !needs_fetch {
                        log::info!("URL {} already fetched, skipping", url);
                        return;
                    }

                    // Check if URL reading is enabled (default: true)
                    let repo = Repository::new(db.clone());
                    let url_reading_enabled = repo
                        .get_setting("url_reading_enabled")
                        .ok()
                        .flatten()
                        .map(|v| v != "false")
                        .unwrap_or(true);

                    if url_reading_enabled {
                        log::info!("Spawning URL fetch task for {} (url={})", content.id, url);
                        let content_id = content.id.clone();
                        let app_clone = app.clone();
                        tauri::async_runtime::spawn(fetch_url_content(
                            content_id,
                            url,
                            db.clone(),
                            app_clone,
                        ));
                    } else {
                        log::info!("URL reading disabled, skipping fetch for {}", content.id);
                    }
                } else {
                    log::warn!("URL content {} has no source_url, cannot fetch", content.id);
                }
            }

            // For image content, auto-run OCR in background
            if content.content_type.as_str() == "image" {
                if let Some(image_path) = content.image_path {
                    let content_id = content.id.clone();
                    let db_clone = db.clone();
                    let app_clone = app.clone();
                    tauri::async_runtime::spawn(async move {
                        log::info!("[OCR] Auto-OCR starting for {}", content_id);
                        match tokio::task::spawn_blocking({
                            let path = image_path.clone();
                            move || super::ocr::recognize_text(&path)
                        })
                        .await
                        {
                            Ok(Ok(text)) => {
                                let db_for_summary = db_clone.clone();
                                let repo = Repository::new(db_clone);
                                if let Err(e) = repo.update_raw_text(&content_id, &text) {
                                    log::error!("[OCR] Failed to save: {}", e);
                                } else {
                                    log::info!(
                                        "[OCR] Auto-OCR done for {}: {} chars",
                                        content_id,
                                        text.len()
                                    );
                                    crate::commands::capture::spawn_summary_task(
                                        db_for_summary,
                                        app_clone.clone(),
                                        content_id.clone(),
                                        text.clone(),
                                    );
                                    let _ = app_clone.emit(
                                        "content:ocr-done",
                                        serde_json::json!({
                                            "id": content_id,
                                            "text_length": text.len(),
                                        }),
                                    );
                                }
                            }
                            Ok(Err(e)) => {
                                log::info!("[OCR] No text found in {}: {}", content_id, e);
                            }
                            Err(e) => {
                                log::error!("[OCR] Task failed for {}: {}", content_id, e);
                            }
                        }
                    });
                }
            }
        }
        Err(e) => {
            if e.contains("Duplicate content") {
                log::debug!("Duplicate content moved to top");
                // Notify frontend to refresh (order changed)
                let _ = app.emit(
                    "content:url-fetched",
                    serde_json::json!({"id": "", "reorder": true}),
                );
            } else {
                log::error!("Failed to auto-save content: {}", e);
                if let Some(path) = pending_image_path {
                    let _ = super::image_lifecycle::cleanup_pending_image(&path);
                }
            }
        }
    }
}

/// Numbers each pending capture, so a bubble can say which one it finished with.
static NEXT_PENDING_ID: AtomicU64 = AtomicU64::new(1);
/// A bubble show is already on its way; it will pick up the newest pending capture.
static BUBBLE_SHOW_SCHEDULED: AtomicBool = AtomicBool::new(false);

/// Store pending capture data in AppState for the bubble window to retrieve. Returns the data
/// stamped with its `pending_id`, which is what the bubble gets to see.
fn store_pending_capture(
    app: &AppHandle,
    data: &serde_json::Value,
    cleanup_replaced: bool,
) -> serde_json::Value {
    let mut data = data.clone();
    if let Some(fields) = data.as_object_mut() {
        fields.insert("pending_id".into(), NEXT_PENDING_ID.fetch_add(1, Ordering::Relaxed).into());
    }
    let pending_arc = app.state::<AppState>().pending_capture.clone();
    let guard = pending_arc.lock();
    if let Ok(mut pending) = guard {
        if cleanup_replaced {
            if let Some(previous) = pending.as_ref() {
                cleanup_event_pending_image(previous);
            }
        }
        *pending = Some(data.clone());
    }
    data
}

/// Make the window fully transparent on macOS (no opaque background).
/// Used for circle bubble mode so CSS can fully control the visual appearance
/// and animate from circle to capsule without native view interference.
/// IMPORTANT: All macOS window API calls MUST run on the main thread.
/// This function schedules the work on the main thread via dispatch crate.
#[cfg(target_os = "macos")]
fn make_window_transparent(win: &tauri::WebviewWindow) {
    use raw_window_handle::HasWindowHandle;

    let handle = match win.window_handle() {
        Ok(h) => h,
        Err(_) => return,
    };

    let appkit = match handle.as_raw() {
        raw_window_handle::RawWindowHandle::AppKit(h) => h,
        _ => return,
    };

    let ns_view_ptr = appkit.ns_view.as_ptr() as usize;

    // Execute synchronously — caller (show_bubble_window) already runs on main thread
    // via run_on_main_thread, so no dispatch needed. This ensures transparency is
    // fully applied BEFORE the window becomes visible.
    unsafe {
        use objc2::runtime::{AnyClass, AnyObject, Sel};

        let ns_view: &AnyObject = &*(ns_view_ptr as *const AnyObject);

        let ns_window: *const AnyObject = objc2::msg_send![ns_view, window];
        if ns_window.is_null() {
            return;
        }
        let ns_window: &AnyObject = &*ns_window;

        let _: () = objc2::msg_send![ns_window, setOpaque: false];
        let _: () = objc2::msg_send![ns_window, setHasShadow: false];

        let ns_color_cls = AnyClass::get("NSColor").unwrap();
        let clear_color: *const AnyObject = objc2::msg_send![ns_color_cls, clearColor];
        let _: () = objc2::msg_send![ns_window, setBackgroundColor: clear_color];

        let _: () = objc2::msg_send![ns_window, setAcceptsMouseMovedEvents: true];

        // Disable background drawing on WKWebView and subviews
        let content_view: *const AnyObject = objc2::msg_send![ns_window, contentView];
        if !content_view.is_null() {
            fn disable_bg(view: &objc2::runtime::AnyObject) {
                unsafe {
                    use objc2::runtime::{AnyObject, Sel};
                    let sel = Sel::register("_setDrawsBackground:");
                    let responds: bool = objc2::msg_send![view, respondsToSelector: sel];
                    if responds {
                        let _: () = objc2::msg_send![view, _setDrawsBackground: false];
                    }
                    let sel2 = Sel::register("setDrawsBackground:");
                    let responds2: bool = objc2::msg_send![view, respondsToSelector: sel2];
                    if responds2 {
                        let _: () = objc2::msg_send![view, setDrawsBackground: false];
                    }
                    let subviews: *const AnyObject = objc2::msg_send![view, subviews];
                    if !subviews.is_null() {
                        let count: usize = objc2::msg_send![subviews, count];
                        for i in 0..count {
                            let sub: *const AnyObject =
                                objc2::msg_send![subviews, objectAtIndex: i];
                            if !sub.is_null() {
                                disable_bg(&*sub);
                            }
                        }
                    }
                }
            }
            disable_bg(&*content_view);
        }
    }

    log::info!("Window transparency applied synchronously");
}

#[cfg(target_os = "macos")]
fn suppress_reopen_temporarily(app: &AppHandle, duration: Duration) {
    let suppress_arc = app.state::<AppState>().suppress_reopen_until.clone();
    if let Ok(mut guard) = suppress_arc.lock() {
        *guard = Some(Instant::now() + duration);
    };
}

/// Show the bubble window without stealing focus from the current app.
/// Strategy: record frontmost app before showing, then reactivate it after.
#[cfg(target_os = "macos")]
fn show_bubble_without_focus(win: &tauri::WebviewWindow) {
    use raw_window_handle::HasWindowHandle;

    let handle = match win.window_handle() {
        Ok(h) => h,
        Err(_) => return,
    };

    let appkit = match handle.as_raw() {
        raw_window_handle::RawWindowHandle::AppKit(h) => h,
        _ => return,
    };

    let ns_view_ptr = appkit.ns_view.as_ptr() as usize;

    dispatch::Queue::main().exec_async(move || {
        unsafe {
            use objc2::runtime::{AnyClass, AnyObject};

            // 1. Remember the currently active app BEFORE we show our window
            let workspace_cls = AnyClass::get("NSWorkspace").unwrap();
            let workspace: *const AnyObject = objc2::msg_send![workspace_cls, sharedWorkspace];
            let front_app: *const AnyObject = objc2::msg_send![&*workspace, frontmostApplication];

            // 2. Show bubble window as a non-activating panel
            let ns_view: &AnyObject = &*(ns_view_ptr as *const AnyObject);
            let ns_window: *const AnyObject = objc2::msg_send![ns_view, window];
            if !ns_window.is_null() {
                let ns_window: &AnyObject = &*ns_window;
                // Set window level to floating panel (above normal windows)
                let floating_level: i64 = 3; // NSFloatingWindowLevel
                let _: () = objc2::msg_send![ns_window, setLevel: floating_level];
                // Make the bubble visible over other apps' native fullscreen Spaces.
                // canJoinAllSpaces (1<<0) shows it in every Space, fullScreenAuxiliary
                // (1<<8) lets it sit over a fullscreen window, stationary (1<<4) keeps it
                // put when switching Spaces. Without this the floating level alone keeps
                // the bubble stuck in the desktop Space — invisible while a fullscreen
                // app is active, only reappearing after switching back to the desktop.
                let collection_behavior: u64 = (1u64 << 0) | (1u64 << 4) | (1u64 << 8);
                let _: () = objc2::msg_send![ns_window, setCollectionBehavior: collection_behavior];
                // Mark as non-activating panel — prevents focus steal entirely
                // NSWindowStyleMaskNonactivatingPanel = 1 << 7 = 128
                let style_mask: u64 = objc2::msg_send![ns_window, styleMask];
                let _: () = objc2::msg_send![ns_window, setStyleMask: style_mask | 128u64];
                let _: () = objc2::msg_send![ns_window, orderFrontRegardless];
            }

            // 3. Do not reactivate the previous app here.
            // `orderFrontRegardless` is enough to show the bubble, and explicitly
            // activating another app can wake or raise client windows unexpectedly.
            let _ = front_app;
        }
    });

    log::info!("Bubble window shown without stealing focus");
}

/// Find the monitor currently under the mouse cursor, so popup windows follow
/// the user across screens instead of always landing on the primary display.
/// Falls back to the primary monitor (then any monitor) if the cursor can't be resolved.
pub fn monitor_at_cursor(app: &AppHandle) -> Option<tauri::Monitor> {
    let from_cursor = app.cursor_position().ok().and_then(|cursor| {
        // `cursor_position()` is in physical pixels. On macOS it is scaled by
        // the primary monitor's factor while `monitor_from_point()` expects
        // logical points (CGDisplayBounds space), so convert back. On Windows
        // both are physical pixels in the virtual-screen space.
        #[cfg(target_os = "macos")]
        let (x, y) = {
            let scale = app
                .primary_monitor()
                .ok()
                .flatten()
                .map(|m| m.scale_factor())
                .unwrap_or(1.0);
            (cursor.x / scale, cursor.y / scale)
        };
        #[cfg(not(target_os = "macos"))]
        let (x, y) = (cursor.x, cursor.y);

        let found = app.monitor_from_point(x, y).ok().flatten();
        log::info!(
            "monitor_at_cursor: point=({:.0}, {:.0}) → monitor origin={:?}",
            x,
            y,
            found.as_ref().map(|m| m.position())
        );
        found
    });

    from_cursor
        .or_else(|| app.primary_monitor().ok().flatten())
        .or_else(|| {
            app.available_monitors()
                .ok()
                .and_then(|m| m.into_iter().next())
        })
}

/// Window label of the full-screen, click-through layer that draws the football game.
const FOOTBALL_FIELD_LABEL: &str = "football-field";
/// Initial size of the football input window; the field moves it onto the ball.
const FOOTBALL_BALL_WINDOW: f64 = 56.0;

fn set_football_layout(app: &AppHandle, layout: Option<serde_json::Value>) {
    let layout_arc = app.state::<AppState>().football_layout.clone();
    if let Ok(mut guard) = layout_arc.lock() {
        *guard = layout;
    };
}

fn destroy_football_field(app: &AppHandle) {
    if let Some(field) = app.get_webview_window(FOOTBALL_FIELD_LABEL) {
        let _ = field.destroy();
    }
}

/// How long a finished football field stays hidden before it is destroyed.
const FOOTBALL_TEARDOWN_DELAY: Duration = Duration::from_millis(600);

/// Takes a finished field off screen now and destroys it a moment later. Destroying a
/// webview that is still committing frames crashes WebKit on macOS (EXC_BAD_ACCESS in
/// RemoteLayerTreeDrawingAreaProxy::commitLayerTree), so it is hidden first and only
/// destroyed once it has gone quiet. The handle addresses this exact window, so a newer
/// field with the same label is never touched.
fn retire_football_field(field: &tauri::WebviewWindow) {
    let _ = field.hide();
    let field = field.clone();
    std::thread::spawn(move || {
        std::thread::sleep(FOOTBALL_TEARDOWN_DELAY);
        let _ = field.destroy();
    });
}

/// Waits (off the main thread) until a retiring field is gone, so a new football session
/// never collides with the old window's label.
fn wait_for_football_field_teardown(app: &AppHandle) {
    let deadline = Instant::now() + FOOTBALL_TEARDOWN_DELAY + Duration::from_millis(600);
    while app.get_webview_window(FOOTBALL_FIELD_LABEL).is_some() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Cursor position relative to `monitor`, in logical points
/// (same coordinate quirks as `monitor_at_cursor`).
fn cursor_in_monitor(app: &AppHandle, monitor: &tauri::Monitor) -> Option<(f64, f64)> {
    let cursor = app.cursor_position().ok()?;
    let scale = monitor.scale_factor();
    #[cfg(target_os = "macos")]
    {
        let primary_scale = app
            .primary_monitor()
            .ok()
            .flatten()
            .map(|m| m.scale_factor())
            .unwrap_or(1.0);
        let origin = monitor.position().to_logical::<f64>(scale);
        Some((cursor.x / primary_scale - origin.x, cursor.y / primary_scale - origin.y))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let origin = monitor.position();
        Some((
            (cursor.x - origin.x as f64) / scale,
            (cursor.y - origin.y as f64) / scale,
        ))
    }
}

/// Football bubble style: a full-screen transparent field window (click-through)
/// draws the ball, the goal and all animation, while the small hidden `bubble`
/// window is placed on the ball to receive clicks and drags.
/// Returns false if the windows could not be created.
fn show_football_windows(app: &AppHandle, bubble_position: &str) -> bool {
    use tauri::WebviewUrl;
    use tauri::WebviewWindowBuilder;

    let Some(monitor) = monitor_at_cursor(app) else {
        log::warn!("Football bubble: no monitor found");
        return false;
    };
    let scale = monitor.scale_factor();
    let origin = monitor.position().to_logical::<f64>(scale);
    let size = monitor.size().to_logical::<f64>(scale);
    let work = monitor.work_area();
    let work_pos = work.position.to_logical::<f64>(scale);
    let work_size = work.size.to_logical::<f64>(scale);
    // Tauri reports the macOS work area with the monitor's top edge, so split the
    // missing height ourselves: menu bar on top (at most ~38pt), Dock below.
    #[cfg(target_os = "macos")]
    let work_y = (size.height - work_size.height).clamp(0.0, 38.0);
    #[cfg(not(target_os = "macos"))]
    let work_y = work_pos.y - origin.y;

    let cursor = cursor_in_monitor(app, &monitor);
    let side = if bubble_position.contains("left") { "left" } else { "right" };
    let layout = serde_json::json!({
        "width": size.width,
        "height": size.height,
        "originX": origin.x,
        "originY": origin.y,
        "work": {
            "x": work_pos.x - origin.x,
            "y": work_y,
            "w": work_size.width,
            "h": work_size.height,
        },
        "cursor": cursor.map(|(x, y)| serde_json::json!({ "x": x, "y": y })),
        "side": side,
    });
    log::info!("Football layout: {}", layout);
    set_football_layout(app, Some(layout));

    // 1px shorter than the monitor so Windows doesn't treat it as a fullscreen app.
    let field = match WebviewWindowBuilder::new(
        app,
        FOOTBALL_FIELD_LABEL,
        WebviewUrl::App("/football-field".into()),
    )
    .title("")
    .inner_size(size.width, size.height - 1.0)
    .position(origin.x, origin.y)
    .resizable(false)
    .decorations(false)
    .transparent(true)
    .shadow(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .visible(false)
    .focused(false)
    .build()
    {
        Ok(win) => win,
        Err(e) => {
            log::error!("Failed to create football field window: {}", e);
            set_football_layout(app, None);
            return false;
        }
    };
    let _ = field.set_ignore_cursor_events(true);
    let app_for_field = app.clone();
    field.on_window_event(move |event| {
        if let tauri::WindowEvent::Destroyed = event {
            // Removing our last visible window can make macOS fire Reopen.
            let suppress_arc = app_for_field.state::<AppState>().suppress_reopen_until.clone();
            if let Ok(mut guard) = suppress_arc.lock() {
                *guard = Some(Instant::now() + Duration::from_secs(2));
            };
        }
    });

    let ball = match WebviewWindowBuilder::new(app, "bubble", WebviewUrl::App("/bubble".into()))
        .title("")
        .inner_size(FOOTBALL_BALL_WINDOW, FOOTBALL_BALL_WINDOW)
        .position(origin.x, origin.y)
        .resizable(false)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .focused(false)
        .accept_first_mouse(true)
        .build()
    {
        Ok(win) => win,
        Err(e) => {
            log::error!("Failed to create football ball window: {}", e);
            let _ = field.destroy();
            set_football_layout(app, None);
            return false;
        }
    };
    let app_for_ball = app.clone();
    let field_for_ball = field.clone();
    ball.on_window_event(move |event| match event {
        tauri::WindowEvent::CloseRequested { .. } => {
            let suppress_arc = app_for_ball.state::<AppState>().suppress_reopen_until.clone();
            if let Ok(mut guard) = suppress_arc.lock() {
                *guard = Some(Instant::now() + Duration::from_secs(2));
            };
        }
        // The field belongs to this capture: never leave it behind.
        tauri::WindowEvent::Destroyed => retire_football_field(&field_for_ball),
        _ => {}
    });

    #[cfg(target_os = "macos")]
    {
        suppress_reopen_temporarily(app, Duration::from_secs(5));
        make_window_transparent(&field);
        make_window_transparent(&ball);
        show_bubble_without_focus(&field);
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = field.show();
    }
    let _ = field.set_ignore_cursor_events(true);
    log::info!("Football bubble windows created (side={})", side);
    true
}

/// Moves the hidden football input window onto the ball (field coordinates) and
/// shows it without stealing focus. Called by the field once the ball is drawn.
pub fn show_football_ball_window(app: &AppHandle, x: f64, y: f64, size: f64) -> Result<(), String> {
    let (origin_x, origin_y) = {
        let layout_arc = app.state::<AppState>().football_layout.clone();
        let guard = layout_arc.lock().map_err(|e| format!("Lock error: {}", e))?;
        let layout = guard.as_ref().ok_or("No football session")?;
        (
            layout.get("originX").and_then(|v| v.as_f64()).unwrap_or(0.0),
            layout.get("originY").and_then(|v| v.as_f64()).unwrap_or(0.0),
        )
    };
    let win = app
        .get_webview_window("bubble")
        .ok_or("Football ball window missing")?;
    let size = size.clamp(24.0, 160.0);
    win.set_size(tauri::LogicalSize::new(size, size))
        .map_err(|e| e.to_string())?;
    win.set_position(tauri::LogicalPosition::new(
        origin_x + x - size / 2.0,
        origin_y + y - size / 2.0,
    ))
    .map_err(|e| e.to_string())?;
    #[cfg(target_os = "macos")]
    show_bubble_without_focus(&win);
    #[cfg(not(target_os = "macos"))]
    win.show().map_err(|e| e.to_string())?;
    Ok(())
}

/// Shows the bubble for the stored pending capture a moment later, once the previous bubble and
/// its football field are gone (macOS window APIs must then run on the main thread). Only one
/// show is scheduled at a time, and the bubble picks up whichever capture is newest by then.
pub fn schedule_bubble_window(app: &AppHandle) {
    if BUBBLE_SHOW_SCHEDULED.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(150));
        let deadline = Instant::now() + Duration::from_secs(2);
        while app.get_webview_window("bubble").is_some() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        wait_for_football_field_teardown(&app);
        let app_main = app.clone();
        let queued = app.run_on_main_thread(move || {
            BUBBLE_SHOW_SCHEDULED.store(false, Ordering::SeqCst);
            let waiting = app_main
                .state::<AppState>()
                .pending_capture
                .lock()
                .map(|pending| pending.is_some())
                .unwrap_or(false);
            if waiting {
                show_bubble_window(&app_main);
            }
        });
        if queued.is_err() {
            BUBBLE_SHOW_SCHEDULED.store(false, Ordering::SeqCst);
        }
    });
}

/// Dynamically create and show the bubble window at the bottom-right of the screen.
/// If a bubble window already exists, close it first to avoid duplicates.
fn show_bubble_window(app: &AppHandle) {
    use tauri::WebviewUrl;
    use tauri::WebviewWindowBuilder;

    // Close existing bubble if any, with retry to ensure it's fully closed
    if let Some(existing) = app.get_webview_window("bubble") {
        let _ = existing.destroy();
        // Wait and verify the window is actually gone
        for _ in 0..10 {
            std::thread::sleep(std::time::Duration::from_millis(30));
            if app.get_webview_window("bubble").is_none() {
                break;
            }
        }
        // If still exists, skip creating new one
        if app.get_webview_window("bubble").is_some() {
            log::warn!("Bubble window still exists after destroy, skipping creation");
            return;
        }
    }

    destroy_football_field(app);

    // Read bubble style and position from settings
    let (bubble_style, bubble_position) = {
        let state = app.state::<AppState>();
        let repo = Repository::new(state.db.clone());
        let style = repo
            .get_setting("bubble_style")
            .ok()
            .flatten()
            .unwrap_or_else(|| "circle".to_string());
        let position = repo
            .get_setting("bubble_position")
            .ok()
            .flatten()
            .unwrap_or_else(|| "bottom-right".to_string());
        (style, position)
    };

    if bubble_style == "football" && show_football_windows(app, &bubble_position) {
        return;
    }
    set_football_layout(app, None);

    // The football style falls back to the circle bubble if its windows can't be created.
    let is_circle = bubble_style != "bar";
    // Circle mode: 64px height (48px circle + 16px padding for bounce animation).
    // Keep the collapsed circle window physically tight so transparent bounds
    // do not intercept clicks behind the visible 48px bubble.
    let circle_win_w: f64 = 64.0;
    let win_w: f64 = if is_circle { circle_win_w } else { 340.0 };
    let win_h: f64 = if is_circle { 64.0 } else { 72.0 };

    // Determine position based on bubble_position setting, on the monitor
    // under the mouse cursor (multi-screen: follow the user, not the primary).
    let (x, y) = if let Some(monitor) = monitor_at_cursor(app) {
        let screen = monitor.size();
        let scale = monitor.scale_factor();
        // Monitor origin in logical (global) coordinates — (0,0) for the
        // primary, offset for secondary screens.
        let origin = monitor.position().to_logical::<f64>(scale);
        let screen_w = screen.width as f64 / scale;
        let screen_h = screen.height as f64 / scale;
        let margin = 20.0;
        let menu_bar_h = 30.0; // macOS menu bar height

        let x = match bubble_position.as_str() {
            "bottom-left" | "top-left" => margin,
            "bottom-center" | "top-center" => {
                if is_circle {
                    // Center: align the 48px circle visually at center
                    (screen_w - 48.0) / 2.0 - (win_w - 48.0) / 2.0
                } else {
                    (screen_w - win_w) / 2.0
                }
            }
            _ => screen_w - win_w - margin, // bottom-right, top-right, default
        };

        let y = match bubble_position.as_str() {
            "top-left" | "top-center" | "top-right" => margin + menu_bar_h,
            _ => screen_h - win_h - margin - 60.0, // bottom-*, default (60 for dock)
        };

        let x = origin.x + x;
        let y = origin.y + y;

        log::info!(
            "Bubble position: ({}, {}), monitor origin=({}, {}), style={}, pos={}, size={}x{}",
            x,
            y,
            origin.x,
            origin.y,
            bubble_style,
            bubble_position,
            win_w,
            win_h
        );
        (x, y)
    } else {
        (500.0, 500.0)
    };

    // Create the bubble window dynamically
    match WebviewWindowBuilder::new(app, "bubble", WebviewUrl::App("/bubble".into()))
        .title("")
        .inner_size(win_w, win_h)
        .position(x, y)
        .resizable(false)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .focused(false)
        .accept_first_mouse(true)
        .build()
    {
        Ok(win) => {
            let app_handle = app.clone();
            win.on_window_event(move |event| {
                if let tauri::WindowEvent::CloseRequested { .. } = event {
                    let suppress_arc = app_handle.state::<AppState>().suppress_reopen_until.clone();
                    if let Ok(mut guard) = suppress_arc.lock() {
                        *guard = Some(Instant::now() + Duration::from_secs(2));
                    };
                }
            });

            #[cfg(target_os = "macos")]
            {
                suppress_reopen_temporarily(app, Duration::from_secs(5));

                if is_circle {
                    // Circle mode: apply transparency BEFORE showing window
                    make_window_transparent(&win);
                } else {
                    // Bar mode: use native vibrancy for glass background
                    use window_vibrancy::{apply_vibrancy, NSVisualEffectMaterial};
                    let _ =
                        apply_vibrancy(&win, NSVisualEffectMaterial::HudWindow, None, Some(16.0));
                }
                // Show the window without stealing focus from the current app
                show_bubble_without_focus(&win);
            }
            #[cfg(not(target_os = "macos"))]
            {
                let _ = win.show();
            }
            log::info!("Bubble window created (style={})", bubble_style);
        }
        Err(e) => {
            log::error!("Failed to create bubble window: {}", e);
        }
    }
}

pub struct CaptureDetector {
    clipboard_watcher: ClipboardWatcher,
    screenshot_watcher: ScreenshotWatcher,
}

impl CaptureDetector {
    pub fn new() -> Self {
        CaptureDetector {
            clipboard_watcher: ClipboardWatcher::new(),
            screenshot_watcher: ScreenshotWatcher::new(),
        }
    }

    pub fn start(&self, app: AppHandle) {
        log::info!("Starting capture detector with auto-save...");

        let dedup_state = Arc::new(Mutex::new(DeduplicationState::new()));

        // Listen to clipboard events, dedup, then auto-save
        let app_for_clipboard = app.clone();
        let dedup_for_clipboard = dedup_state.clone();
        app.listen("capture:clipboard", move |event| {
            if let Ok(data) = serde_json::from_str::<serde_json::Value>(event.payload()) {
                let keys = compute_dedup_keys(&data);
                let should_save = {
                    let mut state = dedup_for_clipboard.lock().unwrap_or_else(|e| e.into_inner());
                    state.should_emit(&keys)
                };

                if should_save {
                    // Check capture mode: "confirm" requires user to click floating bubble
                    let capture_mode = {
                        let state = app_for_clipboard.state::<AppState>();
                        let repo = Repository::new(state.db.clone());
                        repo.get_setting("capture_mode")
                            .ok()
                            .flatten()
                            .unwrap_or_else(|| "confirm".to_string())
                    };

                    if capture_mode == "confirm" {
                        let bubble_exists = app_for_clipboard.get_webview_window("bubble").is_some();
                        // Store pending data in AppState so BubbleView can retrieve it
                        let data = store_pending_capture(&app_for_clipboard, &data, !bubble_exists);

                        // If bubble already open, just emit event — let frontend decide
                        // whether to accept (circle mode) or ignore (expanded mode). A football
                        // ball on its way out ignores it and dismisses with its own pending_id,
                        // so this capture stays stored and gets a bubble of its own afterwards.
                        if bubble_exists {
                            log::info!("Bubble window already open, emitting capture:pending for frontend to handle");
                            if let Err(e) = app_for_clipboard.emit("capture:pending", &data) {
                                log::error!("Failed to emit capture:pending: {}", e);
                            }
                            return;
                        }

                        // Emit event for the BubbleView listener
                        log::info!("Capture pending confirmation (mode=confirm)");
                        if let Err(e) = app_for_clipboard.emit("capture:pending", &data) {
                            log::error!("Failed to emit capture:pending: {}", e);
                        }

                        schedule_bubble_window(&app_for_clipboard);
                    } else {
                        handle_auto_save(&app_for_clipboard, data);
                    }
                } else {
                    cleanup_event_pending_image(&data);
                }
            }
        });

        // Listen to screenshot events, dedup, then auto-save
        let app_for_screenshot = app.clone();
        let dedup_for_screenshot = dedup_state;
        app.listen("capture:screenshot", move |event| {
            if let Ok(data) = serde_json::from_str::<serde_json::Value>(event.payload()) {
                let keys = compute_dedup_keys(&data);
                let should_save = {
                    let mut state = dedup_for_screenshot
                        .lock()
                        .unwrap_or_else(|e| e.into_inner());
                    state.should_emit(&keys)
                };

                if should_save {
                    // Screenshots always auto-save (no floating bubble needed)
                    handle_auto_save(&app_for_screenshot, data);
                } else {
                    cleanup_event_pending_image(&data);
                }
            }
        });

        // Start the underlying watchers (they emit to capture:clipboard / capture:screenshot)
        self.clipboard_watcher.start(app.clone());
        self.screenshot_watcher.start(app);

        log::info!("Capture detector started with auto-save enabled");
    }

    pub fn stop(&self) {
        log::info!("Stopping capture detector...");
        self.clipboard_watcher.stop();
        self.screenshot_watcher.stop();
        log::info!("Capture detector stopped");
    }
}

#[cfg(test)]
mod tests {
    use super::should_show_confirmation_bubble;

    #[test]
    fn bubble_shows_for_non_text_content() {
        let event = serde_json::json!({
            "content_type": "image",
            "source_app": "WeChat",
            "from_clipboard": true
        });
        assert!(should_show_confirmation_bubble(&event));
    }

    #[test]
    fn bubble_is_suppressed_for_external_clipboard_text() {
        let event = serde_json::json!({
            "content_type": "text",
            "source_app": "WeChat",
            "from_clipboard": true
        });
        assert!(!should_show_confirmation_bubble(&event));
    }

    #[test]
    fn bubble_is_allowed_for_xiaoyun_clipboard_text() {
        let event = serde_json::json!({
            "content_type": "text",
            "source_app": "xiaoyun",
            "from_clipboard": true
        });
        assert!(should_show_confirmation_bubble(&event));
    }
}
