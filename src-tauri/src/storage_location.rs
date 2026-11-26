use crate::{
    clipboard_preferences, clipboard_runtime,
    commands::{authorize, on_main},
    platform, AppState,
};
use nami::database::HistoryDatabase;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::Ordering,
    time::Duration,
};
use tauri::{AppHandle, Manager, Webview};

const LOCATION_POINTER_FILE: &str = "storage-location.json";
const DB_FILENAME: &str = "nami.sqlite3";

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct StorageLocationPointer {
    data_dir: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StorageLocationInfo {
    pub data_dir: String,
    pub default_data_dir: String,
    pub is_custom_data_dir: bool,
    pub log_dir: String,
}

pub(crate) fn default_data_dir(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map_err(|_| "storage_unavailable".into())
}

pub(crate) fn active_data_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let state = app.state::<AppState>();
    if let Some(dir) = state.data_dir.lock().unwrap().clone() {
        return Ok(dir);
    }
    default_data_dir(app)
}

pub(crate) fn resolve_startup_data_dir(default_dir: &Path) -> PathBuf {
    let pointer_path = default_dir.join(LOCATION_POINTER_FILE);
    let Ok(raw) = fs::read_to_string(&pointer_path) else {
        return default_dir.to_path_buf();
    };
    let Ok(pointer) = serde_json::from_str::<StorageLocationPointer>(&raw) else {
        return default_dir.to_path_buf();
    };
    let trimmed = pointer.data_dir.trim();
    if trimmed.is_empty() || trimmed.contains('\0') {
        return default_dir.to_path_buf();
    }
    let path = PathBuf::from(trimmed);
    if !path.is_absolute() {
        return default_dir.to_path_buf();
    }
    path
}

fn persist_custom_data_dir(default_dir: &Path, custom_dir: Option<&Path>) -> Result<(), String> {
    let pointer_path = default_dir.join(LOCATION_POINTER_FILE);
    match custom_dir {
        None => {
            if let Err(error) = fs::remove_file(&pointer_path) {
                if error.kind() != std::io::ErrorKind::NotFound {
                    return Err("storage_unavailable".into());
                }
            }
            Ok(())
        }
        Some(dir) => {
            ensure_private_dir(default_dir)?;
            let dir_str = dir.to_str().ok_or("storage_path")?.to_owned();
            let payload =
                serde_json::to_string_pretty(&StorageLocationPointer { data_dir: dir_str })
                    .map_err(|_| "storage_record")?;
            let temp = default_dir.join(format!(
                ".{LOCATION_POINTER_FILE}.{}.tmp",
                uuid::Uuid::new_v4().simple()
            ));
            fs::write(&temp, payload).map_err(|_| "storage_unavailable")?;
            if fs::rename(&temp, &pointer_path).is_err() {
                let _ = fs::remove_file(&temp);
                return Err("storage_unavailable".into());
            }
            Ok(())
        }
    }
}

fn ensure_private_dir(directory: &Path) -> Result<(), String> {
    fs::create_dir_all(directory).map_err(|_| "storage_unavailable")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(directory, fs::Permissions::from_mode(0o700));
    }
    Ok(())
}

fn canonical_or_normalized(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn same_directory(left: &Path, right: &Path) -> bool {
    canonical_or_normalized(left) == canonical_or_normalized(right)
}

fn is_symlink_or_has_symlink_leaf(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .ok()
        .is_some_and(|meta| meta.file_type().is_symlink())
}

fn validate_migration_target(
    source_dir: &Path,
    target_dir: &Path,
    forbidden_roots: &[PathBuf],
) -> Result<PathBuf, String> {
    if !target_dir.is_absolute()
        || target_dir
            .to_str()
            .is_none_or(|value| value.is_empty() || value.contains('\0'))
    {
        return Err("storage_dir_invalid".into());
    }
    if is_symlink_or_has_symlink_leaf(target_dir) {
        return Err("storage_dir_invalid".into());
    }
    fs::create_dir_all(target_dir).map_err(|_| "storage_dir_unwritable")?;
    let target = target_dir
        .canonicalize()
        .map_err(|_| "storage_dir_invalid")?;
    let source = canonical_or_normalized(source_dir);
    if target == source {
        return Err("storage_dir_same".into());
    }
    if target.starts_with(&source) || source.starts_with(&target) {
        return Err("storage_dir_nested".into());
    }
    for root in forbidden_roots {
        let forbidden = canonical_or_normalized(root);
        if target == forbidden || target.starts_with(&forbidden) || forbidden.starts_with(&target) {
            return Err("storage_dir_invalid".into());
        }
    }
    let meta = fs::symlink_metadata(&target).map_err(|_| "storage_dir_invalid")?;
    if !meta.is_dir() {
        return Err("storage_dir_invalid".into());
    }
    // Verify target is writable before copying any user data.
    let probe = target.join(format!(
        ".nami-write-probe-{}",
        uuid::Uuid::new_v4().simple()
    ));
    fs::write(&probe, b"ok").map_err(|_| "storage_dir_unwritable")?;
    let _ = fs::remove_file(&probe);

    if target.join(DB_FILENAME).exists()
        || target.join("resources").join("clipboard-images").exists()
    {
        return Err("storage_dir_not_empty".into());
    }
    Ok(target)
}

fn copy_images_tree(source_images: &Path, target_images: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(source_images) {
        Ok(meta) if meta.is_dir() => (),
        Ok(_) => return Err("storage_path".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(_) => return Err("storage_unavailable".into()),
    }
    copy_dir_recursive(source_images, target_images)?;
    Ok(true)
}

fn copy_dir_recursive(source: &Path, target: &Path) -> Result<(), String> {
    fs::create_dir_all(target).map_err(|_| "storage_migrate_failed")?;
    let entries = fs::read_dir(source).map_err(|_| "storage_migrate_failed")?;
    for entry in entries {
        let entry = entry.map_err(|_| "storage_migrate_failed")?;
        let src_path = entry.path();
        let meta = fs::symlink_metadata(&src_path).map_err(|_| "storage_migrate_failed")?;
        if meta.file_type().is_symlink() {
            continue;
        }
        let dst_path = target.join(entry.file_name());
        if meta.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else if meta.is_file() {
            fs::copy(&src_path, &dst_path).map_err(|_| "storage_migrate_failed")?;
        }
    }
    Ok(())
}

fn cleanup_target_rollback(target: &Path, temp_db: &Path, copied_images: bool) {
    let _ = fs::remove_file(temp_db);
    let _ = fs::remove_file(target.join(DB_FILENAME));
    let _ = fs::remove_file(target.join(format!("{DB_FILENAME}-wal")));
    let _ = fs::remove_file(target.join(format!("{DB_FILENAME}-shm")));
    if copied_images {
        let images = target.join("resources").join("clipboard-images");
        let _ = fs::remove_dir_all(&images);
        let resources = target.join("resources");
        if fs::read_dir(&resources)
            .ok()
            .is_some_and(|mut it| it.next().is_none())
        {
            let _ = fs::remove_dir(&resources);
        }
    }
}

fn cleanup_migrated_source(source: &Path) {
    let _ = fs::remove_file(source.join(DB_FILENAME));
    let _ = fs::remove_file(source.join(format!("{DB_FILENAME}-wal")));
    let _ = fs::remove_file(source.join(format!("{DB_FILENAME}-shm")));
    let images = source.join("resources").join("clipboard-images");
    if fs::symlink_metadata(&images)
        .ok()
        .is_some_and(|meta| meta.is_dir())
    {
        let _ = fs::remove_dir_all(&images);
    }
    let resources = source.join("resources");
    if fs::read_dir(&resources)
        .ok()
        .is_some_and(|mut it| it.next().is_none())
    {
        let _ = fs::remove_dir(&resources);
    }
}

fn migrate_database_and_images(
    db: &HistoryDatabase,
    source_dir: &Path,
    target_dir: &Path,
    default_dir: &Path,
    forbidden_roots: &[PathBuf],
) -> Result<(HistoryDatabase, PathBuf), String> {
    let target = validate_migration_target(source_dir, target_dir, forbidden_roots)?;
    let temp_db = target.join(format!(
        ".nami-migrate-{}.sqlite3",
        uuid::Uuid::new_v4().simple()
    ));
    let mut copied_images = false;
    let outcome = (|| -> Result<HistoryDatabase, String> {
        db.vacuum_into(&temp_db)?;
        let source_images = source_dir.join("resources").join("clipboard-images");
        let target_images = target.join("resources").join("clipboard-images");
        copied_images = copy_images_tree(&source_images, &target_images)?;
        let target_db_path = target.join(DB_FILENAME);
        fs::rename(&temp_db, &target_db_path).map_err(|_| "storage_migrate_failed")?;
        let new_db = HistoryDatabase::open(&target_db_path)?;
        let custom = (!same_directory(&target, default_dir)).then_some(target.as_path());
        persist_custom_data_dir(default_dir, custom)?;
        Ok(new_db)
    })();
    match outcome {
        Ok(new_db) => Ok((new_db, target)),
        Err(error) => {
            cleanup_target_rollback(&target, &temp_db, copied_images);
            Err(error)
        }
    }
}

fn build_location_info(app: &AppHandle) -> Result<StorageLocationInfo, String> {
    let default_dir = default_data_dir(app)?;
    let active_dir = active_data_dir(app)?;
    let log_dir = app
        .path()
        .app_log_dir()
        .map_err(|_| "storage_unavailable")?;
    let is_custom = !same_directory(&active_dir, &default_dir);
    Ok(StorageLocationInfo {
        data_dir: active_dir.to_string_lossy().into_owned(),
        default_data_dir: default_dir.to_string_lossy().into_owned(),
        is_custom_data_dir: is_custom,
        log_dir: log_dir.to_string_lossy().into_owned(),
    })
}

async fn perform_data_dir_migration(
    app: &AppHandle,
    target_dir: PathBuf,
) -> Result<StorageLocationInfo, String> {
    let default_dir = default_data_dir(app)?;
    let source_dir = active_data_dir(app)?;
    let mut forbidden = Vec::new();
    if let Ok(cache) = app.path().app_cache_dir() {
        forbidden.push(cache);
    }
    if let Ok(log) = app.path().app_log_dir() {
        forbidden.push(log);
    }
    let state = app.state::<AppState>();
    let _write = state.clipboard_writes.lock().await;
    let app_clone = app.clone();
    let source_for_worker = source_dir.clone();
    let default_for_worker = default_dir.clone();
    let migrated_target =
        tauri::async_runtime::spawn_blocking(move || -> Result<PathBuf, String> {
            let state = app_clone.state::<AppState>();
            if let Some(error) = state.storage_error.lock().unwrap().clone() {
                return Err(error);
            }
            let mut db_guard = state.database.lock().unwrap();
            let db = db_guard.as_ref().ok_or("storage_unavailable")?;
            let (new_db, canonical_target) = migrate_database_and_images(
                db,
                &source_for_worker,
                &target_dir,
                &default_for_worker,
                &forbidden,
            )?;
            // Swap database handle first so the old SQLite connection closes cleanly.
            *db_guard = Some(new_db);
            drop(db_guard);
            *state.data_dir.lock().unwrap() = Some(canonical_target.clone());
            cleanup_migrated_source(&source_for_worker);
            Ok(canonical_target)
        })
        .await
        .map_err(|_| "storage_worker")??;

    let _ = migrated_target;
    let _ = clipboard_runtime::changed(app);
    crate::tray_menu::refresh_disk_usage(app, 0);
    build_location_info(app)
}

#[tauri::command]
pub async fn storage_location_info(
    window: Webview,
    app: AppHandle,
) -> Result<StorageLocationInfo, String> {
    authorize(&window, &["settings"])?;
    build_location_info(&app)
}

#[tauri::command]
pub async fn storage_open_data_dir(window: Webview, app: AppHandle) -> Result<(), String> {
    authorize(&window, &["settings"])?;
    if !platform::capabilities().storage_locations {
        return Err("platform_unavailable".into());
    }
    let dir = active_data_dir(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        ensure_private_dir(&dir)?;
        platform::open_directory(&dir)
    })
    .await
    .map_err(|_| "storage_worker")?
}

#[tauri::command]
pub async fn storage_open_log_dir(window: Webview, app: AppHandle) -> Result<(), String> {
    authorize(&window, &["settings"])?;
    if !platform::capabilities().storage_locations {
        return Err("platform_unavailable".into());
    }
    let dir = app
        .path()
        .app_log_dir()
        .map_err(|_| "storage_unavailable")?;
    tauri::async_runtime::spawn_blocking(move || {
        fs::create_dir_all(&dir).map_err(|_| "storage_unavailable")?;
        platform::open_directory(&dir)
    })
    .await
    .map_err(|_| "storage_worker")?
}

#[tauri::command]
pub async fn storage_data_dir_pick(
    window: Webview,
    app: AppHandle,
) -> Result<Option<StorageLocationInfo>, String> {
    authorize(&window, &["settings"])?;
    if !platform::capabilities().storage_locations {
        return Err("platform_unavailable".into());
    }
    let current = active_data_dir(&app)?;
    let parent = window.window();
    let (sender, receiver) = tokio::sync::oneshot::channel();
    on_main(&app, move |_| {
        platform::pick_storage_directory(&parent, &current, move |result| {
            let _ = sender.send(result);
        })
    })
    .await??;
    let Some(selected) = receiver.await.map_err(|_| "storage_dialog_failed")?? else {
        return Ok(None);
    };
    let info = perform_data_dir_migration(&app, selected).await?;
    Ok(Some(info))
}

#[tauri::command]
pub async fn storage_data_dir_reset(
    window: Webview,
    app: AppHandle,
) -> Result<StorageLocationInfo, String> {
    authorize(&window, &["settings"])?;
    if !platform::capabilities().storage_locations {
        return Err("platform_unavailable".into());
    }
    let default_dir = default_data_dir(&app)?;
    let current = active_data_dir(&app)?;
    if same_directory(&current, &default_dir) {
        let _ = persist_custom_data_dir(&default_dir, None);
        return build_location_info(&app);
    }
    perform_data_dir_migration(&app, default_dir).await
}

const AUTO_CACHE_DRAG_MAX_AGE: Duration = Duration::from_secs(30 * 60);

/// Lossless background cache maintenance (caller holds `clipboard_writes`).
/// Preserves hot thumbnails of live records and skips completely when nothing changed.
pub(crate) async fn run_auto_cache_cleanup_locked(app: &AppHandle) {
    let state = app.state::<AppState>();
    let current_rev = state.clipboard_revision.load(Ordering::SeqCst);
    let dirty = state.cache_cleanup_dirty.swap(false, Ordering::SeqCst);
    let last_rev = state.cache_cleanup_revision.load(Ordering::SeqCst);
    if !dirty && last_rev == current_rev {
        return;
    }

    let Ok(root) = clipboard_runtime::image_root(app) else {
        state.cache_cleanup_dirty.store(true, Ordering::SeqCst);
        return;
    };

    // 1. Retry any pending image deletions first.
    let _ = clipboard_preferences::retry_image_deletions_locked(app, root.clone()).await;

    // 2. Remove orphan stored images older than 60s and their stale thumbnails.
    let Ok(referenced) =
        clipboard_runtime::database(app, |db| db.referenced_clipboard_images()).await
    else {
        state.cache_cleanup_dirty.store(true, Ordering::SeqCst);
        return;
    };
    let referenced_for_orphans = referenced.clone();
    let root_for_orphans = root.clone();
    let orphan_outcome = tauri::async_runtime::spawn_blocking(move || {
        nami::database::unlink_orphan_clipboard_images(
            &root_for_orphans,
            &referenced_for_orphans,
            Duration::from_secs(60),
        )
    })
    .await;
    let (orphan_files, orphan_paths) = match orphan_outcome {
        Ok((files, _bytes, paths)) => (files, paths),
        Err(_) => (0, Vec::new()),
    };
    if !orphan_paths.is_empty() {
        crate::media::remove_thumbs_for_image_paths(app, &orphan_paths);
    }

    // 3. Prune unreferenced thumbnails and expired drag exports (>30 min) without touching hot thumbs.
    let app_for_cache = app.clone();
    let (stale_files, _) = tauri::async_runtime::spawn_blocking(move || {
        crate::media::prune_stale_caches(&app_for_cache, &referenced, AUTO_CACHE_DRAG_MAX_AGE)
    })
    .await
    .unwrap_or((0, 0));

    // 4. Reclaim unreferenced source_apps icon rows and truncate WAL.
    let (removed_sources, _) =
        clipboard_runtime::database(app, |db| db.reclaim_unreferenced_source_apps())
            .await
            .unwrap_or((0, 0));

    state
        .cache_cleanup_revision
        .store(current_rev, Ordering::SeqCst);

    if orphan_files > 0 || stale_files > 0 || removed_sources > 0 {
        crate::tray_menu::refresh_disk_usage(app, 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nami::clipboard::{self, ClipboardRecord};

    struct TempRoot(PathBuf);
    impl TempRoot {
        fn new(label: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("nami-storage-loc-{label}-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }
    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn resolves_default_and_custom_pointer_roundtrip() {
        let root = TempRoot::new("pointer");
        let default_dir = root.0.join("default");
        let custom_dir = root.0.join("custom");
        fs::create_dir_all(&default_dir).unwrap();
        fs::create_dir_all(&custom_dir).unwrap();

        assert_eq!(resolve_startup_data_dir(&default_dir), default_dir);
        persist_custom_data_dir(&default_dir, Some(&custom_dir)).unwrap();
        assert_eq!(resolve_startup_data_dir(&default_dir), custom_dir);

        persist_custom_data_dir(&default_dir, None).unwrap();
        assert_eq!(resolve_startup_data_dir(&default_dir), default_dir);
    }

    #[test]
    fn migrates_database_and_images_and_rejects_nested_or_non_empty_targets() {
        let root = TempRoot::new("migrate");
        let default_dir = root.0.join("default");
        let custom_dir = root.0.join("custom");
        let cache_dir = root.0.join("cache");
        fs::create_dir_all(&default_dir).unwrap();
        fs::create_dir_all(&custom_dir).unwrap();
        fs::create_dir_all(&cache_dir).unwrap();

        let mut db = HistoryDatabase::open(&default_dir.join(DB_FILENAME)).unwrap();
        let img_root = default_dir.join("resources").join("clipboard-images");
        let png = b"\x89PNG\r\n\x1a\nmigration-test-payload";
        let rel = clipboard::store_image(&img_root, png, "png").unwrap();
        db.clipboard_insert(&ClipboardRecord {
            id: "rec-img-1".into(),
            kind: "image".into(),
            text: String::new(),
            date: "2026-11-27T00:00:00Z".into(),
            is_favorite: true,
            is_pinned: false,
            source_app: None,
            image_path: Some(rel.clone()),
            fingerprint: clipboard::fingerprint("image", png),
        })
        .unwrap();

        // Reject same, nested, and forbidden targets.
        let nested = default_dir.join("sub");
        assert_eq!(
            validate_migration_target(&default_dir, &default_dir, std::slice::from_ref(&cache_dir))
                .unwrap_err(),
            "storage_dir_same"
        );
        assert_eq!(
            validate_migration_target(&default_dir, &nested, std::slice::from_ref(&cache_dir))
                .unwrap_err(),
            "storage_dir_nested"
        );
        assert_eq!(
            validate_migration_target(&default_dir, &cache_dir, std::slice::from_ref(&cache_dir))
                .unwrap_err(),
            "storage_dir_invalid"
        );

        // Migrate default -> custom.
        let (migrated_db, canonical_target) = migrate_database_and_images(
            &db,
            &default_dir,
            &custom_dir,
            &default_dir,
            std::slice::from_ref(&cache_dir),
        )
        .unwrap();
        drop(db);
        cleanup_migrated_source(&default_dir);

        assert_eq!(resolve_startup_data_dir(&default_dir), canonical_target);
        assert!(!default_dir.join(DB_FILENAME).exists());
        assert!(canonical_target.join(DB_FILENAME).exists());
        let restored = clipboard::read_image(
            &canonical_target.join("resources").join("clipboard-images"),
            &rel,
        )
        .unwrap();
        assert_eq!(restored, png);
        let rec = migrated_db.clipboard_record("rec-img-1").unwrap();
        assert!(rec.is_favorite);

        // Reject migrating into a directory that already has nami.sqlite3.
        let occupied = root.0.join("occupied");
        fs::create_dir_all(&occupied).unwrap();
        fs::write(occupied.join(DB_FILENAME), b"existing").unwrap();
        assert_eq!(
            migrate_database_and_images(
                &migrated_db,
                &canonical_target,
                &occupied,
                &default_dir,
                std::slice::from_ref(&cache_dir),
            )
            .err()
            .as_deref(),
            Some("storage_dir_not_empty")
        );

        // Migrate back custom -> default (reset).
        let (reset_db, reset_target) = migrate_database_and_images(
            &migrated_db,
            &canonical_target,
            &default_dir,
            &default_dir,
            &[cache_dir],
        )
        .unwrap();
        drop(migrated_db);
        cleanup_migrated_source(&canonical_target);

        assert_eq!(resolve_startup_data_dir(&default_dir), default_dir);
        assert!(!default_dir.join(LOCATION_POINTER_FILE).exists());
        assert!(reset_target.join(DB_FILENAME).exists());
        assert!(reset_db.clipboard_record("rec-img-1").is_ok());
    }
}
