//! Where OpenWiki keeps its data: the database plus captured images and thumbnails (issue #17).
//!
//! The data lives in `<system data dir>/com.openwiki.app` unless the user moved it, for example
//! to an external drive because the built-in disk is small. The chosen folder is recorded in
//! `data-location.json`, which always stays in that default folder (next to `secure-store.key`,
//! which never moves) so startup can find it.

use crate::storage::database::Database;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

pub const APP_DIR_NAME: &str = "com.openwiki.app";
pub const DB_FILE: &str = "openwiki.db";
const POINTER_FILE: &str = "data-location.json";
/// Folders that move together with the database.
const DATA_SUBDIRS: [&str; 2] = ["captures", "thumbnails"];
/// The files SQLite keeps for the database.
const DB_FILES: [&str; 3] = ["openwiki.db", "openwiki.db-wal", "openwiki.db-shm"];
/// Created inside the picked folder unless that folder is empty.
const DATA_FOLDER_NAME: &str = "OpenWiki";
/// Files the system drops into folders on its own; a folder holding only these counts as empty.
const SYSTEM_JUNK: [&str; 3] = [".DS_Store", "desktop.ini", "Thumbs.db"];
const WRITE_TEST_FILE: &str = ".openwiki-write-test";

/// Error codes the settings page translates (`storage.moveErrors.<code>`).
pub mod errors {
    pub const NOT_A_FOLDER: &str = "not_a_folder";
    pub const SAME_AS_CURRENT: &str = "same_as_current";
    pub const INSIDE_CURRENT: &str = "inside_current";
    pub const ALREADY_HAS_DATA: &str = "already_has_data";
    pub const NOT_EMPTY: &str = "not_empty";
    pub const NOT_WRITABLE: &str = "not_writable";
    pub const COPY_FAILED: &str = "copy_failed";
}

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
pub struct LocationConfig {
    /// Where the data lives; `None` means the default folder.
    #[serde(default)]
    pub path: Option<PathBuf>,
    /// The folder the data was copied out of, until the user removes that old copy.
    #[serde(default)]
    pub previous: Option<PathBuf>,
}

#[derive(Serialize, Clone, Debug)]
pub struct MoveProgress {
    pub copied: u64,
    pub total: u64,
}

#[derive(Serialize, Debug)]
pub struct DataLocationInfo {
    pub path: String,
    pub is_default: bool,
    pub size_mb: f64,
    /// The old copy left behind by the last move, while it is still there.
    pub previous: Option<PreviousCopy>,
}

#[derive(Serialize, Debug)]
pub struct PreviousCopy {
    pub path: String,
    pub size_mb: f64,
}

pub fn default_dir() -> Result<PathBuf, String> {
    dirs::data_dir()
        .or_else(|| dirs::home_dir().map(|home| home.join("Library").join("Application Support")))
        .map(|base| base.join(APP_DIR_NAME))
        .ok_or_else(|| "Cannot determine application data directory".to_string())
}

pub fn read_config_in(default_dir: &Path) -> LocationConfig {
    let file = default_dir.join(POINTER_FILE);
    let Ok(text) = std::fs::read_to_string(&file) else {
        return LocationConfig::default();
    };
    serde_json::from_str(&text).unwrap_or_else(|error| {
        eprintln!(
            "[data-location] ignoring unreadable {}: {}",
            file.display(),
            error
        );
        LocationConfig::default()
    })
}

fn write_config_in(default_dir: &Path, config: &LocationConfig) -> Result<(), String> {
    std::fs::create_dir_all(default_dir).map_err(|e| e.to_string())?;
    let text = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    // Write a temporary file first so a crash never leaves a half-written location behind.
    let temp = default_dir.join(format!("{POINTER_FILE}.tmp"));
    std::fs::write(&temp, text).map_err(|e| format!("Failed to save data location: {e}"))?;
    std::fs::rename(&temp, default_dir.join(POINTER_FILE))
        .map_err(|e| format!("Failed to save data location: {e}"))
}

pub fn resolve_in(default_dir: &Path) -> PathBuf {
    read_config_in(default_dir)
        .path
        .unwrap_or_else(|| default_dir.to_path_buf())
}

static DATA_DIR: OnceLock<PathBuf> = OnceLock::new();

/// The folder holding the database, captures and thumbnails. It is read once per run: the
/// location only changes through `move_data`, which restarts the app.
pub fn data_dir() -> Result<PathBuf, String> {
    if let Some(dir) = DATA_DIR.get() {
        return Ok(dir.clone());
    }
    let dir = resolve_in(&default_dir()?);
    Ok(DATA_DIR.get_or_init(|| dir).clone())
}

/// A moved data folder that can't be reached right now, e.g. its external drive isn't connected.
fn unreachable_custom_dir_in(default_dir: &Path) -> Option<PathBuf> {
    let custom = read_config_in(default_dir).path?;
    (!custom.join(DB_FILE).is_file()).then_some(custom)
}

/// At startup, before the database opens. If the data was moved to a folder that isn't there,
/// wait a moment (a drive can mount a few seconds after login, when OpenWiki starts with the
/// system), then ask the user to connect it — never start over with an empty database.
/// Returns false when the user chose to quit.
pub fn wait_until_reachable() -> bool {
    let Ok(default) = default_dir() else {
        return true;
    };
    if unreachable_custom_dir_in(&default).is_none() {
        return true;
    }
    for _ in 0..20 {
        std::thread::sleep(Duration::from_millis(500));
        if unreachable_custom_dir_in(&default).is_none() {
            return true;
        }
    }
    while let Some(dir) = unreachable_custom_dir_in(&default) {
        eprintln!("[data-location] data folder unreachable: {}", dir.display());
        if !ask_to_retry(&dir) {
            return false;
        }
    }
    true
}

fn ask_to_retry(dir: &Path) -> bool {
    let zh = crate::locale::system_locale() == crate::locale::LOCALE_ZH_CN;
    let (title, body, retry, quit) = if zh {
        (
            "找不到 OpenWiki 的数据",
            format!(
                "你把 OpenWiki 的数据放在了：\n{}\n\n现在打不开这个位置。如果它在外接硬盘上，请先连接硬盘，再点「重试」。",
                dir.display()
            ),
            "重试",
            "退出",
        )
    } else {
        (
            "OpenWiki can't find its data",
            format!(
                "Your OpenWiki data is stored in:\n{}\n\nThis location isn't available right now. If it's on an external drive, connect the drive, then click Retry.",
                dir.display()
            ),
            "Retry",
            "Quit",
        )
    };
    let dialog = rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Warning)
        .set_title(title);
    #[cfg(target_os = "macos")]
    let dialog = dialog
        .set_description(body)
        .set_buttons(rfd::MessageButtons::OkCancelCustom(
            retry.to_string(),
            quit.to_string(),
        ));
    // Plain OK / Cancel on Windows, which always works; the text says what each one does.
    #[cfg(not(target_os = "macos"))]
    let dialog = dialog
        .set_description(if zh {
            format!("{body}\n\n点「确定」重试，点「取消」退出。")
        } else {
            format!("{body}\n\nClick OK to retry, or Cancel to quit.")
        })
        .set_buttons(rfd::MessageButtons::OkCancel);
    let _ = quit;
    match dialog.show() {
        rfd::MessageDialogResult::Ok | rfd::MessageDialogResult::Yes => true,
        rfd::MessageDialogResult::Custom(label) => label == retry,
        _ => false,
    }
}

fn is_effectively_empty(dir: &Path) -> Result<bool, std::io::Error> {
    for entry in std::fs::read_dir(dir)? {
        let name = entry?.file_name();
        if !SYSTEM_JUNK.iter().any(|junk| name == *junk) {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Where the data would go for the folder the user picked: the folder itself when it is empty,
/// otherwise an `OpenWiki` folder inside it. Nothing is created here.
pub fn target_for(picked: &Path, current: &Path) -> Result<PathBuf, String> {
    let picked = picked
        .canonicalize()
        .map_err(|_| errors::NOT_A_FOLDER.to_string())?;
    if !picked.is_dir() {
        return Err(errors::NOT_A_FOLDER.into());
    }
    let current = current
        .canonicalize()
        .unwrap_or_else(|_| current.to_path_buf());
    if picked == current {
        return Err(errors::SAME_AS_CURRENT.into());
    }
    if picked.starts_with(&current) {
        return Err(errors::INSIDE_CURRENT.into());
    }
    let empty = is_effectively_empty(&picked).map_err(|_| errors::NOT_A_FOLDER.to_string())?;
    let target = if empty {
        picked
    } else {
        picked.join(DATA_FOLDER_NAME)
    };
    if target == current {
        return Err(errors::SAME_AS_CURRENT.into());
    }
    if target.exists() {
        if target.join(DB_FILE).exists() {
            return Err(errors::ALREADY_HAS_DATA.into());
        }
        if !is_effectively_empty(&target).map_err(|_| errors::NOT_A_FOLDER.to_string())? {
            return Err(errors::NOT_EMPTY.into());
        }
    }
    Ok(target)
}

fn count_files(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .map(|entry| match entry.file_type() {
            Ok(kind) if kind.is_dir() => count_files(&entry.path()),
            Ok(kind) if kind.is_file() => 1,
            _ => 0,
        })
        .sum()
}

fn dir_size(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .map(|entry| match entry.file_type() {
            Ok(kind) if kind.is_dir() => dir_size(&entry.path()),
            Ok(kind) if kind.is_file() => entry.metadata().map(|m| m.len()).unwrap_or(0),
            _ => 0,
        })
        .sum()
}

fn data_size(dir: &Path) -> u64 {
    let db: u64 = DB_FILES
        .iter()
        .filter_map(|file| std::fs::metadata(dir.join(file)).ok())
        .map(|meta| meta.len())
        .sum();
    db + DATA_SUBDIRS
        .iter()
        .map(|sub| dir_size(&dir.join(sub)))
        .sum::<u64>()
}

fn to_mb(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

/// Copy a folder tree. With `only_missing`, files already copied with the same size are skipped,
/// which catches up on images saved while the first pass ran.
fn copy_tree(
    src: &Path,
    dst: &Path,
    only_missing: bool,
    on_file: &mut dyn FnMut(),
) -> Result<(), String> {
    if !src.is_dir() {
        return Ok(());
    }
    std::fs::create_dir_all(dst).map_err(|e| format!("{}: {}", dst.display(), e))?;
    for entry in std::fs::read_dir(src).map_err(|e| format!("{}: {}", src.display(), e))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        if kind.is_dir() {
            copy_tree(&from, &to, only_missing, on_file)?;
        } else if kind.is_file() {
            if only_missing {
                let same = match (std::fs::metadata(&from), std::fs::metadata(&to)) {
                    (Ok(a), Ok(b)) => a.len() == b.len(),
                    _ => false,
                };
                if same {
                    continue;
                }
            }
            std::fs::copy(&from, &to).map_err(|e| format!("{}: {}", from.display(), e))?;
            on_file();
        }
    }
    Ok(())
}

/// Image paths are stored in full, so the copied database must point them at the new folder.
fn rewrite_paths(conn: &Connection, from: &Path, to: &Path) -> Result<(), String> {
    let sep = std::path::MAIN_SEPARATOR;
    let old = format!("{}{}", from.to_string_lossy().trim_end_matches(sep), sep);
    let new = format!("{}{}", to.to_string_lossy().trim_end_matches(sep), sep);
    // SQLite's substr counts characters, not bytes.
    let old_len = old.chars().count() as i64;
    for column in ["image_path", "thumbnail_path"] {
        conn.execute(
            &format!(
                "UPDATE captured_content SET {column} = ?1 || substr({column}, ?2) \
                 WHERE substr({column}, 1, ?3) = ?4"
            ),
            params![new, old_len + 1, old_len, old],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn verify_copy(conn: &Connection, rows: i64, from: &Path, to: &Path) -> Result<(), String> {
    let check: String = conn
        .query_row("PRAGMA quick_check", [], |row| row.get(0))
        .map_err(|e| e.to_string())?;
    let copied: i64 = conn
        .query_row("SELECT COUNT(*) FROM captured_content", [], |row| {
            row.get(0)
        })
        .map_err(|e| e.to_string())?;
    let files_ok = DATA_SUBDIRS
        .iter()
        .all(|sub| count_files(&to.join(sub)) >= count_files(&from.join(sub)));
    if check != "ok" || copied != rows || !files_ok {
        return Err(format!(
            "{}: check={check} rows={copied}/{rows} files_ok={files_ok}",
            errors::COPY_FAILED
        ));
    }
    Ok(())
}

/// Copy everything to `target`, check the copy, then switch OpenWiki over to it.
///
/// Images are copied first while OpenWiki keeps running. Then writes are frozen (the database
/// lock is held) while the database is snapshotted and newer images are copied, and the lock is
/// held until `switch_over` — which restarts the app — so nothing saved meanwhile is lost.
/// The original folder is left untouched; on failure everything copied so far is removed.
pub fn move_data(
    db: &Database,
    default_dir: &Path,
    current: &Path,
    target: &Path,
    on_progress: &dyn Fn(MoveProgress),
    switch_over: impl FnOnce(),
) -> Result<(), String> {
    let target_existed = target.exists();
    let result = (|| -> Result<(), String> {
        std::fs::create_dir_all(target).map_err(|_| errors::NOT_WRITABLE.to_string())?;
        let probe = target.join(WRITE_TEST_FILE);
        std::fs::write(&probe, b"ok").map_err(|_| errors::NOT_WRITABLE.to_string())?;
        let _ = std::fs::remove_file(&probe);

        // The database counts as one more step at the end.
        let total = DATA_SUBDIRS
            .iter()
            .map(|sub| count_files(&current.join(sub)))
            .sum::<u64>()
            + 1;
        let mut copied = 0u64;
        on_progress(MoveProgress { copied, total });
        let mut tick = || {
            copied += 1;
            if copied % 20 == 0 {
                on_progress(MoveProgress {
                    copied: copied.min(total - 1),
                    total,
                });
            }
        };
        for sub in DATA_SUBDIRS {
            copy_tree(&current.join(sub), &target.join(sub), false, &mut tick)?;
        }

        let conn = db.conn.lock().map_err(|e| format!("Lock error: {e}"))?;
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM captured_content", [], |row| {
                row.get(0)
            })
            .map_err(|e| e.to_string())?;
        let new_db = target.join(DB_FILE);
        conn.execute("VACUUM INTO ?1", params![new_db.to_string_lossy()])
            .map_err(|e| format!("{}: {}", errors::COPY_FAILED, e))?;
        let mut catch_up = || {};
        for sub in DATA_SUBDIRS {
            copy_tree(&current.join(sub), &target.join(sub), true, &mut catch_up)?;
        }

        let copy = Connection::open(&new_db).map_err(|e| e.to_string())?;
        rewrite_paths(&copy, current, target)?;
        verify_copy(&copy, rows, current, target)?;
        drop(copy);

        let mut config = read_config_in(default_dir);
        config.previous = Some(current.to_path_buf());
        config.path = Some(target.to_path_buf());
        write_config_in(default_dir, &config)?;
        on_progress(MoveProgress {
            copied: total,
            total,
        });

        switch_over();
        drop(conn);
        Ok(())
    })();

    if result.is_err() {
        if target_existed {
            for file in DB_FILES {
                let _ = std::fs::remove_file(target.join(file));
            }
            for sub in DATA_SUBDIRS {
                let _ = std::fs::remove_dir_all(target.join(sub));
            }
            let _ = std::fs::remove_file(target.join(WRITE_TEST_FILE));
        } else {
            let _ = std::fs::remove_dir_all(target);
        }
    }
    result
}

pub fn info_in(default_dir: &Path, current: &Path) -> DataLocationInfo {
    let config = read_config_in(default_dir);
    let previous = config
        .previous
        .filter(|prev| prev != current && prev.join(DB_FILE).is_file())
        .map(|prev| PreviousCopy {
            size_mb: to_mb(data_size(&prev)),
            path: prev.to_string_lossy().into_owned(),
        });
    DataLocationInfo {
        path: current.to_string_lossy().into_owned(),
        is_default: current == default_dir,
        size_mb: to_mb(data_size(current)),
        previous,
    }
}

/// Delete the old copy a move left behind. Only OpenWiki's own files are removed from it.
pub fn remove_previous_in(default_dir: &Path, current: &Path) -> Result<(), String> {
    let mut config = read_config_in(default_dir);
    let Some(previous) = config.previous.clone() else {
        return Ok(());
    };
    if previous != current {
        for file in DB_FILES {
            let path = previous.join(file);
            if path.exists() {
                std::fs::remove_file(&path).map_err(|e| format!("{}: {}", path.display(), e))?;
            }
        }
        for sub in DATA_SUBDIRS {
            let path = previous.join(sub);
            if path.exists() {
                std::fs::remove_dir_all(&path).map_err(|e| format!("{}: {}", path.display(), e))?;
            }
        }
        // A folder that only held OpenWiki's data goes too (never the default folder, which
        // keeps the encryption key and this location file).
        if previous != default_dir && is_effectively_empty(&previous).unwrap_or(false) {
            let _ = std::fs::remove_dir_all(&previous);
        }
    }
    config.previous = None;
    write_config_in(default_dir, &config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "openwiki-data-location-{}-{}",
            name,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root.canonicalize().unwrap()
    }

    fn insert_capture(db: &Database, id: &str, image: &Path, thumb: &Path) {
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO captured_content (id, content_type, image_path, thumbnail_path, source_app, \
             captured_at, content_hash) VALUES (?1, 'image', ?2, ?3, 'Test', '2026-11-20T10:00:00Z', ?1)",
            params![id, image.to_string_lossy(), thumb.to_string_lossy()],
        )
        .unwrap();
    }

    #[test]
    fn config_round_trip_and_resolution() {
        let root = temp_root("config");
        assert_eq!(resolve_in(&root), root);
        let custom = root.join("elsewhere");
        let config = LocationConfig {
            path: Some(custom.clone()),
            previous: None,
        };
        write_config_in(&root, &config).unwrap();
        assert_eq!(read_config_in(&root), config);
        assert_eq!(resolve_in(&root), custom);
        // Not created yet, so it counts as unreachable.
        assert_eq!(unreachable_custom_dir_in(&root), Some(custom.clone()));
        std::fs::create_dir_all(&custom).unwrap();
        std::fs::write(custom.join(DB_FILE), b"").unwrap();
        assert_eq!(unreachable_custom_dir_in(&root), None);
        // A damaged file falls back to the default folder.
        std::fs::write(root.join(POINTER_FILE), "{not json").unwrap();
        assert_eq!(resolve_in(&root), root);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn target_rules() {
        let root = temp_root("target");
        let current = root.join("current");
        std::fs::create_dir_all(&current).unwrap();

        let empty = root.join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        std::fs::write(empty.join(".DS_Store"), b"").unwrap();
        assert_eq!(target_for(&empty, &current).unwrap(), empty);

        let busy = root.join("busy");
        std::fs::create_dir_all(&busy).unwrap();
        std::fs::write(busy.join("photo.jpg"), b"x").unwrap();
        assert_eq!(target_for(&busy, &current).unwrap(), busy.join("OpenWiki"));

        assert_eq!(
            target_for(&current, &current).unwrap_err(),
            errors::SAME_AS_CURRENT
        );
        let inner = current.join("inner");
        std::fs::create_dir_all(&inner).unwrap();
        assert_eq!(
            target_for(&inner, &current).unwrap_err(),
            errors::INSIDE_CURRENT
        );
        assert_eq!(
            target_for(&root.join("missing"), &current).unwrap_err(),
            errors::NOT_A_FOLDER
        );

        std::fs::create_dir_all(busy.join("OpenWiki")).unwrap();
        std::fs::write(busy.join("OpenWiki").join("notes.txt"), b"x").unwrap();
        assert_eq!(target_for(&busy, &current).unwrap_err(), errors::NOT_EMPTY);
        std::fs::write(busy.join("OpenWiki").join(DB_FILE), b"").unwrap();
        assert_eq!(
            target_for(&busy, &current).unwrap_err(),
            errors::ALREADY_HAS_DATA
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn move_copies_everything_and_switches() {
        let root = temp_root("move");
        let default = root.join("默认 data");
        let current = default.clone();
        std::fs::create_dir_all(current.join("captures").join(".pending")).unwrap();
        std::fs::create_dir_all(current.join("thumbnails")).unwrap();
        let image = current.join("captures").join("a.png");
        let thumb = current.join("thumbnails").join("a_thumb.png");
        std::fs::write(&image, b"image-bytes").unwrap();
        std::fs::write(&thumb, b"thumb").unwrap();
        std::fs::write(
            current.join("captures").join(".pending").join("p.png"),
            b"p",
        )
        .unwrap();

        let db = Database::new_in_memory().unwrap();
        insert_capture(&db, "a", &image, &thumb);
        // Stored somewhere else entirely: left alone.
        insert_capture(
            &db,
            "b",
            Path::new("/elsewhere/b.png"),
            Path::new("/elsewhere/b_t.png"),
        );

        let target = root.join("外接盘").join("OpenWiki");
        let progress = RefCell::new(Vec::new());
        let switched = RefCell::new(false);
        move_data(
            &db,
            &default,
            &current,
            &target,
            &|p| progress.borrow_mut().push((p.copied, p.total)),
            || *switched.borrow_mut() = true,
        )
        .unwrap();

        assert!(*switched.borrow());
        assert_eq!(progress.borrow().last(), Some(&(4, 4)));
        assert_eq!(
            std::fs::read(target.join("captures").join("a.png")).unwrap(),
            b"image-bytes"
        );
        assert!(target
            .join("captures")
            .join(".pending")
            .join("p.png")
            .is_file());
        assert!(target.join("thumbnails").join("a_thumb.png").is_file());
        // The original stays put until the user removes it.
        assert!(image.is_file());

        let copy = Connection::open(target.join(DB_FILE)).unwrap();
        let paths: Vec<(String, String)> = copy
            .prepare("SELECT image_path, thumbnail_path FROM captured_content ORDER BY id")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .map(|row| row.unwrap())
            .collect();
        assert_eq!(
            paths[0],
            (
                target
                    .join("captures")
                    .join("a.png")
                    .to_string_lossy()
                    .into_owned(),
                target
                    .join("thumbnails")
                    .join("a_thumb.png")
                    .to_string_lossy()
                    .into_owned()
            )
        );
        assert_eq!(paths[1].0, "/elsewhere/b.png");

        let config = read_config_in(&default);
        assert_eq!(config.path.as_deref(), Some(target.as_path()));
        assert_eq!(config.previous.as_deref(), Some(current.as_path()));

        // The old copy shows up until it is removed; the default folder itself stays.
        std::fs::write(current.join(DB_FILE), b"old").unwrap();
        std::fs::write(current.join("secure-store.key"), b"key").unwrap();
        let info = info_in(&default, &target);
        assert!(!info.is_default);
        assert!(info.previous.is_some());
        remove_previous_in(&default, &target).unwrap();
        assert!(!current.join(DB_FILE).exists());
        assert!(!current.join("captures").exists());
        assert!(current.join("secure-store.key").is_file());
        assert!(current.join(POINTER_FILE).is_file());
        assert!(info_in(&default, &target).previous.is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn failed_move_cleans_up_and_keeps_location() {
        let root = temp_root("fail");
        let default = root.join("default");
        std::fs::create_dir_all(default.join("captures")).unwrap();
        std::fs::write(default.join("captures").join("a.png"), b"a").unwrap();
        let db = Database::new_in_memory().unwrap();
        // The snapshot can't be written over a folder of the same name, after images were copied.
        let target = root.join("new");
        std::fs::create_dir_all(target.join(DB_FILE)).unwrap();
        let result = move_data(&db, &default, &default, &target, &|_| {}, || {
            panic!("must not switch")
        });
        assert!(result.unwrap_err().starts_with(errors::COPY_FAILED));
        assert!(!target.join("captures").exists());
        assert!(default.join("captures").join("a.png").is_file());
        assert_eq!(read_config_in(&default), LocationConfig::default());

        // A target that can't be created at all.
        let blocker = root.join("file.txt");
        std::fs::write(&blocker, b"x").unwrap();
        let result = move_data(&db, &default, &default, &blocker.join("x"), &|_| {}, || {
            panic!("must not switch")
        });
        assert_eq!(result.unwrap_err(), errors::NOT_WRITABLE);
        let _ = std::fs::remove_dir_all(&root);
    }
}
