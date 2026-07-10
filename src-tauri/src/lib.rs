mod ai;
mod automation;
mod capture;
mod commands;
mod export;
pub mod locale;
mod scheduler;
mod secure_store;
mod storage;
mod update;

use capture::detector::CaptureDetector;
use commands::capture::AppState;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::Manager;
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};

const AUTOSTART_DEFAULT_APPLIED_KEY: &str = "autostart_default_applied";

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let db = Arc::new(storage::database::Database::new().expect("Failed to initialize database"));

    let detector = CaptureDetector::new();

    tauri::Builder::default()
        .plugin(
            tauri_plugin_log::Builder::default()
                .level(log::LevelFilter::Info)
                .targets([
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::LogDir {
                        file_name: Some("openwiki".into()),
                    }),
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Stdout),
                ])
                .build(),
        )
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_shortcuts(["CmdOrCtrl+Shift+Y", "CmdOrCtrl+Shift+C"])
                .expect("Failed to parse shortcuts")
                .with_handler(|app, shortcut, event| {
                    if let tauri_plugin_global_shortcut::ShortcutState::Pressed = event.state {
                        let key = shortcut.key;
                        use tauri_plugin_global_shortcut::Code;
                        match key {
                            Code::KeyY => {
                                show_main_window(app, None);
                            }
                            Code::KeyC => {
                                trigger_spotlight_capture(app);
                            }
                            _ => {}
                        }
                    }
                })
                .build(),
        )
        .manage(AppState {
            db,
            pending_capture: std::sync::Arc::new(std::sync::Mutex::new(None)),
            suppress_reopen_until: std::sync::Arc::new(std::sync::Mutex::new(None)),
        })
        .setup(move |app| {
            eprintln!("[openwiki] App setup started");

            // --- Resolve bundled OCR helper binary ---
            // The Swift OCR helper is pre-compiled at build time and shipped
            // as a Tauri resource, so end users don't need Xcode Command Line Tools.
            //
            #[cfg(target_os = "macos")]
            if let Ok(resource_dir) = app.path().resource_dir() {
                let ocr_bin = resource_dir.join("openwiki_ocr_bin");
                log::info!("[OCR] Registered bundled helper at {}", ocr_bin.display());
                crate::capture::ocr::init_ocr_binary_path(ocr_bin);
            }
            #[cfg(target_os = "windows")]
            log::info!("[OCR] Windows OCR uses the built-in Windows Runtime OCR engine");
            #[cfg(not(any(target_os = "macos", target_os = "windows")))]
            log::info!("[OCR] OCR is disabled on this platform");

            // --- Apply native spotlight styling + auto-hide on blur ---
            if let Some(spotlight_win) = app.get_webview_window("spotlight") {
                #[cfg(target_os = "macos")]
                {
                use window_vibrancy::{apply_vibrancy, NSVisualEffectMaterial};
                let _ = apply_vibrancy(
                    &spotlight_win,
                    NSVisualEffectMaterial::HudWindow,
                    None,
                    Some(22.0),
                );
                }

                // Hide spotlight when it loses focus (user clicked elsewhere).
                // This is more reliable than JS onFocusChanged which can stop
                // firing after repeated show/hide cycles.
                let win_clone = spotlight_win.clone();
                let app_handle = app.handle().clone();
                spotlight_win.on_window_event(move |event| {
                    if let tauri::WindowEvent::Focused(false) = event {
                        suppress_reopen(&app_handle, Duration::from_secs(2));
                        let _ = win_clone.hide();
                    }
                });
            }

            // --- Intercept window close: hide instead of destroy ---
            if let Some(main_win) = app.get_webview_window("main") {
                let win_clone = main_win.clone();
                main_win.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        // Prevent actual close, just hide the window
                        api.prevent_close();
                        let _ = win_clone.hide();
                    }
                });
            }

            // --- System Tray ---
            setup_tray(app)?;

            // --- Launch at startup default ---
            apply_default_autostart_once(app);

            // --- Cleanup stale compile locks from interrupted sessions ---
            {
                let state: tauri::State<'_, AppState> = app.state();
                let repo = crate::storage::repository::Repository::new(state.db.clone());
                match repo.cleanup_stale_compile_locks() {
                    Ok(n) if n > 0 => log::info!("Cleaned {} stale compile locks", n),
                    _ => {}
                }

                // Clean up legacy "source deleted" lint notifications from before
                // we stopped auto-generating them. One-time cleanup — safe to run
                // on every startup since it only touches open "orphan" lints.
                match repo.resolve_lint_results_by_type("orphan") {
                    Ok(n) if n > 0 => log::info!("Cleaned {} legacy orphan lint notifications", n),
                    _ => {}
                }

                // Wipe the old tag-based "related" edges. The old algorithm
                // connected any two pages sharing a single tag, which exploded
                // the graph into a nearly-complete mess (988 pairs over 151
                // pages). The new TF-IDF + cosine-similarity algorithm will
                // regenerate them in the background task below. One-time safe
                // migration: deleting edges never loses source data, and the
                // rebuild task immediately repopulates the graph with meaningful
                // connections.
                match repo.delete_edges_by_relation("related") {
                    Ok(n) if n > 0 => log::info!(
                        "Cleared {} legacy 'related' edges, scheduling graph rebuild",
                        n
                    ),
                    _ => {}
                }
            }

            // --- Rebuild the wiki graph with the new TF-IDF algorithm ---
            // Runs 5 seconds after startup so the main UI is interactive first.
            // Non-blocking: failures log a warning but don't affect anything else.
            {
                use tauri::Emitter;
                let state: tauri::State<'_, AppState> = app.state();
                let db = state.db.clone();
                let app_handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    match crate::ai::wiki_engine::link_pages_by_shared_tags(db) {
                        Ok(count) => {
                            log::info!("Wiki graph rebuilt: {} edges", count);
                            let _ = app_handle.emit("wiki-graph-rebuilt", count);
                        }
                        Err(e) => {
                            log::warn!("Wiki graph rebuild failed: {}", e);
                        }
                    }
                });
            }

            // --- Start capture detector (auto-saves to database) ---
            eprintln!("[openwiki] Starting capture detector...");
            detector.start(app.handle().clone());
            eprintln!("[openwiki] Capture detector started!");

            // --- Background update check (GitHub Releases polling) ---
            // Runs 3s after startup, emits `update-available` if a newer version
            // is published. Every failure mode is swallowed to log::warn — never
            // surfaces to the user.
            {
                let state: tauri::State<'_, AppState> = app.state();
                crate::update::spawn_background_check(app.handle().clone(), state.db.clone());
            }

            // --- Automation (Apple Events) permission guard ---
            // On first launch emits `automation-needed` so the frontend can show
            // the pre-auth modal. On subsequent launches probes the current
            // status and emits `automation-denied` if revoked, so the banner
            // can surface a fix-it button.
            {
                let state: tauri::State<'_, AppState> = app.state();
                crate::automation::spawn_startup_check(app.handle().clone(), state.db.clone());
            }

            // --- Insight report auto-scheduler ---
            // Auto-generates the first attention report once enough content is
            // saved, then refreshes weekly when there's new content. Reuses the
            // same analysis routine as the manual button; all failures logged only.
            {
                let state: tauri::State<'_, AppState> = app.state();
                crate::scheduler::weekly::spawn_insight_scheduler(
                    app.handle().clone(),
                    state.db.clone(),
                );
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::capture::save_captured_content,
            commands::capture::save_spotlight_content,
            commands::capture::import_markdown_files,
            commands::capture::import_content_files,
            commands::capture::import_urls,
            commands::capture::confirm_capture,
            commands::capture::dismiss_capture,
            commands::capture::get_pending_capture,
            commands::capture::debug_log,
            commands::capture::retry_url_fetch,
            commands::capture::ocr_image,
            commands::capture::get_contents_by_ids,
            commands::capture::test_ai_connection,
            commands::storage::get_all_content,
            commands::storage::delete_content,
            commands::report::generate_report,
            commands::report::get_report,
            commands::report::get_all_reports,
            commands::report::submit_feedback,
            commands::preferences::get_settings,
            commands::preferences::update_setting,
            commands::preferences::check_xreader_status,
            commands::digest::get_digest_items,
            commands::digest::digest_item,
            commands::mcp::get_mcp_status,
            commands::mcp::connect_mcp,
            commands::mcp::disconnect_mcp,
            commands::mcp::copy_content_summary,
            commands::datahub::search_content,
            commands::datahub::get_dates_with_content,
            commands::datahub::get_content_for_date,
            commands::datahub::export_day_markdown,
            commands::datahub::export_all_markdown,
            commands::datahub::export_date_range_markdown,
            commands::datahub::get_export_dir,
            commands::datahub::set_export_dir,
            commands::datahub::open_export_dir,
            commands::datahub::get_storage_info,
            commands::datahub::export_all_single,
            commands::datahub::export_all_single_quiet,
            commands::datahub::export_range_single,
            commands::datahub::export_current_radar_report,
            commands::datahub::open_data_folder,
            commands::attention::get_attention_insights,
            commands::attention::trigger_attention_analysis,
            commands::oauth::start_openai_oauth,
            commands::oauth::get_openai_oauth_status,
            commands::oauth::logout_openai_oauth,
            commands::oauth::start_gemini_oauth,
            commands::oauth::get_gemini_oauth_status,
            commands::oauth::logout_gemini_oauth,
            commands::wiki::get_wiki_pages,
            commands::wiki::get_wiki_page,
            commands::wiki::search_wiki,
            commands::wiki::get_wiki_stats,
            commands::wiki::delete_wiki_page,
            commands::wiki::get_wiki_graph,
            commands::wiki::compile_content_to_wiki,
            commands::wiki::wiki_ask,
            commands::wiki::get_chat_sessions,
            commands::wiki::get_chat_messages,
            commands::wiki::delete_chat_session,
            commands::wiki::save_message_as_page,
            commands::wiki::get_saved_message_ids,
            commands::wiki::get_wiki_conversations,
            commands::wiki::wiki_link_by_tags,
            commands::wiki::trigger_wiki_lint,
            commands::wiki::get_wiki_lint_results,
            commands::wiki::wiki_lint_keep,
            commands::wiki::wiki_lint_delete,
            commands::wiki::wiki_lint_recompile,
            commands::wiki::get_page_sources,
            commands::wiki::get_content_wiki_pages,
            update::check_for_update_manual,
            update::set_update_check_enabled,
            update::get_update_settings,
            automation::get_automation_status,
            automation::request_automation_permission,
            automation::dismiss_automation_prompt,
            automation::open_automation_settings,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            #[cfg(not(target_os = "macos"))]
            let _ = (app, event);

            #[cfg(target_os = "macos")]
            {
            // Handle Dock icon click on macOS: show main window only if it's hidden.
            // When bubble closes, macOS fires Reopen because no visible windows remain.
            // We only respond if the main window is actually hidden (user closed it),
            // not when it's just behind other windows.
            if let tauri::RunEvent::Reopen { .. } = event {
                let reopen_suppressed = is_reopen_suppressed(app);
                let main_hidden = app
                    .get_webview_window("main")
                    .and_then(|win| win.is_visible().ok())
                    .map(|visible| !visible)
                    .unwrap_or(true);

                if should_show_main_on_reopen(main_hidden, reopen_suppressed) {
                    show_main_window(app, None);
                }
            }
            }
        });
}

/// Triggered by Cmd+Shift+C.
/// 1. Simulate Cmd+C to copy the user's current selection
/// 2. Wait a moment for the clipboard to update
/// 3. Read clipboard content
/// 4. Send it to the spotlight window via event
/// 5. Show the spotlight window
fn trigger_spotlight_capture(app: &tauri::AppHandle) {
    use tauri::Emitter;

    let app_clone = app.clone();
    std::thread::spawn(move || {
        simulate_copy_shortcut();

        // Step 2: Wait for clipboard to update
        std::thread::sleep(std::time::Duration::from_millis(150));

        // Step 3: Read clipboard
        let (content_type, raw_text, image_path) = read_clipboard_content();

        // Step 4: Detect source app
        let source_app = detect_frontmost_app();

        // Step 5: Emit content to spotlight window
        let payload = serde_json::json!({
            "content_type": content_type,
            "raw_text": raw_text,
            "image_path": image_path,
            "source_app": source_app,
        });
        let _ = app_clone.emit("spotlight:content-ready", payload);

        // Step 6: Show spotlight window
        if let Some(win) = app_clone.get_webview_window("spotlight") {
            let _ = win.center();
            let _ = win.show();
            let _ = win.set_focus();
        }
    });
}

fn apply_default_autostart_once(app: &mut tauri::App) {
    let state: tauri::State<'_, AppState> = app.state();
    let repo = crate::storage::repository::Repository::new(state.db.clone());

    match repo.get_setting(AUTOSTART_DEFAULT_APPLIED_KEY) {
        Ok(Some(value)) if value == "true" => return,
        Ok(_) => {}
        Err(e) => {
            log::warn!("[autostart] failed to read default marker: {}", e);
            return;
        }
    }

    match app.autolaunch().is_enabled() {
        Ok(true) => {
            log::info!("[autostart] already enabled; marking default as applied");
        }
        Ok(false) => {
            if let Err(e) = app.autolaunch().enable() {
                log::warn!(
                    "[autostart] failed to enable default launch at startup: {}",
                    e
                );
                return;
            }
            log::info!("[autostart] enabled launch at startup by default");
        }
        Err(e) => {
            log::warn!("[autostart] failed to read current status: {}", e);
            return;
        }
    }

    if let Err(e) = repo.update_setting(AUTOSTART_DEFAULT_APPLIED_KEY, "true") {
        log::warn!("[autostart] failed to persist default marker: {}", e);
    }
}

/// Read current clipboard content (text or image).
fn read_clipboard_content() -> (String, Option<String>, Option<String>) {
    if let Ok(mut clipboard) = arboard::Clipboard::new() {
        // Try text first
        if let Ok(text) = clipboard.get_text() {
            if !text.is_empty() {
                // Check if it's a URL
                if text.trim().starts_with("http://") || text.trim().starts_with("https://") {
                    return ("url".to_string(), Some(text), None);
                }
                return ("text".to_string(), Some(text), None);
            }
        }

        // Try image
        if let Ok(img) = clipboard.get_image() {
            // Save image to disk
            if let Some(path) = save_clipboard_image(&img) {
                return ("image".to_string(), None, Some(path));
            }
        }
    }

    ("text".to_string(), None, None)
}

/// Save clipboard image pixels to a temporary PNG file.
fn save_clipboard_image(img: &arboard::ImageData) -> Option<String> {
    let base = dirs::data_dir()
        .or_else(|| dirs::home_dir().map(|h| h.join("Library").join("Application Support")))?;

    let captures_dir = base.join("com.openwiki.app").join("captures");
    let _ = std::fs::create_dir_all(&captures_dir);

    let id = uuid::Uuid::new_v4().to_string();
    let file_path = captures_dir.join(format!("{}.png", id));

    let rgba_buf =
        image::RgbaImage::from_raw(img.width as u32, img.height as u32, img.bytes.to_vec())?;

    if rgba_buf.save(&file_path).is_ok() {
        Some(file_path.to_string_lossy().to_string())
    } else {
        None
    }
}

fn simulate_copy_shortcut() {
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("osascript")
            .args([
                "-e",
                r#"tell application "System Events" to keystroke "c" using command down"#,
            ])
            .output();
    }

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;

        let mut command = std::process::Command::new("powershell");
        command
            .creation_flags(CREATE_NO_WINDOW)
            .args([
            "-NoProfile",
            "-Command",
            "Add-Type -AssemblyName System.Windows.Forms; [System.Windows.Forms.SendKeys]::SendWait('^c')",
        ]);
        let _ = command.output();
    }
}

/// Detect the frontmost application on macOS.
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
        _ => "Unknown".to_string(),
    }
    }

    #[cfg(target_os = "windows")]
    {
        detect_frontmost_window_title()
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        "Unknown".to_string()
    }
}

#[cfg(target_os = "windows")]
fn detect_frontmost_window_title() -> String {
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
        if title.is_empty() { "Unknown".to_string() } else { title }
    }
}

fn suppress_reopen(app: &tauri::AppHandle, duration: Duration) {
    let suppress_arc = app.state::<AppState>().suppress_reopen_until.clone();
    if let Ok(mut guard) = suppress_arc.lock() {
        *guard = Some(Instant::now() + duration);
    };
}

#[cfg(target_os = "macos")]
fn is_reopen_suppressed(app: &tauri::AppHandle) -> bool {
    let suppress_arc = app.state::<AppState>().suppress_reopen_until.clone();
    let Ok(mut guard) = suppress_arc.lock() else {
        return false;
    };

    match *guard {
        Some(until) if until > Instant::now() => true,
        Some(_) => {
            *guard = None;
            false
        }
        None => false,
    }
}

#[cfg_attr(not(test), allow(dead_code))]
fn should_show_main_on_reopen(main_hidden: bool, reopen_suppressed: bool) -> bool {
    main_hidden && !reopen_suppressed
}

fn setup_tray(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    use tauri::menu::{Menu, MenuItem};
    use tauri::tray::TrayIconBuilder;

    let show = MenuItem::with_id(app, "show", "Open OpenWiki", true, None::<&str>)?;
    let report = MenuItem::with_id(app, "report", "Generate Report", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;

    let menu = Menu::with_items(app, &[&show, &report, &quit])?;

    TrayIconBuilder::new()
        .icon(app.default_window_icon().unwrap().clone())
        .tooltip("OpenWiki")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => {
                show_main_window(app, None);
            }
            "report" => {
                show_main_window(app, Some("digest"));
            }
            "quit" => {
                app.exit(0);
            }
            _ => {}
        })
        .build(app)?;

    Ok(())
}

fn show_main_window(app: &tauri::AppHandle, tab: Option<&str>) {
    use tauri::Emitter;

    if let Some(win) = app.get_webview_window("main") {
        let _ = win.show();
        let _ = win.set_focus();
        if let Some(tab) = tab {
            let _ = app.emit("navigate-tab", tab);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::should_show_main_on_reopen;

    #[test]
    fn reopen_shows_main_when_hidden_and_not_suppressed() {
        assert!(should_show_main_on_reopen(true, false));
    }

    #[test]
    fn reopen_does_not_show_main_when_suppressed() {
        assert!(!should_show_main_on_reopen(true, true));
    }

    #[test]
    fn reopen_does_not_show_main_when_already_visible() {
        assert!(!should_show_main_on_reopen(false, false));
    }
}
