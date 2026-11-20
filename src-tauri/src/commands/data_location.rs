//! Settings → Storage: show where the data lives and move it to another folder (issue #17).

use crate::commands::capture::AppState;
use crate::storage::data_location::{self as location, DataLocationInfo};
use serde::Serialize;
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_dialog::DialogExt;

fn blocking_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[tauri::command]
pub async fn get_data_location() -> Result<DataLocationInfo, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let default = location::default_dir()?;
        let current = location::data_dir()?;
        Ok(location::info_in(&default, &current))
    })
    .await
    .map_err(blocking_error)?
}

#[derive(Serialize)]
pub struct PickedDataFolder {
    /// The folder the user picked; `move_data_dir` takes it back.
    picked: String,
    /// Where the data would actually go (the folder itself, or an `OpenWiki` folder inside it).
    target: String,
}

/// Let the user pick a folder and say where the data would go. Nothing is copied yet.
#[tauri::command]
pub async fn pick_data_folder(app: AppHandle) -> Result<Option<PickedDataFolder>, String> {
    let picked =
        tauri::async_runtime::spawn_blocking(move || app.dialog().file().blocking_pick_folder())
            .await
            .map_err(blocking_error)?;
    let Some(picked) = picked else {
        return Ok(None);
    };
    let picked = picked.into_path().map_err(blocking_error)?;
    let target = location::target_for(&picked, &location::data_dir()?)?;
    Ok(Some(PickedDataFolder {
        picked: picked.to_string_lossy().into_owned(),
        target: target.to_string_lossy().into_owned(),
    }))
}

/// Copy everything to the picked folder, switch to it and restart. Only returns on failure
/// (the original data is untouched then).
#[tauri::command]
pub async fn move_data_dir(
    picked: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let db = state.db.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let default = location::default_dir()?;
        let current = location::data_dir()?;
        let target = location::target_for(&PathBuf::from(&picked), &current)?;
        log::info!(
            "[data-location] moving data from {} to {}",
            current.display(),
            target.display()
        );
        let progress_app = app.clone();
        let result = location::move_data(
            &db,
            &default,
            &current,
            &target,
            &|progress| {
                let _ = progress_app.emit("data-move-progress", progress);
            },
            || {
                log::info!("[data-location] data moved; restarting");
                app.request_restart();
                // Keep the database locked until the app exits, so nothing is saved to the old copy.
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(1));
                }
            },
        );
        if let Err(error) = &result {
            log::error!("[data-location] move failed: {error}");
        }
        result
    })
    .await
    .map_err(blocking_error)?
}

/// Delete the old copy left behind by a move.
#[tauri::command]
pub async fn remove_previous_data() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(|| {
        let default = location::default_dir()?;
        let current = location::data_dir()?;
        location::remove_previous_in(&default, &current)?;
        log::info!("[data-location] removed the old copy of the data");
        Ok(())
    })
    .await
    .map_err(blocking_error)?
}

#[tauri::command]
pub async fn open_previous_data_folder() -> Result<(), String> {
    let default = location::default_dir()?;
    let previous = location::read_config_in(&default)
        .previous
        .ok_or("No previous data folder")?;
    crate::commands::datahub::reveal_path(&previous.join(location::DB_FILE), "previous data folder")
}
