//! Image files in the app data folder (`docs/specs/library.md`, 이미지 파일의
//! 수명; `docs/adr/0008-trss-folder-and-app-data-files.md`).
//!
//! The app data folder is the folder of the database. Images live in
//! `artwork/` under names the app makes up (`<uuid>.<ext>`); an upload's
//! name, a URL or a provider ID never names a file.
//!
//! # Publishing
//!
//! 1. The file is recorded in `artwork_files` (`staging`) before it exists.
//! 2. The verified bytes are written to a new file in `artwork/.staging/`
//!    (created exclusively, flushed), and its identity (`st_dev`, `st_ino`)
//!    is recorded.
//! 3. It is renamed to its published name with `RENAME_NOREPLACE`, which
//!    never replaces a file and keeps the identity. A name that is taken is
//!    left alone ([`PublishError::Occupied`]).
//! 4. The caller makes a selection refer to it in the transaction that checks
//!    the selection's version, which marks the file `published`.
//!
//! A failure before 4 leaves the earlier selection and its image as they were.
//!
//! # Serving
//!
//! [`check`] opens the referenced path one component at a time without
//! following links, from the app data folder down, and hands out the bytes
//! only when the size, the SHA-256 and the format's first bytes are the ones
//! recorded. Whatever it finds is reported, never fixed up.
//!
//! A file whose hash matched is remembered by its [`Stamp`] (device, inode,
//! size, modification and change times in nanoseconds; [`Verified`]). While
//! the opened file has the same stamp, it is not read again to tell its state
//! (a status, a `304`) or hashed again to serve it: a write changes its change
//! time and a replacement its inode. Kernels before multigrain timestamps
//! (Linux 6.13) can give a write within the same clock tick (a few
//! milliseconds) after a check the same change time, so such a write can go
//! unnoticed until the next change.
//!
//! # Cleanup
//!
//! [`cleanup`] removes a `published` file only while it holds the database's
//! write lock (so no reference can appear meanwhile), when no selection of any
//! work refers to it by path or to the same file by identity (a hard link or
//! another spelling of the path), and when the file at the recorded place is
//! still the one the app made. When the identity of a referenced path cannot
//! be read, it removes nothing. [`recover`] finishes what an interrupted
//! publish left: the staged file and a published file no selection took, again
//! only by recorded identity.
//!
//! A recorded file is known by its inode alone, not by its device number: a
//! file system mounted again may give the same file another one (btrfs numbers
//! its devices at each mount; on the dev PC an unchanged file went from 47 to
//! 46 after a reboot), and the app's own files would then be taken for someone
//! else's and left on the disk for good. No birth time is recorded either, as
//! `trss_jobs::area::same_object` compares none: the image builds for musl,
//! where the standard library reads none. A different file that happens to get
//! the same inode number at the app's own name is then taken for the app's,
//! and it is still removed only when nothing refers to it. The identities of
//! the referenced paths are read under the same mount as the file, so there
//! the device number still tells files apart.

use std::{
    collections::{HashMap, HashSet},
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Component, Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex,
    },
    time::Duration,
};

use bytes::Bytes;
use rusqlite::TransactionBehavior;
use sha2::{Digest, Sha256};

use crate::artwork::image::sniff;
use crate::store::{
    artwork as store,
    artwork::{ArtworkError, ArtworkStore, FileRow, Format, ImageRef, Source},
};
use trss_anilist::MAX_IMAGE_BYTES;
use trss_core::{files::rename_noreplace, Millis};

/// The folder of the images, relative to the app data folder.
pub const ARTWORK_DIR: &str = "artwork";
/// Where new images are written before they are published.
pub const STAGING_DIR: &str = "artwork/.staging";
/// A publish recorded this long ago and not finished was interrupted; its
/// files are recovered. Far longer than one publish may take
/// ([`super::FETCH_TIMEOUT`] plus a decode).
pub const STALE_STAGING: Duration = Duration::from_secs(15 * 60);

/// The app data folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppData {
    root: PathBuf,
}

impl AppData {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        AppData { root: root.into() }
    }

    /// The folder of the database at `db_path`.
    pub fn for_database(db_path: &Path) -> Self {
        let parent = db_path.parent().filter(|p| !p.as_os_str().is_empty());
        AppData::new(parent.unwrap_or(Path::new(".")))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PublishError {
    #[error(transparent)]
    Store(#[from] ArtworkError),
    #[error("cannot write the image: {0}")]
    Io(#[from] io::Error),
    /// The published name is taken by a file the app did not just make.
    #[error("the image's place is taken")]
    Occupied,
}

/// SHA-256 as lowercase hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// Makes the folder `rel` under `root` if needed, and checks that it is a real
/// folder, not a link.
fn ensure_dir(root: &Path, rel: &str) -> io::Result<()> {
    let path = root.join(rel);
    match fs::create_dir(&path) {
        Ok(()) => {
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
        }
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e),
    }
    let meta = fs::symlink_metadata(&path)?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return Err(io::Error::other(format!("{rel} is not a plain folder")));
    }
    Ok(())
}

/// Writes `bytes` to a new file at `staging` and returns its identity.
fn write_staged(root: &Path, staging: &str, bytes: &[u8]) -> io::Result<(u64, u64)> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
        .open(root.join(staging))?;
    file.write_all(bytes)?;
    file.set_permissions(fs::Permissions::from_mode(0o644))?;
    file.sync_all()?;
    let meta = file.metadata()?;
    Ok((meta.dev(), meta.ino()))
}

/// Publishes `bytes` (already verified as `format`) under a new app-made name.
/// The file is recorded `staging` until a selection takes it.
pub async fn publish(
    app: &AppData,
    store: &ArtworkStore,
    bytes: Bytes,
    format: Format,
    now: Millis,
) -> Result<String, PublishError> {
    let id = store::new_id();
    let target = format!("{ARTWORK_DIR}/{id}.{}", format.extension());
    let staging = format!("{STAGING_DIR}/{id}.tmp");
    publish_at(app, store, bytes, &target, &staging, now).await?;
    Ok(target)
}

/// [`publish`] to given names (tests use it to take the name first).
pub(crate) async fn publish_at(
    app: &AppData,
    store: &ArtworkStore,
    bytes: Bytes,
    target: &str,
    staging: &str,
    now: Millis,
) -> Result<(), PublishError> {
    store.reserve_file(target, staging, now).await?;
    let root = app.root.clone();
    let staging_owned = staging.to_owned();
    let staged = tokio::task::spawn_blocking(move || -> io::Result<(u64, u64)> {
        ensure_dir(&root, ARTWORK_DIR)?;
        ensure_dir(&root, STAGING_DIR)?;
        write_staged(&root, &staging_owned, &bytes)
    })
    .await
    .map_err(|e| io::Error::other(e.to_string()))?;
    let (dev, ino) = match staged {
        Ok(identity) => identity,
        Err(e) => {
            // The file was not made (create_new failed) or is half-written;
            // either way it is the one this call made, or nothing.
            let _ = fs::remove_file(app.root.join(staging));
            store.forget_file(target).await?;
            return Err(e.into());
        }
    };
    store.file_identity(target, dev, ino).await?;

    let renamed = rename_noreplace(&app.root.join(staging), &app.root.join(target));
    if let Err(e) = renamed {
        // The staged file is this call's own (created exclusively just now).
        let _ = fs::remove_file(app.root.join(staging));
        store.forget_file(target).await?;
        return Err(if e.kind() == io::ErrorKind::AlreadyExists {
            PublishError::Occupied
        } else {
            e.into()
        });
    }
    // Make the rename durable before a selection refers to the name.
    if let Ok(dir) = File::open(app.root.join(ARTWORK_DIR)) {
        let _ = dir.sync_all();
    }
    Ok(())
}

/// Builds the reference to a just published image.
pub fn image_ref(origin: Source, relative_path: String, bytes: &[u8], format: Format) -> ImageRef {
    ImageRef {
        id: store::new_id(),
        origin,
        relative_path,
        byte_size: bytes.len() as u64,
        sha256: sha256_hex(bytes),
        format,
    }
}

/// Why a referenced image is not served (`docs/specs/settings.md`, 가져오기
/// 검증과 파일 가용성).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unavailable {
    /// No file at the referenced place.
    Missing,
    /// A file is there with other bytes (size, hash or format).
    Mismatch,
    /// The file could not be checked: a link, outside the app data folder, not
    /// a plain file, or unreadable.
    Unverified,
}

impl Unavailable {
    pub fn code(self) -> &'static str {
        match self {
            Unavailable::Missing => "missing",
            Unavailable::Mismatch => "mismatch",
            Unavailable::Unverified => "unverified",
        }
    }
}

/// The components of a reference's path, when it stays inside the app data
/// folder by its spelling: relative, and only plain names.
fn components(relative: &str) -> Option<Vec<&str>> {
    if relative.is_empty() || relative.contains('\0') {
        return None;
    }
    let mut parts = Vec::new();
    for component in Path::new(relative).components() {
        match component {
            Component::Normal(name) => parts.push(name.to_str()?),
            _ => return None,
        }
    }
    (!parts.is_empty()).then_some(parts)
}

/// Opens `relative` under `root` without following any link on the way.
fn open_inside(root: &Path, relative: &str) -> Result<File, Unavailable> {
    use rustix::fs::{openat, Mode, OFlags, CWD};

    let parts = components(relative).ok_or(Unavailable::Unverified)?;
    let classify = |e: rustix::io::Errno| {
        if e == rustix::io::Errno::NOENT {
            Unavailable::Missing
        } else {
            Unavailable::Unverified
        }
    };
    let mut dir = openat(
        CWD,
        root,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|_| Unavailable::Unverified)?;
    let (last, folders) = parts.split_last().expect("at least one component");
    for name in folders {
        dir = openat(
            &dir,
            *name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(classify)?;
    }
    let fd = openat(
        &dir,
        *last,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(classify)?;
    Ok(File::from(fd))
}

/// What tells a file's contents apart without reading them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    dev: u64,
    ino: u64,
    size: u64,
    mtime_ns: i128,
    ctime_ns: i128,
}

impl Stamp {
    fn of(meta: &fs::Metadata) -> Stamp {
        let ns = |s: i64, n: i64| s as i128 * 1_000_000_000 + n as i128;
        Stamp {
            dev: meta.dev(),
            ino: meta.ino(),
            size: meta.size(),
            mtime_ns: ns(meta.mtime(), meta.mtime_nsec()),
            ctime_ns: ns(meta.ctime(), meta.ctime_nsec()),
        }
    }
}

/// Files whose bytes matched their reference, by path, with the stamp they
/// had then. At most [`VERIFIED_ENTRIES`]; past that it starts over.
#[derive(Default)]
pub struct Verified {
    known: Mutex<HashMap<String, (Stamp, String, Format)>>,
    hashed: AtomicUsize,
}

/// How many verified files are remembered.
pub const VERIFIED_ENTRIES: usize = 4096;

impl Verified {
    fn holds(&self, image: &ImageRef, stamp: Stamp) -> bool {
        let known = self.known.lock().unwrap_or_else(|e| e.into_inner());
        known
            .get(&image.relative_path)
            .is_some_and(|(s, sha256, format)| {
                *s == stamp && *sha256 == image.sha256 && *format == image.format
            })
    }

    fn remember(&self, image: &ImageRef, stamp: Option<Stamp>) {
        let mut known = self.known.lock().unwrap_or_else(|e| e.into_inner());
        match stamp {
            Some(stamp) => {
                if known.len() >= VERIFIED_ENTRIES {
                    known.clear();
                }
                known.insert(
                    image.relative_path.clone(),
                    (stamp, image.sha256.clone(), image.format),
                );
            }
            None => {
                known.remove(&image.relative_path);
            }
        }
    }

    /// How many times a file was hashed.
    pub fn hashed(&self) -> usize {
        self.hashed.load(Ordering::Relaxed)
    }
}

/// Checks `image`'s file in the app data folder against the reference and,
/// with `read`, hands out its bytes (`None` without). A file verified before
/// with the same [`Stamp`] is neither read to tell its state nor hashed again.
/// Blocking.
pub fn check(
    app: &AppData,
    image: &ImageRef,
    verified: &Verified,
    read: bool,
) -> Result<Option<Vec<u8>>, Unavailable> {
    let mut file = open_inside(&app.root, &image.relative_path)?;
    let meta = file.metadata().map_err(|_| Unavailable::Unverified)?;
    if !meta.file_type().is_file() {
        return Err(Unavailable::Unverified);
    }
    if meta.len() != image.byte_size {
        return Err(Unavailable::Mismatch);
    }
    if meta.len() > MAX_IMAGE_BYTES as u64 {
        return Err(Unavailable::Unverified);
    }
    let before = Stamp::of(&meta);
    let known = verified.holds(image, before);
    if known && !read {
        return Ok(None);
    }
    let mut bytes = Vec::with_capacity(meta.len() as usize);
    (&mut file)
        .take(MAX_IMAGE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Unavailable::Unverified)?;
    // The bytes read are the stamped ones only when nothing changed meanwhile.
    let after = file.metadata().ok().map(|m| Stamp::of(&m));
    let unchanged = (after == Some(before)).then_some(before);
    if bytes.len() as u64 != image.byte_size || sniff(&bytes) != Some(image.format) {
        verified.remember(image, None);
        return Err(Unavailable::Mismatch);
    }
    if !(known && unchanged.is_some()) {
        verified.hashed.fetch_add(1, Ordering::Relaxed);
        if sha256_hex(&bytes) != image.sha256 {
            verified.remember(image, None);
            return Err(Unavailable::Mismatch);
        }
        verified.remember(image, unchanged);
    }
    Ok(read.then_some(bytes))
}

/// The path a reference names, spelled out lexically (`.` and `..` resolved
/// without the file system), for comparing references by path.
fn lexical(root: &Path, relative: &str) -> PathBuf {
    let mut out = PathBuf::new();
    for component in root.join(relative).components() {
        match component {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

/// The identities (device, inode) of what the referenced paths lead to now,
/// both the entry itself and what a link there points to. `None` when one
/// cannot be read for a reason other than its absence: then nothing can be
/// known to be unreferenced.
fn referenced_identities(root: &Path, paths: &[String]) -> Option<HashSet<(u64, u64)>> {
    let mut identities = HashSet::new();
    for relative in paths {
        let path = root.join(relative);
        for meta in [fs::symlink_metadata(&path), fs::metadata(&path)] {
            match meta {
                Ok(meta) => {
                    identities.insert((meta.dev(), meta.ino()));
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(_) => return None,
            }
        }
    }
    Some(identities)
}

/// What a cleanup did, for logs and tests.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Cleaned {
    pub removed: usize,
    /// Files kept because something refers to them.
    pub kept: usize,
    /// Records dropped whose file was gone or is not the app's any more (the
    /// file, if any, left alone).
    pub forgotten: usize,
    /// Whether the cleanup stopped because it could not be sure.
    pub unsure: bool,
}

/// What is at a recorded place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Found {
    Nothing,
    /// Something other than the file the app recorded.
    Other,
    /// The recorded file, with its identity (device, inode) as mounted now.
    Ours((u64, u64)),
}

/// What the entry at `path` is to `file`'s record, which knows it by its
/// inode alone (see the module docs).
fn found_at(path: &Path, file: &FileRow) -> io::Result<Found> {
    match fs::symlink_metadata(path) {
        Ok(meta) => {
            let ours = meta.file_type().is_file() && Some(meta.ino()) == file.ino;
            Ok(if ours {
                Found::Ours((meta.dev(), meta.ino()))
            } else {
                Found::Other
            })
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Found::Nothing),
        Err(e) => Err(e),
    }
}

/// Removes the published files no selection refers to (see the module docs).
pub async fn cleanup(app: &AppData, store: &ArtworkStore) -> Result<Cleaned, ArtworkError> {
    let root = app.root.clone();
    store
        .run(move |conn| {
            // The write lock keeps every reference where it is until the end.
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let mut cleaned = Cleaned::default();
            let files = store::files_of_state(&tx, "published")?;
            if files.is_empty() {
                return Ok(cleaned);
            }
            let referenced = store::referenced_paths(&tx)?;
            let by_path: HashSet<PathBuf> = referenced.iter().map(|p| lexical(&root, p)).collect();
            let Some(identities) = referenced_identities(&root, &referenced) else {
                cleaned.unsure = true;
                return Ok(cleaned);
            };
            for file in files {
                let path = root.join(&file.relative_path);
                if by_path.contains(&lexical(&root, &file.relative_path)) {
                    cleaned.kept += 1;
                    continue;
                }
                match found_at(&path, &file) {
                    Err(_) => {
                        cleaned.unsure = true;
                        continue;
                    }
                    Ok(Found::Nothing) => {
                        store::delete_file_row(&tx, &file.relative_path)?;
                        cleaned.forgotten += 1;
                    }
                    Ok(Found::Other) => {
                        // Not the file the app made: left alone, and no longer
                        // the app's to remove.
                        store::delete_file_row(&tx, &file.relative_path)?;
                        cleaned.forgotten += 1;
                    }
                    Ok(Found::Ours(identity)) => {
                        // Read under the same mount as the referenced paths'.
                        if identities.contains(&identity) {
                            cleaned.kept += 1;
                            continue;
                        }
                        match fs::remove_file(&path) {
                            Ok(()) => {}
                            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                            Err(_) => {
                                cleaned.unsure = true;
                                continue;
                            }
                        }
                        store::delete_file_row(&tx, &file.relative_path)?;
                        cleaned.removed += 1;
                    }
                }
            }
            tx.commit()?;
            Ok(cleaned)
        })
        .await
}

/// Finishes the publishes recorded before `now - STALE_STAGING` that never
/// reached a selection: removes their staged file, and hands a published one
/// to [`cleanup`], each only when its identity is the recorded one.
pub async fn recover(
    app: &AppData,
    store: &ArtworkStore,
    now: Millis,
) -> Result<usize, ArtworkError> {
    let root = app.root.clone();
    let before = now - STALE_STAGING.as_millis() as i64;
    store
        .run(move |conn| {
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let mut recovered = 0;
            for file in store::files_of_state(&tx, "staging")? {
                if file.created_at > before {
                    continue;
                }
                let staged = root.join(&file.staging_path);
                if let Ok(Found::Ours(_)) = found_at(&staged, &file) {
                    let _ = fs::remove_file(&staged);
                }
                match found_at(&root.join(&file.relative_path), &file) {
                    Ok(Found::Ours(_)) => {
                        tx.prepare_cached(
                            "UPDATE artwork_files SET state = 'published' WHERE relative_path = ?1",
                        )?
                        .execute([&file.relative_path])?;
                    }
                    Err(_) => continue,
                    Ok(_) => store::delete_file_row(&tx, &file.relative_path)?,
                }
                recovered += 1;
            }
            tx.commit()?;
            Ok(recovered)
        })
        .await
}
