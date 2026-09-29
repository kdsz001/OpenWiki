use crate::commands::capture::AppState;
use crate::storage::database::Database;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use tauri::State;

const MANIFEST_FILE: &str = "manifest.json";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct OrphanCandidate {
    pub path: String,
    pub size: u64,
    pub kind: String,
}

#[derive(Debug, Serialize)]
pub struct OrphanScan {
    pub candidates: Vec<OrphanCandidate>,
    pub total_size: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct ManifestEntry {
    original_path: String,
    quarantine_path: String,
    size: u64,
    quarantined_at: String,
    status: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct OrphanManifest {
    created_at: String,
    entries: Vec<ManifestEntry>,
}

#[derive(Debug, Serialize)]
pub struct QuarantineResult {
    pub moved: Vec<OrphanCandidate>,
    pub manifests: Vec<String>,
}

fn is_supported_image(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .map(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "tif" | "tiff" | "avif" | "heic"
            )
        })
        .unwrap_or(false)
}

fn referenced_paths(db: &Database) -> Result<HashSet<PathBuf>, String> {
    let conn = db
        .conn
        .lock()
        .map_err(|error| format!("Database lock failed: {}", error))?;
    let mut statement = conn
        .prepare("SELECT image_path, thumbnail_path FROM captured_content")
        .map_err(|error| format!("Failed to query image references: {}", error))?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                row.get::<_, Option<String>>(1)?,
            ))
        })
        .map_err(|error| format!("Failed to read image references: {}", error))?;

    let mut paths = HashSet::new();
    for row in rows {
        let (image, thumbnail) =
            row.map_err(|error| format!("Failed to decode image reference: {}", error))?;
        for path in [image, thumbnail].into_iter().flatten() {
            if let Ok(canonical) = Path::new(&path).canonicalize() {
                paths.insert(canonical);
            }
        }
    }
    Ok(paths)
}

fn scan_directory(
    root: &Path,
    kind: &str,
    references: &HashSet<PathBuf>,
    output: &mut Vec<OrphanCandidate>,
) -> Result<(), String> {
    for entry in std::fs::read_dir(root)
        .map_err(|error| format!("Failed to read {}: {}", root.display(), error))?
    {
        let entry = entry.map_err(|error| format!("Failed to read directory entry: {}", error))?;
        let file_type = entry
            .file_type()
            .map_err(|error| format!("Failed to read file type: {}", error))?;
        let path = entry.path();
        if !file_type.is_file() || !is_supported_image(&path) {
            continue;
        }
        let canonical = path
            .canonicalize()
            .map_err(|error| format!("Failed to resolve {}: {}", path.display(), error))?;
        if references.contains(&canonical) {
            continue;
        }
        let size = entry
            .metadata()
            .map_err(|error| format!("Failed to read {} metadata: {}", path.display(), error))?
            .len();
        output.push(OrphanCandidate {
            path: canonical.to_string_lossy().to_string(),
            size,
            kind: kind.to_string(),
        });
    }
    Ok(())
}

fn scan_image_orphans_in(
    db: &Database,
    captures: &Path,
    thumbnails: &Path,
) -> Result<OrphanScan, String> {
    let references = referenced_paths(db)?;
    let mut candidates = Vec::new();
    scan_directory(captures, "capture", &references, &mut candidates)?;
    scan_directory(thumbnails, "thumbnail", &references, &mut candidates)?;
    candidates.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(OrphanScan {
        total_size: candidates.iter().map(|candidate| candidate.size).sum(),
        candidates,
    })
}

// async on the three image commands: a plain command runs on the UI thread, and these walk or
// move every saved image.
#[tauri::command(async)]
pub fn scan_image_orphans(state: State<'_, AppState>) -> Result<OrphanScan, String> {
    scan_image_orphans_in(
        &state.db,
        &super::image_lifecycle::captures_dir()?,
        &super::image_lifecycle::thumbnails_dir()?,
    )
}

fn write_manifest(path: &Path, manifest: &OrphanManifest) -> Result<(), String> {
    let json = serde_json::to_vec_pretty(manifest)
        .map_err(|error| format!("Failed to serialize orphan manifest: {}", error))?;
    std::fs::write(path, json)
        .map_err(|error| format!("Failed to write {}: {}", path.display(), error))
}

fn quarantine_image_orphans_in(
    db: &Database,
    captures: &Path,
    thumbnails: &Path,
    reviewed_paths: &[String],
    timestamp: &str,
) -> Result<QuarantineResult, String> {
    let scan = scan_image_orphans_in(db, captures, thumbnails)?;
    let current: HashSet<&str> = scan
        .candidates
        .iter()
        .map(|candidate| candidate.path.as_str())
        .collect();
    let reviewed: HashSet<&str> = reviewed_paths.iter().map(String::as_str).collect();
    if reviewed.len() != reviewed_paths.len() || !reviewed.iter().all(|path| current.contains(path))
    {
        return Err(
            "Reviewed orphan list is stale or contains an unsafe path; run dry-run again"
                .to_string(),
        );
    }

    let mut moved = Vec::new();
    let mut manifests = Vec::new();
    for (kind, root) in [("capture", captures), ("thumbnail", thumbnails)] {
        let selected: Vec<_> = scan
            .candidates
            .iter()
            .filter(|candidate| {
                candidate.kind == kind && reviewed.contains(candidate.path.as_str())
            })
            .cloned()
            .collect();
        if selected.is_empty() {
            continue;
        }

        let quarantine_dir = root.join("_orphaned").join(timestamp);
        std::fs::create_dir_all(&quarantine_dir).map_err(|error| {
            format!(
                "Failed to create quarantine directory {}: {}",
                quarantine_dir.display(),
                error
            )
        })?;
        let manifest_path = quarantine_dir.join(MANIFEST_FILE);
        if manifest_path.exists() {
            return Err(format!("Quarantine batch already exists: {}", timestamp));
        }
        let created_at = Utc::now().to_rfc3339();
        let mut manifest = OrphanManifest {
            created_at: created_at.clone(),
            entries: Vec::new(),
        };

        for candidate in selected {
            let source = Path::new(&candidate.path);
            let file_name = source
                .file_name()
                .ok_or_else(|| format!("Invalid candidate path: {}", candidate.path))?;
            let destination = quarantine_dir.join(file_name);
            if destination.exists() {
                return Err(format!("Refusing to overwrite {}", destination.display()));
            }

            manifest.entries.push(ManifestEntry {
                original_path: candidate.path.clone(),
                quarantine_path: destination.to_string_lossy().to_string(),
                size: candidate.size,
                quarantined_at: created_at.clone(),
                status: "planned".to_string(),
            });
            write_manifest(&manifest_path, &manifest)?;
            if let Err(error) = std::fs::rename(source, &destination) {
                manifest.entries.pop();
                write_manifest(&manifest_path, &manifest)?;
                return Err(format!(
                    "Failed to quarantine {}: {}",
                    source.display(),
                    error
                ));
            }
            if let Some(entry) = manifest.entries.last_mut() {
                entry.status = "moved".to_string();
            }
            write_manifest(&manifest_path, &manifest)?;
            moved.push(candidate);
        }
        manifests.push(manifest_path.to_string_lossy().to_string());
    }

    Ok(QuarantineResult { moved, manifests })
}

#[tauri::command(async)]
pub fn quarantine_image_orphans(
    state: State<'_, AppState>,
    reviewed_paths: Vec<String>,
) -> Result<QuarantineResult, String> {
    let now = Utc::now();
    let timestamp = format!(
        "{}-{}",
        now.format("%Y%m%dT%H%M%SZ"),
        now.timestamp_subsec_millis()
    );
    quarantine_image_orphans_in(
        &state.db,
        &super::image_lifecycle::captures_dir()?,
        &super::image_lifecycle::thumbnails_dir()?,
        &reviewed_paths,
        &timestamp,
    )
}

fn restore_manifest_in(manifest_path: &Path, root: &Path) -> Result<usize, String> {
    let orphan_root = root.join("_orphaned").canonicalize().map_err(|error| {
        format!(
            "Failed to resolve quarantine root {}: {}",
            root.display(),
            error
        )
    })?;
    let canonical_manifest = manifest_path
        .canonicalize()
        .map_err(|error| format!("Failed to resolve manifest: {}", error))?;
    if canonical_manifest
        .file_name()
        .and_then(|name| name.to_str())
        != Some(MANIFEST_FILE)
        || !canonical_manifest.starts_with(&orphan_root)
    {
        return Err("Manifest is outside the OpenWiki quarantine directory".to_string());
    }

    let bytes = std::fs::read(&canonical_manifest)
        .map_err(|error| format!("Failed to read manifest: {}", error))?;
    let manifest: OrphanManifest = serde_json::from_slice(&bytes)
        .map_err(|error| format!("Failed to parse manifest: {}", error))?;
    let canonical_root = root
        .canonicalize()
        .map_err(|error| format!("Failed to resolve image root: {}", error))?;

    for entry in manifest
        .entries
        .iter()
        .filter(|entry| entry.status == "moved")
    {
        let original = Path::new(&entry.original_path);
        let quarantined = Path::new(&entry.quarantine_path);
        let canonical_quarantined = quarantined
            .canonicalize()
            .map_err(|error| format!("Failed to resolve quarantined image: {}", error))?;
        if original.parent() != Some(canonical_root.as_path())
            || canonical_quarantined.parent() != canonical_manifest.parent()
            || original.exists()
        {
            return Err(format!(
                "Manifest entry is unsafe or cannot be restored: {}",
                entry.original_path
            ));
        }
    }

    let mut restored = 0;
    for entry in manifest
        .entries
        .iter()
        .filter(|entry| entry.status == "moved")
    {
        std::fs::rename(&entry.quarantine_path, &entry.original_path)
            .map_err(|error| format!("Failed to restore {}: {}", entry.original_path, error))?;
        restored += 1;
    }
    Ok(restored)
}

#[tauri::command(async)]
pub fn restore_quarantined_images(manifest_path: String) -> Result<usize, String> {
    let manifest = Path::new(&manifest_path);
    let captures = super::image_lifecycle::captures_dir()?;
    let thumbnails = super::image_lifecycle::thumbnails_dir()?;
    let canonical_manifest = manifest
        .canonicalize()
        .map_err(|error| format!("Failed to resolve manifest: {}", error))?;
    let capture_orphans = captures.join("_orphaned");
    if capture_orphans.exists()
        && canonical_manifest.starts_with(
            capture_orphans
                .canonicalize()
                .map_err(|error| error.to_string())?,
        )
    {
        return restore_manifest_in(manifest, &captures);
    }
    restore_manifest_in(manifest, &thumbnails)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;

    fn test_db() -> Database {
        Database::new_in_memory().unwrap()
    }

    fn dirs(root: &Path) -> (PathBuf, PathBuf) {
        let captures = root.join("captures");
        let thumbnails = root.join("thumbnails");
        std::fs::create_dir_all(captures.join(".pending")).unwrap();
        std::fs::create_dir_all(captures.join("_orphaned/old")).unwrap();
        std::fs::create_dir_all(thumbnails.join("_orphaned/old")).unwrap();
        (captures, thumbnails)
    }

    fn insert_reference(
        db: &Database,
        id: &str,
        image: Option<&Path>,
        thumbnail: Option<&Path>,
        deleted: bool,
    ) {
        db.conn
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO captured_content (id, content_type, image_path, thumbnail_path, source_app, captured_at, content_hash, byte_size, is_deleted) VALUES (?1, 'image', ?2, ?3, 'test', '2026-01-01T00:00:00Z', ?4, 1, ?5)",
                params![id, image.map(|path| path.to_string_lossy().to_string()), thumbnail.map(|path| path.to_string_lossy().to_string()), format!("hash-{id}"), deleted],
            )
            .unwrap();
    }

    #[test]
    fn dry_run_includes_only_unreferenced_root_images() {
        let root = tempfile::tempdir().unwrap();
        let (captures, thumbnails) = dirs(root.path());
        let referenced = captures.join("referenced.png");
        let deleted_reference = thumbnails.join("deleted-thumb.png");
        let orphan = captures.join("orphan.jpg");
        std::fs::write(&referenced, b"image").unwrap();
        std::fs::write(&deleted_reference, b"thumb").unwrap();
        std::fs::write(&orphan, b"orphan").unwrap();
        std::fs::write(captures.join("notes.log"), b"log").unwrap();
        std::fs::write(captures.join(".pending/pending.png"), b"pending").unwrap();
        std::fs::write(captures.join("_orphaned/old/old.png"), b"old").unwrap();
        let db = test_db();
        insert_reference(&db, "active", Some(&referenced), None, false);
        insert_reference(&db, "deleted", None, Some(&deleted_reference), true);

        let scan = scan_image_orphans_in(&db, &captures, &thumbnails).unwrap();
        assert_eq!(scan.candidates.len(), 1);
        assert_eq!(
            scan.candidates[0].path,
            orphan.canonicalize().unwrap().to_string_lossy()
        );
    }

    #[test]
    fn quarantine_is_idempotent_and_manifest_restores_files() {
        let root = tempfile::tempdir().unwrap();
        let (captures, thumbnails) = dirs(root.path());
        let orphan = captures.join("orphan.png");
        std::fs::write(&orphan, b"orphan").unwrap();
        let db = test_db();
        let candidate = scan_image_orphans_in(&db, &captures, &thumbnails)
            .unwrap()
            .candidates
            .remove(0);

        let result = quarantine_image_orphans_in(
            &db,
            &captures,
            &thumbnails,
            std::slice::from_ref(&candidate.path),
            "batch",
        )
        .unwrap();
        assert_eq!(result.moved, vec![candidate]);
        assert!(!orphan.exists());
        assert!(scan_image_orphans_in(&db, &captures, &thumbnails)
            .unwrap()
            .candidates
            .is_empty());

        let manifest = Path::new(&result.manifests[0]);
        assert_eq!(restore_manifest_in(manifest, &captures), Ok(1));
        assert_eq!(std::fs::read(&orphan).unwrap(), b"orphan");
    }

    #[test]
    fn stale_review_aborts_without_moving_any_file() {
        let root = tempfile::tempdir().unwrap();
        let (captures, thumbnails) = dirs(root.path());
        let first = captures.join("first.png");
        let second = captures.join("second.png");
        std::fs::write(&first, b"first").unwrap();
        std::fs::write(&second, b"second").unwrap();
        let db = test_db();
        let scan = scan_image_orphans_in(&db, &captures, &thumbnails).unwrap();
        insert_reference(&db, "new-reference", Some(&second), None, false);

        assert!(quarantine_image_orphans_in(
            &db,
            &captures,
            &thumbnails,
            &scan
                .candidates
                .iter()
                .map(|item| item.path.clone())
                .collect::<Vec<_>>(),
            "batch",
        )
        .is_err());
        assert!(first.exists());
        assert!(second.exists());
    }

    #[test]
    fn external_or_duplicate_review_paths_are_rejected() {
        let root = tempfile::tempdir().unwrap();
        let (captures, thumbnails) = dirs(root.path());
        let external = root.path().join("external.png");
        std::fs::write(&external, b"external").unwrap();
        let db = test_db();
        let path = external.to_string_lossy().to_string();

        assert!(quarantine_image_orphans_in(
            &db,
            &captures,
            &thumbnails,
            &[path.clone(), path],
            "batch",
        )
        .is_err());
        assert!(external.exists());
    }

    #[test]
    fn referenced_paths_reads_all_rows_including_deleted() {
        let db = test_db();
        let root = tempfile::tempdir().unwrap();
        let image = root.path().join("image.png");
        std::fs::write(&image, b"image").unwrap();
        insert_reference(&db, "deleted", Some(&image), None, true);
        assert!(referenced_paths(&db)
            .unwrap()
            .contains(&image.canonicalize().unwrap()));
    }
}
