use super::{storage_error, HistoryDatabase, APPLICATION_ID, VERSION};
use crate::clipboard;
use ring::{
    aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM},
    pbkdf2,
    rand::{SecureRandom, SystemRandom},
};
use rusqlite::{
    params, Connection, OpenFlags, OptionalExtension, Transaction, TransactionBehavior,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

const BACKUP_FORMAT_VERSION: &str = "1";
const MAX_BACKUP_FILE_BYTES: u64 = 2 * 1024 * 1024 * 1024;

const ENCRYPTED_BACKUP_PREFIX: &[u8; 7] = b"NAMIBAK";
const ENCRYPTED_BACKUP_VERSION: u8 = 1;
const KDF_PBKDF2_SHA256: u8 = 1;
const AEAD_AES_256_GCM: u8 = 1;
const ENCRYPTED_HEADER_LEN: usize = 72;
const AEAD_TAG_LEN: usize = 16;
const DEFAULT_PBKDF2_ITERATIONS: u32 = 600_000;
const MIN_PBKDF2_ITERATIONS: u32 = 100_000;
const MAX_PBKDF2_ITERATIONS: u32 = 5_000_000;
const DEFAULT_CHUNK_SIZE: u32 = 1024 * 1024;
const MIN_CHUNK_SIZE: u32 = 4096;
const MAX_CHUNK_SIZE: u32 = 16 * 1024 * 1024;
pub const MAX_BACKUP_PASSWORD_BYTES: usize = 1024;

pub struct TempFileGuard(pub PathBuf);

impl Drop for TempFileGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

pub fn create_restricted_temp_file(path: &Path) -> Result<std::fs::File, String> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(path)
        .map_err(|_| "backup_export_failed".into())
}

pub fn is_encrypted_backup(path: &Path) -> Result<bool, String> {
    let meta = std::fs::symlink_metadata(path).map_err(|_| "backup_invalid")?;
    if !meta.is_file() || meta.len() == 0 || meta.len() > MAX_BACKUP_FILE_BYTES {
        return Err("backup_invalid".into());
    }
    let mut file = std::fs::File::open(path).map_err(|_| "backup_invalid")?;
    let mut magic = [0u8; 8];
    if file.read(&mut magic).map_err(|_| "backup_invalid")? < 8 {
        return Ok(false);
    }
    if &magic[..7] == ENCRYPTED_BACKUP_PREFIX {
        if magic[7] > ENCRYPTED_BACKUP_VERSION {
            return Err("backup_unsupported_version".into());
        }
        if magic[7] != ENCRYPTED_BACKUP_VERSION {
            return Err("backup_invalid".into());
        }
        return Ok(true);
    }
    Ok(false)
}

fn chunk_nonce(base_nonce: &[u8; 12], chunk_index: u64) -> [u8; 12] {
    let mut nonce = *base_nonce;
    let index_bytes = chunk_index.to_be_bytes();
    for (slot, byte) in nonce[4..12].iter_mut().zip(index_bytes) {
        *slot ^= byte;
    }
    nonce
}

fn chunk_aad(
    header: &[u8; ENCRYPTED_HEADER_LEN],
    chunk_index: u64,
) -> [u8; ENCRYPTED_HEADER_LEN + 8] {
    let mut aad = [0u8; ENCRYPTED_HEADER_LEN + 8];
    aad[..ENCRYPTED_HEADER_LEN].copy_from_slice(header);
    aad[ENCRYPTED_HEADER_LEN..].copy_from_slice(&chunk_index.to_be_bytes());
    aad
}

fn derive_backup_key(
    password: &str,
    salt: &[u8; 32],
    iterations: u32,
) -> Result<Zeroizing<[u8; 32]>, String> {
    let Some(count) = NonZeroU32::new(iterations) else {
        return Err("backup_invalid".into());
    };
    let mut key = Zeroizing::new([0u8; 32]);
    pbkdf2::derive(
        pbkdf2::PBKDF2_HMAC_SHA256,
        count,
        salt,
        password.as_bytes(),
        &mut *key,
    );
    Ok(key)
}

pub fn encrypt_backup_file(
    source: &Path,
    destination: &Path,
    password: &str,
) -> Result<u64, String> {
    encrypt_backup_file_with_params(
        source,
        destination,
        password,
        DEFAULT_PBKDF2_ITERATIONS,
        DEFAULT_CHUNK_SIZE,
    )
}

fn encrypt_backup_file_with_params(
    source: &Path,
    destination: &Path,
    password: &str,
    iterations: u32,
    chunk_size: u32,
) -> Result<u64, String> {
    if password.is_empty() || password.len() > MAX_BACKUP_PASSWORD_BYTES {
        return Err("invalid_argument".into());
    }
    if !(MIN_PBKDF2_ITERATIONS..=MAX_PBKDF2_ITERATIONS).contains(&iterations)
        || !(MIN_CHUNK_SIZE..=MAX_CHUNK_SIZE).contains(&chunk_size)
    {
        return Err("invalid_argument".into());
    }
    let meta = std::fs::symlink_metadata(source).map_err(|_| "backup_export_failed")?;
    let plaintext_len = meta.len();
    if !meta.is_file() || plaintext_len == 0 || plaintext_len > MAX_BACKUP_FILE_BYTES {
        return Err("backup_export_failed".into());
    }

    let rng = SystemRandom::new();
    let mut salt = [0u8; 32];
    let mut base_nonce = [0u8; 12];
    rng.fill(&mut salt).map_err(|_| "backup_export_failed")?;
    rng.fill(&mut base_nonce)
        .map_err(|_| "backup_export_failed")?;

    let key_bytes = derive_backup_key(password, &salt, iterations)?;
    let unbound = UnboundKey::new(&AES_256_GCM, &*key_bytes).map_err(|_| "backup_export_failed")?;
    let key = LessSafeKey::new(unbound);

    let mut header = [0u8; ENCRYPTED_HEADER_LEN];
    header[..7].copy_from_slice(ENCRYPTED_BACKUP_PREFIX);
    header[7] = ENCRYPTED_BACKUP_VERSION;
    header[8] = KDF_PBKDF2_SHA256;
    header[9] = AEAD_AES_256_GCM;
    header[10..12].copy_from_slice(&[0, 0]);
    header[12..16].copy_from_slice(&iterations.to_be_bytes());
    header[16..20].copy_from_slice(&chunk_size.to_be_bytes());
    header[20..28].copy_from_slice(&plaintext_len.to_be_bytes());
    header[28..60].copy_from_slice(&salt);
    header[60..72].copy_from_slice(&base_nonce);

    let mut src = std::fs::File::open(source).map_err(|_| "backup_export_failed")?;
    let mut dst = create_restricted_temp_file(destination)?;

    let write_result = (|| -> Result<u64, String> {
        dst.write_all(&header).map_err(|_| "backup_export_failed")?;
        let mut total_written = ENCRYPTED_HEADER_LEN as u64;
        let mut remaining = plaintext_len;
        let mut chunk_index = 0u64;
        let mut buffer = Vec::with_capacity(chunk_size as usize + AEAD_TAG_LEN);

        while remaining > 0 {
            let take = remaining.min(u64::from(chunk_size)) as usize;
            buffer.resize(take, 0);
            src.read_exact(&mut buffer)
                .map_err(|_| "backup_export_failed")?;
            let nonce = Nonce::assume_unique_for_key(chunk_nonce(&base_nonce, chunk_index));
            let aad = chunk_aad(&header, chunk_index);
            key.seal_in_place_append_tag(nonce, Aad::from(&aad), &mut buffer)
                .map_err(|_| "backup_export_failed")?;
            dst.write_all(&buffer).map_err(|_| "backup_export_failed")?;
            total_written = total_written
                .checked_add(buffer.len() as u64)
                .ok_or("backup_export_failed")?;
            remaining -= take as u64;
            chunk_index += 1;
        }
        dst.sync_all().map_err(|_| "backup_export_failed")?;
        Ok(total_written)
    })();

    if write_result.is_err() {
        let _ = std::fs::remove_file(destination);
    }
    write_result
}

pub fn decrypt_backup_file(
    source: &Path,
    destination: &Path,
    password: &str,
) -> Result<(), String> {
    if password.is_empty() || password.len() > MAX_BACKUP_PASSWORD_BYTES {
        return Err("backup_wrong_password".into());
    }
    let meta = std::fs::symlink_metadata(source).map_err(|_| "backup_invalid")?;
    let file_len = meta.len();
    if !meta.is_file()
        || file_len <= ENCRYPTED_HEADER_LEN as u64
        || file_len > MAX_BACKUP_FILE_BYTES
    {
        return Err("backup_invalid".into());
    }

    let mut src = std::fs::File::open(source).map_err(|_| "backup_invalid")?;
    let mut header = [0u8; ENCRYPTED_HEADER_LEN];
    src.read_exact(&mut header).map_err(|_| "backup_invalid")?;

    if &header[..7] != ENCRYPTED_BACKUP_PREFIX {
        return Err("backup_invalid".into());
    }
    if header[7] > ENCRYPTED_BACKUP_VERSION {
        return Err("backup_unsupported_version".into());
    }
    if header[7] != ENCRYPTED_BACKUP_VERSION
        || header[8] != KDF_PBKDF2_SHA256
        || header[9] != AEAD_AES_256_GCM
        || header[10..12] != [0, 0]
    {
        return Err("backup_invalid".into());
    }
    let iterations = u32::from_be_bytes(header[12..16].try_into().unwrap());
    let chunk_size = u32::from_be_bytes(header[16..20].try_into().unwrap());
    let plaintext_len = u64::from_be_bytes(header[20..28].try_into().unwrap());
    if !(MIN_PBKDF2_ITERATIONS..=MAX_PBKDF2_ITERATIONS).contains(&iterations)
        || !(MIN_CHUNK_SIZE..=MAX_CHUNK_SIZE).contains(&chunk_size)
        || plaintext_len == 0
        || plaintext_len > MAX_BACKUP_FILE_BYTES
    {
        return Err("backup_invalid".into());
    }
    let total_chunks = plaintext_len.div_ceil(u64::from(chunk_size));
    let expected_len = (ENCRYPTED_HEADER_LEN as u64)
        .checked_add(plaintext_len)
        .and_then(|v| v.checked_add(total_chunks.checked_mul(AEAD_TAG_LEN as u64)?))
        .ok_or("backup_invalid")?;
    if file_len != expected_len {
        return Err("backup_invalid".into());
    }

    let salt: [u8; 32] = header[28..60].try_into().unwrap();
    let base_nonce: [u8; 12] = header[60..72].try_into().unwrap();
    let key_bytes = derive_backup_key(password, &salt, iterations)?;
    let unbound = UnboundKey::new(&AES_256_GCM, &*key_bytes).map_err(|_| "backup_invalid")?;
    let key = LessSafeKey::new(unbound);

    let mut dst = create_restricted_temp_file(destination).map_err(|_| "backup_import_failed")?;
    let decrypt_result = (|| -> Result<(), String> {
        let mut remaining = plaintext_len;
        let mut chunk_index = 0u64;
        let mut buffer = Vec::with_capacity(chunk_size as usize + AEAD_TAG_LEN);

        while remaining > 0 {
            let plain_take = remaining.min(u64::from(chunk_size)) as usize;
            let cipher_take = plain_take + AEAD_TAG_LEN;
            buffer.resize(cipher_take, 0);
            src.read_exact(&mut buffer).map_err(|_| "backup_invalid")?;
            let nonce = Nonce::assume_unique_for_key(chunk_nonce(&base_nonce, chunk_index));
            let aad = chunk_aad(&header, chunk_index);
            let plaintext = key
                .open_in_place(nonce, Aad::from(&aad), &mut buffer)
                .map_err(|_| "backup_wrong_password")?;
            if plaintext.len() != plain_take {
                return Err("backup_invalid".into());
            }
            dst.write_all(plaintext)
                .map_err(|_| "backup_import_failed")?;
            remaining -= plain_take as u64;
            chunk_index += 1;
        }
        dst.sync_all().map_err(|_| "backup_import_failed")?;
        Ok(())
    })();

    if decrypt_result.is_err() {
        let _ = std::fs::remove_file(destination);
    }
    decrypt_result
}

pub fn inspect_backup_with_password(
    path: &Path,
    password: Option<&str>,
) -> Result<BackupSummary, String> {
    if is_encrypted_backup(path)? {
        let password = password
            .filter(|p| !p.is_empty())
            .ok_or("backup_password_required")?;
        let encrypted_size = std::fs::symlink_metadata(path)
            .map_err(|_| "backup_invalid")?
            .len();
        let staging = TempFileGuard(std::env::temp_dir().join(format!(
            "nami-backup-inspect-{}.sqlite3",
            uuid::Uuid::new_v4().simple()
        )));
        decrypt_backup_file(path, &staging.0, password)?;
        let mut summary = inspect_backup(&staging.0)?;
        summary.file_size = encrypted_size;
        Ok(summary)
    } else {
        inspect_backup(path)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BackupImportMode {
    Merge,
    Replace,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupSummary {
    pub exported_at: String,
    pub clipboard_count: usize,
    pub image_count: usize,
    pub file_size: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupImportReport {
    pub mode: BackupImportMode,
    pub imported_clipboard: usize,
    pub skipped_clipboard: usize,
    pub restored_images: usize,
}

fn backup_error(error: rusqlite::Error) -> String {
    match error.sqlite_error_code() {
        Some(
            rusqlite::ErrorCode::NotADatabase
            | rusqlite::ErrorCode::DatabaseCorrupt
            | rusqlite::ErrorCode::SchemaChanged,
        ) => "backup_invalid".into(),
        _ => storage_error(error),
    }
}

fn open_validated_backup(path: &Path) -> Result<(Connection, u64), String> {
    let meta = std::fs::symlink_metadata(path).map_err(|_| "backup_invalid")?;
    if !meta.is_file() || meta.len() == 0 || meta.len() > MAX_BACKUP_FILE_BYTES {
        return Err("backup_invalid".into());
    }
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = Connection::open_with_flags(path, flags).map_err(|_| "backup_invalid")?;
    let app_id: i64 = conn
        .pragma_query_value(None, "application_id", |row| row.get(0))
        .map_err(|_| "backup_invalid")?;
    if app_id != APPLICATION_ID {
        return Err("backup_invalid".into());
    }
    let version: i64 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(|_| "backup_invalid")?;
    if version > VERSION {
        return Err("backup_unsupported_version".into());
    }
    if version < 10 {
        return Err("backup_invalid".into());
    }
    let quick_check: String = conn
        .pragma_query_value(None, "quick_check", |row| row.get(0))
        .map_err(|_| "backup_invalid")?;
    if quick_check != "ok" {
        return Err("backup_invalid".into());
    }
    let format_version: Option<String> = conn
        .query_row(
            "SELECT value FROM _nami_backup_meta WHERE key = 'format_version'",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(|_| "backup_invalid")?;
    if format_version.as_deref() != Some(BACKUP_FORMAT_VERSION) {
        return Err("backup_invalid".into());
    }
    Ok((conn, meta.len()))
}

pub fn inspect_backup(path: &Path) -> Result<BackupSummary, String> {
    let (conn, file_size) = open_validated_backup(path)?;
    let exported_at: String = conn
        .query_row(
            "SELECT value FROM _nami_backup_meta WHERE key = 'exported_at'",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(backup_error)?
        .unwrap_or_default();
    let clipboard_count: i64 = conn
        .query_row("SELECT count(*) FROM clipboard_records", [], |row| {
            row.get(0)
        })
        .map_err(backup_error)?;
    let image_count: i64 = conn
        .query_row("SELECT count(*) FROM _nami_backup_images", [], |row| {
            row.get(0)
        })
        .map_err(backup_error)?;
    Ok(BackupSummary {
        exported_at,
        clipboard_count: clipboard_count.max(0) as usize,
        image_count: image_count.max(0) as usize,
        file_size,
    })
}

fn collect_referenced_images(conn: &Connection) -> Result<HashSet<String>, String> {
    let mut paths = HashSet::new();
    for sql in [
        "SELECT DISTINCT image_path FROM clipboard_records WHERE image_path IS NOT NULL",
        "SELECT DISTINCT image_path FROM clipboard_file_representations WHERE image_path IS NOT NULL",
    ] {
        let mut stmt = conn.prepare(sql).map_err(storage_error)?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(storage_error)?;
        for item in rows {
            paths.insert(item.map_err(storage_error)?);
        }
    }
    Ok(paths)
}

fn resolve_source_app(tx: &Transaction<'_>, payload: &str) -> Result<i64, String> {
    tx.execute(
        "INSERT OR IGNORE INTO source_apps(payload) VALUES(?1)",
        params![payload],
    )
    .map_err(storage_error)?;
    tx.query_row(
        "SELECT id FROM source_apps WHERE payload = ?1",
        params![payload],
        |row| row.get(0),
    )
    .map_err(storage_error)
}

fn load_source_map(conn: &Connection) -> Result<HashMap<i64, String>, String> {
    let mut stmt = conn
        .prepare("SELECT id, payload FROM source_apps")
        .map_err(backup_error)?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(backup_error)?;
    let mut map = HashMap::new();
    for row in rows {
        let (id, payload) = row.map_err(backup_error)?;
        map.insert(id, payload);
    }
    Ok(map)
}

fn validate_and_restore_images(backup: &Connection, image_root: &Path) -> Result<usize, String> {
    let mut stmt = backup
        .prepare("SELECT image_path, sha256, data FROM _nami_backup_images")
        .map_err(backup_error)?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Vec<u8>>(2)?,
            ))
        })
        .map_err(backup_error)?;
    let mut restored = 0usize;
    for row in rows {
        let (image_path, sha256, data) = row.map_err(backup_error)?;
        if data.len() > clipboard::MAX_IMAGE_BYTES {
            return Err("backup_invalid".into());
        }
        let extension =
            clipboard::image_extension_from_path(&image_path).ok_or("backup_invalid")?;
        if clipboard::fingerprint("image", &data) != sha256 {
            return Err("backup_invalid".into());
        }
        let (stored_path, created) =
            clipboard::store_image_with_status(image_root, &data, extension)
                .map_err(|_| "backup_invalid")?;
        if stored_path != image_path {
            return Err("backup_invalid".into());
        }
        if created {
            restored += 1;
        }
    }
    Ok(restored)
}

impl HistoryDatabase {
    pub fn export_backup(
        &self,
        image_root: &Path,
        staging_path: &Path,
    ) -> Result<BackupSummary, String> {
        let staging_str = staging_path.to_str().ok_or("backup_export_failed")?;
        if staging_path.exists() {
            let _ = std::fs::remove_file(staging_path);
        }
        self.connection
            .execute("VACUUM INTO ?1", params![staging_str])
            .map_err(storage_error)?;

        let result = (|| -> Result<BackupSummary, String> {
            let mut backup = Connection::open(staging_path).map_err(storage_error)?;
            backup
                .pragma_update(None, "journal_mode", "DELETE")
                .map_err(storage_error)?;
            backup
                .pragma_update(None, "synchronous", "FULL")
                .map_err(storage_error)?;
            backup
                .pragma_update(None, "foreign_keys", "ON")
                .map_err(storage_error)?;

            let referenced = collect_referenced_images(&backup)?;
            let mut image_blobs: Vec<(String, String, Vec<u8>)> = Vec::new();
            for relative in referenced {
                match clipboard::read_image(image_root, &relative) {
                    Ok(bytes) => {
                        let hash = clipboard::fingerprint("image", &bytes);
                        image_blobs.push((relative, hash, bytes));
                    }
                    Err(code) if code == "clipboard_image_missing" => continue,
                    Err(code) => return Err(code),
                }
            }

            let exported_at = chrono::Utc::now().to_rfc3339();
            let clipboard_count: i64 = backup
                .query_row("SELECT count(*) FROM clipboard_records", [], |row| {
                    row.get(0)
                })
                .map_err(storage_error)?;

            let tx = backup
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(storage_error)?;
            tx.execute_batch(
                "DELETE FROM clipboard_pending_image_deletions;
                CREATE TABLE _nami_backup_meta (
                    key TEXT PRIMARY KEY NOT NULL,
                    value TEXT NOT NULL
                );
                CREATE TABLE _nami_backup_images (
                    image_path TEXT PRIMARY KEY NOT NULL,
                    sha256 TEXT NOT NULL,
                    data BLOB NOT NULL
                );",
            )
            .map_err(storage_error)?;
            for (path, sha256, data) in &image_blobs {
                tx.execute(
                    "INSERT INTO _nami_backup_images(image_path, sha256, data) VALUES(?1, ?2, ?3)",
                    params![path, sha256, data],
                )
                .map_err(storage_error)?;
            }
            let meta_entries = [
                ("format_version", BACKUP_FORMAT_VERSION.to_owned()),
                ("app_version", env!("CARGO_PKG_VERSION").to_owned()),
                ("schema_version", VERSION.to_string()),
                ("exported_at", exported_at.clone()),
                ("clipboard_count", clipboard_count.to_string()),
                ("image_count", image_blobs.len().to_string()),
            ];
            for (key, value) in meta_entries {
                tx.execute(
                    "INSERT INTO _nami_backup_meta(key, value) VALUES(?1, ?2)",
                    params![key, value],
                )
                .map_err(storage_error)?;
            }
            tx.commit().map_err(storage_error)?;
            drop(backup);

            let file_size = std::fs::metadata(staging_path)
                .map_err(|_| "backup_export_failed")?
                .len();
            Ok(BackupSummary {
                exported_at,
                clipboard_count: clipboard_count.max(0) as usize,
                image_count: image_blobs.len(),
                file_size,
            })
        })();

        if result.is_err() {
            let _ = std::fs::remove_file(staging_path);
        }
        result
    }

    pub fn import_backup_with_password(
        &mut self,
        backup_path: &Path,
        image_root: &Path,
        mode: BackupImportMode,
        password: Option<&str>,
    ) -> Result<BackupImportReport, String> {
        if is_encrypted_backup(backup_path)? {
            let password = password
                .filter(|p| !p.is_empty())
                .ok_or("backup_password_required")?;
            let staging = TempFileGuard(std::env::temp_dir().join(format!(
                "nami-backup-import-{}.sqlite3",
                uuid::Uuid::new_v4().simple()
            )));
            decrypt_backup_file(backup_path, &staging.0, password)?;
            self.import_backup(&staging.0, image_root, mode)
        } else {
            self.import_backup(backup_path, image_root, mode)
        }
    }

    pub fn import_backup(
        &mut self,
        backup_path: &Path,
        image_root: &Path,
        mode: BackupImportMode,
    ) -> Result<BackupImportReport, String> {
        let (backup, _) = open_validated_backup(backup_path)?;
        let restored_images = validate_and_restore_images(&backup, image_root)?;
        let source_map = load_source_map(&backup)?;
        let old_images = if mode == BackupImportMode::Replace {
            collect_referenced_images(&self.connection)?
        } else {
            HashSet::new()
        };

        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage_error)?;

        if mode == BackupImportMode::Replace {
            tx.execute_batch(
                "DELETE FROM clipboard_records;
                DELETE FROM source_apps;
                DELETE FROM preferences;",
            )
            .map_err(storage_error)?;
        }

        let mut imported_clipboard = 0usize;
        let has_dir_paths =
            super::clipboard::has_column(&backup, "clipboard_contents", "directory_paths")?;
        let dir_paths_col = if has_dir_paths {
            "c.directory_paths"
        } else {
            "NULL"
        };
        let mut skipped_clipboard = 0usize;
        {
            let sql = format!(
                "SELECT r.sequence, r.id, r.kind, r.summary, r.title, r.date, r.created_at, r.is_favorite, r.is_pinned, r.source_app_id, r.image_path, r.fingerprint,
                        c.text, c.search_folded, {dir_paths_col},
                        f.paths, f.image_path,
                        t.search_folded,
                        o.original_text, o.search_folded,
                        fmt.html, fmt.rtf
                 FROM clipboard_records r
                 JOIN clipboard_contents c ON c.sequence = r.sequence
                 LEFT JOIN clipboard_file_representations f ON f.sequence = r.sequence
                 LEFT JOIN clipboard_titles t ON t.sequence = r.sequence
                 LEFT JOIN clipboard_ocr o ON o.sequence = r.sequence
                 LEFT JOIN clipboard_text_formatting fmt ON fmt.sequence = r.sequence
                 ORDER BY r.sequence ASC"
            );
            let mut stmt = backup.prepare(&sql).map_err(backup_error)?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, i64>(6)?,
                        row.get::<_, i64>(7)?,
                        row.get::<_, i64>(8)?,
                        row.get::<_, Option<i64>>(9)?,
                        row.get::<_, Option<String>>(10)?,
                        row.get::<_, String>(11)?,
                        row.get::<_, String>(12)?,
                        row.get::<_, String>(13)?,
                        row.get::<_, Option<String>>(14)?,
                        row.get::<_, Option<String>>(15)?,
                        row.get::<_, Option<String>>(16)?,
                        row.get::<_, Option<String>>(17)?,
                        row.get::<_, Option<String>>(18)?,
                        row.get::<_, Option<String>>(19)?,
                        row.get::<_, Option<Vec<u8>>>(20)?,
                        row.get::<_, Option<Vec<u8>>>(21)?,
                    ))
                })
                .map_err(backup_error)?;
            for row in rows {
                let (
                    id,
                    kind,
                    summary,
                    title,
                    date,
                    created_at,
                    is_favorite,
                    is_pinned,
                    src_id,
                    image_path,
                    fingerprint,
                    text,
                    search_folded,
                    directory_paths,
                    rep_paths,
                    rep_image_path,
                    title_folded,
                    ocr_original,
                    ocr_folded,
                    html,
                    rtf,
                ) = row.map_err(backup_error)?;

                if mode == BackupImportMode::Merge {
                    let dup_by_id: bool = tx
                        .query_row(
                            "SELECT EXISTS(SELECT 1 FROM clipboard_records WHERE id = ?1)",
                            params![&id],
                            |r| r.get(0),
                        )
                        .map_err(storage_error)?;
                    let dup_by_content: bool = if dup_by_id {
                        true
                    } else {
                        tx.query_row(
                            "SELECT EXISTS(
                                SELECT 1 FROM clipboard_records r
                                JOIN clipboard_contents c ON c.sequence = r.sequence
                                WHERE r.kind = ?1 AND r.fingerprint = ?2 AND c.text = ?3
                                  AND COALESCE(r.image_path, '') = COALESCE(?4, '')
                            )",
                            params![&kind, &fingerprint, &text, &image_path],
                            |r| r.get(0),
                        )
                        .map_err(storage_error)?
                    };
                    if dup_by_content {
                        skipped_clipboard += 1;
                        continue;
                    }
                }

                let mapped_src = match src_id.and_then(|sid| source_map.get(&sid)) {
                    Some(payload) => Some(resolve_source_app(&tx, payload)?),
                    None => None,
                };
                tx.execute(
                    "INSERT INTO clipboard_records(id, kind, summary, title, date, created_at, is_favorite, is_pinned, source_app_id, image_path, fingerprint)
                     VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                    params![
                        id,
                        kind,
                        summary,
                        title,
                        date,
                        created_at,
                        is_favorite,
                        is_pinned,
                        mapped_src,
                        image_path,
                        fingerprint
                    ],
                )
                .map_err(storage_error)?;
                let sequence = tx.last_insert_rowid();
                tx.execute(
                    "INSERT INTO clipboard_contents(sequence, text, search_folded, directory_paths) VALUES(?1, ?2, ?3, ?4)",
                    params![sequence, text, search_folded, directory_paths],
                )
                .map_err(storage_error)?;
                if let (Some(paths), Some(rep_img)) = (rep_paths, rep_image_path) {
                    tx.execute(
                        "INSERT INTO clipboard_file_representations(sequence, paths, image_path) VALUES(?1, ?2, ?3)",
                        params![sequence, paths, rep_img],
                    )
                    .map_err(storage_error)?;
                }
                if let Some(folded) = title_folded {
                    tx.execute(
                        "INSERT INTO clipboard_titles(sequence, search_folded) VALUES(?1, ?2)",
                        params![sequence, folded],
                    )
                    .map_err(storage_error)?;
                }
                if let Some(folded) = ocr_folded {
                    tx.execute(
                        "INSERT INTO clipboard_ocr(sequence, search_folded, original_text) VALUES(?1, ?2, ?3)",
                        params![sequence, folded, ocr_original],
                    )
                    .map_err(storage_error)?;
                }
                if html.is_some() || rtf.is_some() {
                    tx.execute(
                        "INSERT INTO clipboard_text_formatting(sequence, html, rtf) VALUES(?1, ?2, ?3)",
                        params![sequence, html, rtf],
                    )
                    .map_err(storage_error)?;
                }
                imported_clipboard += 1;
            }
        }

        if mode == BackupImportMode::Replace {
            let mut stmt = backup
                .prepare("SELECT key, value FROM preferences")
                .map_err(backup_error)?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(backup_error)?;
            for row in rows {
                let (key, value) = row.map_err(backup_error)?;
                if key == "translation_proxy_url"
                    && crate::translation::proxy_contains_credentials(&value)
                {
                    continue;
                }
                tx.execute(
                    "INSERT OR REPLACE INTO preferences(key, value) VALUES(?1, ?2)",
                    params![key, value],
                )
                .map_err(storage_error)?;
            }

            // Enqueue any previously referenced images that are no longer referenced after Replace.
            for old_path in old_images {
                let still_used: bool = tx
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM clipboard_records WHERE image_path = ?1)
                            OR EXISTS(SELECT 1 FROM clipboard_file_representations WHERE image_path = ?1)",
                        params![&old_path],
                        |r| r.get(0),
                    )
                    .map_err(storage_error)?;
                if !still_used {
                    tx.execute(
                        "INSERT OR IGNORE INTO clipboard_pending_image_deletions(image_path) VALUES(?1)",
                        params![old_path],
                    )
                    .map_err(storage_error)?;
                }
            }
        }

        if mode == BackupImportMode::Replace
            && super::clipboard::has_column(&backup, "clipboard_records", "pin_paste_key")?
        {
            let mut stmt = backup
                .prepare(
                    "SELECT id, pin_paste_key FROM clipboard_records WHERE pin_paste_key IS NOT NULL",
                )
                .map_err(backup_error)?;
            let saved = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(backup_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(backup_error)?;
            drop(stmt);
            let mut used = HashSet::new();
            for (id, key) in saved {
                if crate::shortcuts::normalize_pin_paste_letter(&key).is_none()
                    || !used.insert(key.clone())
                {
                    continue;
                }
                tx.execute(
                    "UPDATE clipboard_records SET pin_paste_key=?1 WHERE id=?2 AND is_pinned=1",
                    params![key, id],
                )
                .map_err(storage_error)?;
            }
        }
        super::clipboard::sync_pin_paste_keys(&tx)?;

        tx.commit().map_err(storage_error)?;

        let clipboard_total: i64 = self
            .connection
            .query_row("SELECT count(*) FROM clipboard_records", [], |r| r.get(0))
            .map_err(storage_error)?;
        self.clipboard_total_records = clipboard_total.max(0) as usize;

        Ok(BackupImportReport {
            mode,
            imported_clipboard,
            skipped_clipboard,
            restored_images,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_png() -> Vec<u8> {
        // Minimal valid 1x1 PNG signature + chunks.
        vec![
            0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, b'I', b'H',
            b'D', b'R', 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00,
            0x00, 0x90, 0x77, 0x53, 0xDE, 0x00, 0x00, 0x00, 0x0C, b'I', b'D', b'A', b'T', 0x08,
            0xD7, 0x63, 0xF8, 0xFF, 0xFF, 0x3F, 0x00, 0x05, 0xFE, 0x02, 0xFE, 0xDC, 0xCC, 0x59,
            0xE7, 0x00, 0x00, 0x00, 0x00, b'I', b'E', b'N', b'D', 0xAE, 0x42, 0x60, 0x82,
        ]
    }

    fn clip_text(id: &str, text: &str) -> clipboard::ClipboardRecord {
        clipboard::ClipboardRecord {
            id: id.into(),
            kind: "text".into(),
            text: text.into(),
            date: "2026-11-24T10:01:00Z".into(),
            is_favorite: false,
            is_pinned: false,
            source_app: Some(serde_json::json!({"name": "Notes", "bundleId": "com.apple.Notes"})),
            image_path: None,
            fingerprint: clipboard::fingerprint("text", text.as_bytes()),
        }
    }

    fn clip_image(id: &str, rel_img: &str, png: &[u8]) -> clipboard::ClipboardRecord {
        clipboard::ClipboardRecord {
            id: id.into(),
            kind: "image".into(),
            text: String::new(),
            date: "2026-11-24T10:02:00Z".into(),
            is_favorite: true,
            is_pinned: true,
            source_app: None,
            image_path: Some(rel_img.into()),
            fingerprint: clipboard::fingerprint("image", png),
        }
    }

    #[test]
    fn backup_export_merge_replace_and_corruption_guards_work() {
        let root = std::env::temp_dir().join(format!("nami-backup-test-{}", uuid::Uuid::new_v4()));
        let src_images = root.join("src_images");
        let dst_images = root.join("dst_images");
        std::fs::create_dir_all(&src_images).unwrap();
        std::fs::create_dir_all(&dst_images).unwrap();

        let src_db_path = root.join("src.sqlite3");
        let mut src = HistoryDatabase::open(&src_db_path).unwrap();
        src.clipboard_insert(&clip_text("cb-1", "clip text one"))
            .unwrap();
        let png = sample_png();
        let rel_img = clipboard::store_image(&src_images, &png, "png").unwrap();
        src.clipboard_insert(&clip_image("cb-2", &rel_img, &png))
            .unwrap();
        src.preference_set("ui_locale", "zh-CN").unwrap();
        src.preference_set("window_position_mode", "cursor")
            .unwrap();

        let backup_path = root.join("export.namibak");
        let summary = src.export_backup(&src_images, &backup_path).unwrap();
        assert_eq!(summary.clipboard_count, 2);
        assert_eq!(summary.image_count, 1);
        assert!(summary.file_size > 0);

        let inspected = inspect_backup(&backup_path).unwrap();
        assert_eq!(inspected.clipboard_count, 2);
        assert_eq!(inspected.image_count, 1);

        // Simulate legacy v10 backup containing a translation_records table; import must safely ignore it.
        let legacy_path = root.join("legacy-v10.namibak");
        std::fs::copy(&backup_path, &legacy_path).unwrap();
        {
            let conn = Connection::open(&legacy_path).unwrap();
            conn.execute_batch(
                "CREATE TABLE translation_records(id TEXT PRIMARY KEY);
                 INSERT INTO translation_records(id) VALUES('legacy-tr');
                 PRAGMA user_version = 10;",
            )
            .unwrap();
        }
        let legacy_summary = inspect_backup(&legacy_path).unwrap();
        assert_eq!(legacy_summary.clipboard_count, 2);

        // Merge into a destination DB with one overlapping text and different locale.
        let dst_db_path = root.join("dst.sqlite3");
        let mut dst = HistoryDatabase::open(&dst_db_path).unwrap();
        dst.clipboard_insert(&clip_text("cb-local-dup", "clip text one"))
            .unwrap();
        dst.clipboard_insert(&clip_text("cb-local-only", "existing local only"))
            .unwrap();
        dst.preference_set("ui_locale", "en").unwrap();

        let merge = dst
            .import_backup(&legacy_path, &dst_images, BackupImportMode::Merge)
            .unwrap();
        assert_eq!(merge.imported_clipboard, 1);
        assert_eq!(merge.skipped_clipboard, 1);
        assert_eq!(merge.restored_images, 1);
        // Merge preserves local preferences.
        assert_eq!(dst.preference("ui_locale").unwrap().as_deref(), Some("en"));
        // Restored image is readable on disk with valid hash.
        assert_eq!(clipboard::read_image(&dst_images, &rel_img).unwrap(), png);

        // Replace overwrites records and preferences cleanly.
        let replace = dst
            .import_backup(&backup_path, &dst_images, BackupImportMode::Replace)
            .unwrap();
        assert_eq!(replace.imported_clipboard, 2);
        assert_eq!(
            dst.preference("ui_locale").unwrap().as_deref(),
            Some("zh-CN")
        );
        assert_eq!(
            dst.preference("window_position_mode").unwrap().as_deref(),
            Some("cursor")
        );

        // Invalid / garbage file is rejected without touching destination DB.
        let bad_path = root.join("bad.namibak");
        std::fs::write(&bad_path, b"not a sqlite file").unwrap();
        assert_eq!(inspect_backup(&bad_path).unwrap_err(), "backup_invalid");
        assert_eq!(
            dst.import_backup(&bad_path, &dst_images, BackupImportMode::Merge)
                .unwrap_err(),
            "backup_invalid"
        );

        // Tampered image path (path traversal) is rejected before mutating destination DB.
        let tampered_path = root.join("tampered.namibak");
        std::fs::copy(&backup_path, &tampered_path).unwrap();
        {
            let conn = Connection::open(&tampered_path).unwrap();
            conn.execute(
                "UPDATE _nami_backup_images SET image_path = '../../escape.png'",
                [],
            )
            .unwrap();
        }
        assert_eq!(
            dst.import_backup(&tampered_path, &dst_images, BackupImportMode::Merge)
                .unwrap_err(),
            "backup_invalid"
        );

        drop(src);
        drop(dst);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn encrypted_backup_round_trips_multi_chunk_and_rejects_wrong_password_or_tampering() {
        let root = std::env::temp_dir().join(format!(
            "nami-encrypted-backup-test-{}",
            uuid::Uuid::new_v4()
        ));
        let src_images = root.join("src_images");
        let dst_images = root.join("dst_images");
        std::fs::create_dir_all(&src_images).unwrap();
        std::fs::create_dir_all(&dst_images).unwrap();

        let mut src = HistoryDatabase::open(&root.join("src.sqlite3")).unwrap();
        src.preference_set("ui_locale", "zh-CN").unwrap();
        let png = b"\x89PNG\r\n\x1a\nencrypted-backup-payload".repeat(300);
        let rel_img = clipboard::store_image(&src_images, &png, "png").unwrap();
        src.clipboard_insert(&clipboard::ClipboardRecord {
            id: "enc-img".into(),
            kind: "image".into(),
            text: String::new(),
            date: "2026-11-24T12:00:00.000Z".into(),
            is_favorite: true,
            is_pinned: false,
            source_app: None,
            image_path: Some(rel_img.clone()),
            fingerprint: clipboard::fingerprint("image", &png),
        })
        .unwrap();
        src.clipboard_insert_capture(
            &clipboard::ClipboardRecord {
                id: "enc-rich".into(),
                kind: "text".into(),
                text: "Secret note".into(),
                date: "2026-11-24T12:01:00.000Z".into(),
                is_favorite: false,
                is_pinned: true,
                source_app: None,
                image_path: None,
                fingerprint: clipboard::fingerprint("text", b"Secret note"),
            },
            None,
            Some(&clipboard::ClipboardTextFormatting {
                html: Some(b"<strong>Secret note</strong>".to_vec()),
                rtf: None,
            }),
        )
        .unwrap();

        let plain_path = root.join("plain.namibak");
        let enc_path = root.join("encrypted.namibak");
        src.export_backup(&src_images, &plain_path).unwrap();
        assert!(!is_encrypted_backup(&plain_path).unwrap());

        // Use MIN_PBKDF2_ITERATIONS + 4096-byte chunks so multi-chunk AEAD is exercised fast.
        let enc_size = encrypt_backup_file_with_params(
            &plain_path,
            &enc_path,
            "correct-horse-battery-staple",
            MIN_PBKDF2_ITERATIONS,
            4096,
        )
        .unwrap();
        assert!(enc_size > ENCRYPTED_HEADER_LEN as u64);
        assert!(is_encrypted_backup(&enc_path).unwrap());

        // Missing or wrong password fails cleanly.
        assert_eq!(
            inspect_backup_with_password(&enc_path, None).unwrap_err(),
            "backup_password_required"
        );
        assert_eq!(
            inspect_backup_with_password(&enc_path, Some("wrong-password")).unwrap_err(),
            "backup_wrong_password"
        );

        let summary =
            inspect_backup_with_password(&enc_path, Some("correct-horse-battery-staple")).unwrap();
        assert_eq!(summary.clipboard_count, 2);
        assert_eq!(summary.image_count, 1);
        assert_eq!(summary.file_size, enc_size);

        // Tampered ciphertext and truncated/extended files are rejected.
        let tampered = root.join("tampered.namibak");
        let mut raw = std::fs::read(&enc_path).unwrap();
        raw[ENCRYPTED_HEADER_LEN + 10] ^= 0xFF;
        std::fs::write(&tampered, &raw).unwrap();
        assert_eq!(
            inspect_backup_with_password(&tampered, Some("correct-horse-battery-staple"))
                .unwrap_err(),
            "backup_wrong_password"
        );

        let truncated = root.join("truncated.namibak");
        let mut raw = std::fs::read(&enc_path).unwrap();
        raw.pop();
        std::fs::write(&truncated, &raw).unwrap();
        assert_eq!(
            inspect_backup_with_password(&truncated, Some("correct-horse-battery-staple"))
                .unwrap_err(),
            "backup_invalid"
        );

        // Replace import restores encrypted backup intact and preserves destination on failure.
        let mut dst = HistoryDatabase::open(&root.join("dst.sqlite3")).unwrap();
        assert_eq!(
            dst.import_backup_with_password(
                &enc_path,
                &dst_images,
                BackupImportMode::Replace,
                Some("wrong-password"),
            )
            .unwrap_err(),
            "backup_wrong_password"
        );
        let report = dst
            .import_backup_with_password(
                &enc_path,
                &dst_images,
                BackupImportMode::Replace,
                Some("correct-horse-battery-staple"),
            )
            .unwrap();
        assert_eq!(report.imported_clipboard, 2);
        assert_eq!(report.restored_images, 1);
        assert_eq!(clipboard::read_image(&dst_images, &rel_img).unwrap(), png);
        assert_eq!(
            dst.clipboard_detail("enc-rich").unwrap().rich_text_format,
            Some("html")
        );
        assert_eq!(
            dst.preference("ui_locale").unwrap().as_deref(),
            Some("zh-CN")
        );

        // Plaintext backup remains backward compatible through import_backup_with_password.
        let plain_report = dst
            .import_backup_with_password(&plain_path, &dst_images, BackupImportMode::Merge, None)
            .unwrap();
        assert_eq!(plain_report.skipped_clipboard, 2);

        // Backup export and import preserve directory_paths for file records and verbatim JPEG images.
        let jpeg = include_bytes!("../../../tests/fixtures/clipboard-wide.jpg");
        let rel_jpg = clipboard::store_image(&src_images, jpeg, "jpg").unwrap();
        assert!(rel_jpg.ends_with(".jpg"));
        let jpg_record = clipboard::ClipboardRecord {
            id: "jpg-backup".into(),
            kind: "image".into(),
            fingerprint: clipboard::fingerprint("image", jpeg),
            text: String::new(),
            date: "2026-11-27T00:00:00Z".into(),
            is_favorite: false,
            is_pinned: false,
            source_app: None,
            image_path: Some(rel_jpg.clone()),
        };
        src.clipboard_insert_capture(&jpg_record, None, None)
            .unwrap();
        let folder_text = serde_json::to_string(&vec!["/synthetic/deleted-folder"]).unwrap();
        let folder_record = clipboard::ClipboardRecord {
            id: "dir-backup".into(),
            kind: "files".into(),
            fingerprint: clipboard::fingerprint("files", folder_text.as_bytes()),
            text: folder_text,
            date: "2026-11-27T00:00:00Z".into(),
            is_favorite: false,
            is_pinned: false,
            source_app: None,
            image_path: None,
        };
        src.clipboard_insert_capture_with_dirs(
            &folder_record,
            None,
            None,
            Some(&["/synthetic/deleted-folder".into()]),
        )
        .unwrap();
        let dir_backup_path = root.join("dir.namibak");
        src.export_backup(&src_images, &dir_backup_path).unwrap();
        let mut dir_dst = HistoryDatabase::open(&root.join("dir-dst.sqlite3")).unwrap();
        let dir_dst_images = root.join("dir-dst-images");
        std::fs::create_dir_all(&dir_dst_images).unwrap();
        dir_dst
            .import_backup(&dir_backup_path, &dir_dst_images, BackupImportMode::Replace)
            .unwrap();
        assert_eq!(
            dir_dst.clipboard_directory_paths("dir-backup").unwrap(),
            vec!["/synthetic/deleted-folder"]
        );
        assert_eq!(
            clipboard::read_image(&dir_dst_images, &rel_jpg).unwrap(),
            jpeg
        );

        drop(src);
        drop(dst);
        let _ = std::fs::remove_dir_all(root);
    }
}
