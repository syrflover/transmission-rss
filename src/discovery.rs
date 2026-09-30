//! Discovering works, seasons and episodes in a watch folder
//! (`docs/specs/library.md`, 작품 발견과 감시 폴더).
//!
//! The web (when a watch folder is added) and the worker (every cycle, and on
//! `다시 확인`) read a folder with the same [`scan`], which only looks: it never
//! creates, moves, renames or deletes anything, and it does not touch the
//! database. What a scan found is handed to [`crate::store::library`], which
//! decides what is new and what has gone.
//!
//! # The layout
//!
//! trname writes `<watch folder>/<work>/Season NN/<work> SxxEyy.ext`:
//!
//! - a **work** is a folder directly under the watch folder;
//! - a **season** is a `Season NN` folder in a work (`NN` is an integer, 0 or
//!   more);
//! - an **episode** is an `SxxEyy` file name in a season folder. `yy` is kept as
//!   written (`01`, `17.5`), so two spellings are two episodes;
//! - a **subtitle** is a file of the same episode with a subtitle extension; a
//!   language fragment before the extension (`… S01E01.ko.smi`) is allowed.
//!
//! What does not fit is counted on the work as a file whose episode could not
//! be told ([`Unrecognized`]): a name whose `SxxEyy` is not its season folder's
//! season, a video or subtitle outside a season folder, anything in a folder
//! inside a season folder (a multi-file torrent's folder), a name with no
//! `SxxEyy`, and `.part` files (still downloading), wherever they are. Files of
//! other kinds (images, `.nfo`, text) are not media and are left out of
//! everything.
//!
//! # What is skipped
//!
//! Hidden entries (a leading `.`, which includes a work's `.trss/`) and the
//! folders NAS appliances and file systems add (`@eaDir`, `#recycle`,
//! `$RECYCLE.BIN`, `lost+found`, `System Volume Information`). A link is followed
//! only when it resolves to a place inside the watch folder, so a link can not
//! lead the scan out of it; a link that leaves is skipped. Names that are not
//! valid UTF-8 cannot be stored and are skipped too.
//!
//! # Failures
//!
//! A watch folder that cannot be read is a [`ScanError`] and says nothing about
//! what is in it. A work folder that cannot be read is [`WorkRead::Unreadable`]
//! and the other works are still read, so one bad folder does not hide the rest.
//! Neither may make a caller forget what it knew.

use std::{collections::BTreeSet, fs, io, path::Path, sync::LazyLock};

use regex::Regex;

/// Video extensions, lower case.
pub const VIDEO_EXTENSIONS: &[&str] = &[
    "mkv", "mp4", "avi", "m4v", "webm", "ts", "mov", "wmv", "flv", "m2ts",
];
/// Subtitle extensions, lower case.
pub const SUBTITLE_EXTENSIONS: &[&str] = &["ass", "ssa", "smi", "srt", "vtt", "sup"];

/// Folders that are never works or seasons, whatever is in them.
const IGNORED_NAMES: &[&str] = &[
    "@eaDir",
    "#recycle",
    "$RECYCLE.BIN",
    "lost+found",
    "System Volume Information",
];

/// How far below a season folder (or another folder of a work) files are
/// looked for. Deep enough for any torrent folder, and it ends a link loop.
const MAX_DEPTH: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Video,
    Subtitle,
}

impl FileKind {
    pub fn code(self) -> &'static str {
        match self {
            FileKind::Video => "video",
            FileKind::Subtitle => "subtitle",
        }
    }

    pub fn from_code(code: &str) -> Option<FileKind> {
        match code {
            "video" => Some(FileKind::Video),
            "subtitle" => Some(FileKind::Subtitle),
            _ => None,
        }
    }
}

/// Why a file is not attached to an episode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Reason {
    /// The `SxxEyy` in the name is not the season of the folder it is in.
    SeasonMismatch,
    /// A video or subtitle in the work folder, or in a folder that is not a
    /// `Season NN` folder.
    OutsideSeason,
    /// In a folder inside a season folder.
    InSubfolder,
    /// A `.part` file: still downloading.
    Partial,
    /// In a season folder, but the name has no `SxxEyy`.
    NoEpisode,
}

impl Reason {
    pub fn code(self) -> &'static str {
        match self {
            Reason::SeasonMismatch => "season_mismatch",
            Reason::OutsideSeason => "outside_season",
            Reason::InSubfolder => "in_subfolder",
            Reason::Partial => "partial",
            Reason::NoEpisode => "no_episode",
        }
    }

    pub fn from_code(code: &str) -> Option<Reason> {
        [
            Reason::SeasonMismatch,
            Reason::OutsideSeason,
            Reason::InSubfolder,
            Reason::Partial,
            Reason::NoEpisode,
        ]
        .into_iter()
        .find(|reason| reason.code() == code)
    }

    /// The reason as a sentence fragment for a screen.
    pub fn message(self) -> &'static str {
        match self {
            Reason::SeasonMismatch => "파일 이름의 시즌이 들어 있는 시즌 폴더와 달라요",
            Reason::OutsideSeason => "시즌 폴더 밖에 있어요",
            Reason::InSubfolder => "시즌 폴더 안의 하위 폴더에 있어요",
            Reason::Partial => "아직 받는 중인 파일이에요",
            Reason::NoEpisode => "이름에서 회차를 읽지 못했어요",
        }
    }
}

/// A video or subtitle attached to an episode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EpisodeFile {
    /// Relative to the work folder, `/`-separated.
    pub path: String,
    pub kind: FileKind,
    pub season: u32,
    /// The episode as written in the name: `01`, `17.5`.
    pub episode: String,
}

/// A file that could not be attached to an episode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unrecognized {
    /// Relative to the work folder, `/`-separated.
    pub path: String,
    pub reason: Reason,
}

/// One work folder as read.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ScannedWork {
    /// The folder's name under the watch folder.
    pub dir_name: String,
    /// The seasons that have a folder, even an empty one.
    pub seasons: BTreeSet<u32>,
    pub files: Vec<EpisodeFile>,
    pub unrecognized: Vec<Unrecognized>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkRead {
    Read(ScannedWork),
    /// The folder exists but could not be read; `reason` is a sentence.
    Unreadable {
        dir_name: String,
        reason: String,
    },
}

impl WorkRead {
    pub fn dir_name(&self) -> &str {
        match self {
            WorkRead::Read(work) => &work.dir_name,
            WorkRead::Unreadable { dir_name, .. } => dir_name,
        }
    }
}

/// Everything one scan of a watch folder found.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Scan {
    /// In folder-name order.
    pub works: Vec<WorkRead>,
}

/// A watch folder that could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanError {
    /// A sentence for the folder's row on the screen.
    pub message: String,
    /// What the system said, for logs only.
    pub detail: String,
}

impl std::fmt::Display for ScanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.message, self.detail)
    }
}

impl std::error::Error for ScanError {}

fn scan_error(error: &io::Error) -> ScanError {
    let message = match error.kind() {
        io::ErrorKind::NotFound => {
            "폴더를 찾지 못했어요. 마운트가 풀렸거나 이름이 바뀌었는지 확인해 주세요."
        }
        io::ErrorKind::PermissionDenied => "폴더를 읽을 권한이 없어요.",
        io::ErrorKind::NotADirectory => "폴더가 아니에요.",
        _ => "폴더를 읽지 못했어요.",
    };
    ScanError {
        message: message.to_owned(),
        detail: error.to_string(),
    }
}

fn unreadable_reason(error: &io::Error) -> String {
    match error.kind() {
        io::ErrorKind::PermissionDenied => "읽을 권한이 없어요.".to_owned(),
        io::ErrorKind::NotFound => "읽는 도중 없어졌어요.".to_owned(),
        _ => "읽지 못했어요.".to_owned(),
    }
}

/// Reads the watch folder at `root`. See the module docs.
pub fn scan(root: &Path) -> Result<Scan, ScanError> {
    let real_root = fs::canonicalize(root).map_err(|e| scan_error(&e))?;
    let entries = sorted_entries(&real_root).map_err(|e| scan_error(&e))?;

    let mut works = Vec::new();
    for entry in entries {
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if skipped(&name) {
            continue;
        }
        if classify(&entry, &real_root) != Node::Dir {
            continue;
        }
        let read = match read_work(&entry.path(), &name, &real_root) {
            Ok(work) => WorkRead::Read(work),
            Err(error) => WorkRead::Unreadable {
                dir_name: name,
                reason: unreadable_reason(&error),
            },
        };
        works.push(read);
    }
    Ok(Scan { works })
}

fn skipped(name: &str) -> bool {
    name.starts_with('.') || IGNORED_NAMES.contains(&name)
}

/// The entries of `dir`, by name.
fn sorted_entries(dir: &Path) -> io::Result<Vec<fs::DirEntry>> {
    let mut entries = fs::read_dir(dir)?.collect::<io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    Ok(entries)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Node {
    File,
    Dir,
    /// Not followed or not a file: a link out of the watch folder, a broken
    /// link, a socket.
    Skip,
}

/// What an entry is, following a link only when it stays inside `real_root`.
fn classify(entry: &fs::DirEntry, real_root: &Path) -> Node {
    let Ok(file_type) = entry.file_type() else {
        return Node::Skip;
    };
    if file_type.is_dir() {
        return Node::Dir;
    }
    if file_type.is_file() {
        return Node::File;
    }
    if !file_type.is_symlink() {
        return Node::Skip;
    }
    let Ok(real) = fs::canonicalize(entry.path()) else {
        return Node::Skip;
    };
    if !real.starts_with(real_root) {
        return Node::Skip;
    }
    match fs::metadata(&real) {
        Ok(metadata) if metadata.is_dir() => {
            // A link to the folder it is in, or to one above it, only loops.
            let loops = entry
                .path()
                .parent()
                .and_then(|parent| fs::canonicalize(parent).ok())
                .is_none_or(|parent| parent.starts_with(&real));
            if loops {
                Node::Skip
            } else {
                Node::Dir
            }
        }
        Ok(metadata) if metadata.is_file() => Node::File,
        _ => Node::Skip,
    }
}

/// The season of a `Season NN` folder name.
pub fn season_of_folder(name: &str) -> Option<u32> {
    static SEASON: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)^Season\s+(\d{1,4})$").unwrap());
    SEASON.captures(name)?[1].parse().ok()
}

fn extension(name: &str) -> Option<String> {
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
}

fn kind_of(name: &str) -> Option<FileKind> {
    let ext = extension(name)?;
    if VIDEO_EXTENSIONS.contains(&ext.as_str()) {
        Some(FileKind::Video)
    } else if SUBTITLE_EXTENSIONS.contains(&ext.as_str()) {
        Some(FileKind::Subtitle)
    } else {
        None
    }
}

fn is_partial(name: &str) -> bool {
    name.len() > ".part".len() && name.to_ascii_lowercase().ends_with(".part")
}

/// Whether a file is something discovery counts: media, or a download in progress.
fn is_media_or_partial(name: &str) -> bool {
    is_partial(name) || kind_of(name).is_some()
}

/// The `SxxEyy` a file name ends with (before its extension and, for a
/// subtitle, before language fragments): the season and the episode as written.
pub fn episode_of(name: &str, kind: FileKind) -> Option<(u32, String)> {
    static EPISODE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)(?:^|[^A-Za-z0-9])S(\d{1,4})E(\d{1,4}(?:\.\d{1,2})?)$").unwrap()
    });
    static LANGUAGE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^[A-Za-z][A-Za-z0-9_-]{0,15}$").unwrap());

    let stem = &name[..name.rfind('.')?];
    let mut stem = stem;
    let mut fragments = 0;
    loop {
        if let Some(caught) = EPISODE.captures(stem) {
            return Some((caught[1].parse().ok()?, caught[2].to_owned()));
        }
        // Only a subtitle carries a language (and flags such as `forced`).
        if kind != FileKind::Subtitle || fragments == 2 {
            return None;
        }
        let dot = stem.rfind('.')?;
        if !LANGUAGE.is_match(&stem[dot + 1..]) {
            return None;
        }
        stem = &stem[..dot];
        fragments += 1;
    }
}

/// Reads one work folder.
fn read_work(dir: &Path, name: &str, real_root: &Path) -> io::Result<ScannedWork> {
    let mut work = ScannedWork {
        dir_name: name.to_owned(),
        ..ScannedWork::default()
    };
    for entry in sorted_entries(dir)? {
        let Some(entry_name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if skipped(&entry_name) {
            continue;
        }
        match classify(&entry, real_root) {
            Node::Skip => {}
            Node::File => {
                if is_media_or_partial(&entry_name) {
                    let reason = if is_partial(&entry_name) {
                        Reason::Partial
                    } else {
                        Reason::OutsideSeason
                    };
                    work.unrecognized.push(Unrecognized {
                        path: entry_name,
                        reason,
                    });
                }
            }
            Node::Dir => match season_of_folder(&entry_name) {
                Some(season) => {
                    work.seasons.insert(season);
                    read_season(&entry.path(), &entry_name, season, real_root, &mut work)?;
                }
                None => collect_all(
                    &entry.path(),
                    &entry_name,
                    Reason::OutsideSeason,
                    real_root,
                    1,
                    &mut work,
                )?,
            },
        }
    }
    Ok(work)
}

/// Reads the files directly in a season folder; what is in folders below it
/// is all unrecognized.
fn read_season(
    dir: &Path,
    folder: &str,
    season: u32,
    real_root: &Path,
    work: &mut ScannedWork,
) -> io::Result<()> {
    for entry in sorted_entries(dir)? {
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if skipped(&name) {
            continue;
        }
        let path = format!("{folder}/{name}");
        match classify(&entry, real_root) {
            Node::Skip => {}
            Node::Dir => collect_all(
                &entry.path(),
                &path,
                Reason::InSubfolder,
                real_root,
                1,
                work,
            )?,
            Node::File => {
                if is_partial(&name) {
                    work.unrecognized.push(Unrecognized {
                        path,
                        reason: Reason::Partial,
                    });
                    continue;
                }
                let Some(kind) = kind_of(&name) else {
                    continue;
                };
                match episode_of(&name, kind) {
                    None => work.unrecognized.push(Unrecognized {
                        path,
                        reason: Reason::NoEpisode,
                    }),
                    Some((found, _)) if found != season => work.unrecognized.push(Unrecognized {
                        path,
                        reason: Reason::SeasonMismatch,
                    }),
                    Some((_, episode)) => work.files.push(EpisodeFile {
                        path,
                        kind,
                        season,
                        episode,
                    }),
                }
            }
        }
    }
    Ok(())
}

/// Every video, subtitle and `.part` file below `dir`, as unrecognized for
/// `reason` (a `.part` file is always [`Reason::Partial`]).
fn collect_all(
    dir: &Path,
    shown: &str,
    reason: Reason,
    real_root: &Path,
    depth: usize,
    work: &mut ScannedWork,
) -> io::Result<()> {
    if depth > MAX_DEPTH {
        return Ok(());
    }
    for entry in sorted_entries(dir)? {
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if skipped(&name) {
            continue;
        }
        let path = format!("{shown}/{name}");
        match classify(&entry, real_root) {
            Node::Skip => {}
            Node::Dir => collect_all(&entry.path(), &path, reason, real_root, depth + 1, work)?,
            Node::File => {
                if is_partial(&name) {
                    work.unrecognized.push(Unrecognized {
                        path,
                        reason: Reason::Partial,
                    });
                } else if kind_of(&name).is_some() {
                    work.unrecognized.push(Unrecognized { path, reason });
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(root: &Path, relative: &str) {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "x").unwrap();
    }

    fn work<'a>(scan: &'a Scan, name: &str) -> &'a ScannedWork {
        scan.works
            .iter()
            .find_map(|w| match w {
                WorkRead::Read(work) if work.dir_name == name => Some(work),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no work {name}"))
    }

    #[test]
    fn episodes_are_read_as_written() {
        let video = |n: &str| episode_of(n, FileKind::Video);
        assert_eq!(video("Show S01E01.mkv"), Some((1, "01".into())));
        assert_eq!(video("Show S02E17.5.mkv"), Some((2, "17.5".into())));
        assert_eq!(video("Show S01E105.mkv"), Some((1, "105".into())));
        assert_eq!(video("show s01e03.MP4"), Some((1, "03".into())));
        assert_eq!(video("Show S01E01 1080p.mkv"), None);
        assert_eq!(video("ShowS01E01.mkv"), None);
        assert_eq!(video("Show.mkv"), None);
        // Only a subtitle has a language fragment.
        assert_eq!(video("Show S01E01.ko.mkv"), None);
        let sub = |n: &str| episode_of(n, FileKind::Subtitle);
        assert_eq!(sub("Show S01E01.ass"), Some((1, "01".into())));
        assert_eq!(sub("Show S01E01.ko.smi"), Some((1, "01".into())));
        assert_eq!(sub("Show S01E02.5.ko.forced.ass"), Some((1, "02.5".into())));
        assert_eq!(sub("Show S01E02.a.b.c.ass"), None);
        assert_eq!(sub("Show S01E02.한국어.ass"), None);
    }

    #[test]
    fn season_folders_are_season_and_a_number() {
        assert_eq!(season_of_folder("Season 01"), Some(1));
        assert_eq!(season_of_folder("Season 0"), Some(0));
        assert_eq!(season_of_folder("season 12"), Some(12));
        assert_eq!(season_of_folder("Season"), None);
        assert_eq!(season_of_folder("Season 1 extras"), None);
        assert_eq!(season_of_folder("Specials"), None);
        assert_eq!(season_of_folder("Season -1"), None);
    }

    #[test]
    fn a_trname_folder_is_read_into_seasons_episodes_and_files() {
        let dir = tempfile::tempdir().unwrap();
        for file in [
            "Lycoris Recoil/Season 01/Lycoris Recoil S01E01.mkv",
            "Lycoris Recoil/Season 01/Lycoris Recoil S01E01.smi",
            "Lycoris Recoil/Season 01/Lycoris Recoil S01E02.mkv",
            "Lycoris Recoil/Season 02/Lycoris Recoil S02E01.mkv",
        ] {
            touch(dir.path(), file);
        }
        let scan = scan(dir.path()).unwrap();
        assert_eq!(scan.works.len(), 1);
        let work = work(&scan, "Lycoris Recoil");
        assert_eq!(work.seasons, BTreeSet::from([1, 2]));
        assert!(work.unrecognized.is_empty());
        let files: Vec<_> = work
            .files
            .iter()
            .map(|f| (f.season, f.episode.as_str(), f.kind, f.path.as_str()))
            .collect();
        assert_eq!(
            files,
            [
                (
                    1,
                    "01",
                    FileKind::Video,
                    "Season 01/Lycoris Recoil S01E01.mkv"
                ),
                (
                    1,
                    "01",
                    FileKind::Subtitle,
                    "Season 01/Lycoris Recoil S01E01.smi"
                ),
                (
                    1,
                    "02",
                    FileKind::Video,
                    "Season 01/Lycoris Recoil S01E02.mkv"
                ),
                (
                    2,
                    "01",
                    FileKind::Video,
                    "Season 02/Lycoris Recoil S02E01.mkv"
                ),
            ]
        );
    }

    #[test]
    fn what_does_not_fit_is_counted_with_a_reason_and_hidden_things_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        for file in [
            "W/Season 01/W S01E01.mkv",
            // The season in the name is not the folder's.
            "W/Season 01/W S03E01.mkv",
            // No episode in the name.
            "W/Season 01/extra.mkv",
            // Outside a season folder.
            "W/W S01E02.mkv",
            "W/Extras/Making of.mp4",
            // In a folder inside a season folder.
            "W/Season 01/[Group] Batch/ep 02.mkv",
            "W/Season 01/[Group] Batch/deeper/ep 03.mkv",
            // Still downloading.
            "W/Season 01/W S01E03.mkv.part",
            "W/loose.part",
            // Not media, or not discovered.
            "W/Season 01/cover.jpg",
            "W/Season 01/W S01E01.nfo",
            "W/notes.txt",
            "W/.trss/subs/W S01E01.ass",
            "W/Season 01/.hidden S01E09.mkv",
            ".Hidden/Season 01/H S01E01.mkv",
            "@eaDir/Season 01/E S01E01.mkv",
        ] {
            touch(dir.path(), file);
        }
        let scan = scan(dir.path()).unwrap();
        assert_eq!(
            scan.works.len(),
            1,
            "hidden and appliance folders are not works"
        );
        let work = work(&scan, "W");
        assert_eq!(work.files.len(), 1);
        assert_eq!(work.files[0].path, "Season 01/W S01E01.mkv");
        let mut unrecognized: Vec<_> = work
            .unrecognized
            .iter()
            .map(|u| (u.path.as_str(), u.reason))
            .collect();
        unrecognized.sort();
        assert_eq!(
            unrecognized,
            [
                ("Extras/Making of.mp4", Reason::OutsideSeason),
                ("Season 01/W S01E03.mkv.part", Reason::Partial),
                ("Season 01/W S03E01.mkv", Reason::SeasonMismatch),
                (
                    "Season 01/[Group] Batch/deeper/ep 03.mkv",
                    Reason::InSubfolder
                ),
                ("Season 01/[Group] Batch/ep 02.mkv", Reason::InSubfolder),
                ("Season 01/extra.mkv", Reason::NoEpisode),
                ("W S01E02.mkv", Reason::OutsideSeason),
                ("loose.part", Reason::Partial),
            ]
        );
    }

    #[test]
    fn links_are_followed_only_inside_the_watch_folder() {
        let dir = tempfile::tempdir().unwrap();
        let watch = dir.path().join("watch");
        let outside = dir.path().join("outside");
        touch(&watch, "Real/Season 01/Real S01E01.mkv");
        touch(&outside, "Out/Season 01/Out S01E01.mkv");
        touch(&outside, "loose/Linked S01E01.mkv");
        std::os::unix::fs::symlink(outside.join("Out"), watch.join("Linked")).unwrap();
        std::os::unix::fs::symlink(
            outside.join("loose/Linked S01E01.mkv"),
            watch.join("Real/Season 01/Real S01E02.mkv"),
        )
        .unwrap();
        // Inside links work, and a link to an ancestor does not loop forever.
        std::os::unix::fs::symlink(watch.join("Real"), watch.join("Alias")).unwrap();
        std::os::unix::fs::symlink(&watch, watch.join("Real/Season 01/loop")).unwrap();
        std::os::unix::fs::symlink(dir.path().join("missing"), watch.join("Broken")).unwrap();

        let scan = scan(&watch).unwrap();
        let names: Vec<_> = scan.works.iter().map(WorkRead::dir_name).collect();
        assert_eq!(names, ["Alias", "Real"]);
        let real = work(&scan, "Real");
        assert_eq!(real.files.len(), 1, "{:?}", real.files);
        assert_eq!(real.files[0].path, "Season 01/Real S01E01.mkv");
        // The link to an ancestor is not walked.
        assert!(real.unrecognized.is_empty(), "{:?}", real.unrecognized);
    }

    #[test]
    fn an_unreadable_work_folder_does_not_hide_the_other_works() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "Good/Season 01/Good S01E01.mkv");
        touch(dir.path(), "Locked/Season 01/Locked S01E01.mkv");
        let locked = dir.path().join("Locked");
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
        let scan = scan(dir.path());
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
        let scan = scan.unwrap();
        assert_eq!(scan.works.len(), 2);
        assert_eq!(work(&scan, "Good").files.len(), 1);
        match &scan.works[1] {
            WorkRead::Unreadable { dir_name, reason } => {
                assert_eq!(dir_name, "Locked");
                assert!(reason.contains("권한"), "{reason}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_missing_or_file_watch_folder_is_an_error_with_a_sentence() {
        let dir = tempfile::tempdir().unwrap();
        let error = scan(&dir.path().join("nope")).unwrap_err();
        assert!(error.message.contains("찾지 못했어요"), "{error}");
        let file = dir.path().join("file");
        fs::write(&file, "x").unwrap();
        assert!(scan(&file).is_err());
    }
}
