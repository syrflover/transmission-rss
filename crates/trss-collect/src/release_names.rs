//! The release names of `tests/fixtures/release-names.tsv`, read the ways
//! trss reads them (`docs/adr/0015-test-a-rule-once-in-its-crate.md`): the
//! work, episode and revision this crate reads from a title, and the episode
//! in the name the worker gives a video file when it renames it: the name
//! this crate has `trname` read ([`crate::release_name::name_for_trname`], none for an unnumbered
//! video or a batch, which keeps its name) and what `trname` makes of it.
//!
//! The names come from the collection history of the server's database (names
//! only) and from public feeds. Each line holds what the name means; a reading
//! that differs from it today is listed in the line's known failures, and the
//! test fails both for a new difference and for a known one that is gone.

use std::{collections::BTreeSet, path::PathBuf};

use crate::{
    commands::receive_once::derived_name,
    release_name::{Kind, ReleaseName},
};

const NAMES: &str = include_str!("../tests/fixtures/release-names.tsv");

/// What a name is read as. `episode` is `12`, `12.5`, `batch`, `batch 1-12`
/// or `-` (no episode); `trname` is the episode in the name the worker gives a
/// video file of that name, `-` when the file keeps its name, and is not read
/// for other names.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Reading {
    work: String,
    episode: String,
    version: u32,
    trname: Option<String>,
}

/// A video file's name, which the worker gives `trname`; other names are
/// feed titles that are never file names.
fn is_video_file(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    [".mkv", ".mp4"].iter().any(|ext| lower.ends_with(ext))
}

/// The episode the worker should give a video file whose name means `episode`:
/// none for a batch or a name without an episode, which keeps its name.
fn trname_episode(episode: &str) -> &str {
    if episode.starts_with("batch") {
        "-"
    } else {
        episode
    }
}

/// `05.5` or `1000` as written in `S01E…`, as [`episode_text`] writes it.
fn written_episode(text: &str) -> String {
    match text.split_once('.') {
        Some((whole, half)) => format!("{}.{half}", whole.trim_start_matches('0')),
        None => text.trim_start_matches('0').to_owned(),
    }
}

fn reading(name: &str) -> Reading {
    let read = ReleaseName::read(name);
    let episode = match read.kind {
        Kind::Episode(episode) => episode.text(),
        Kind::Batch {
            range: Some((from, to)),
        } => format!("batch {from}-{to}"),
        Kind::Batch { range: None } => "batch".to_owned(),
        Kind::Unnumbered => "-".to_owned(),
    };
    let work = read.work.clone().unwrap_or_else(|| "-".to_owned());
    let trname = is_video_file(name).then(|| {
        // The rule's folder for the work, its first season, no conversion.
        let title = if work == "-" { "Work" } else { &work };
        let folder = PathBuf::from("/media")
            .join(title.replace('/', " "))
            .join("Season 01");
        derived_name(&folder, name, 0)
            .and_then(|named| {
                let rest = named.rsplit_once(" S01E")?.1;
                Some(written_episode(rest.rsplit_once('.')?.0))
            })
            .unwrap_or_else(|| "-".to_owned())
    });
    Reading {
        work,
        episode,
        version: read.version,
        trname,
    }
}

/// One line of the file: the name and what it means.
struct Line<'a> {
    name: &'a str,
    expected: Reading,
    known: BTreeSet<&'a str>,
}

fn lines() -> Vec<Line<'static>> {
    NAMES
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| {
            let fields: Vec<&str> = line.split('\t').collect();
            let [name, _source, work, episode, version, known] = fields[..] else {
                panic!("not six fields: {line}");
            };
            Line {
                name,
                expected: Reading {
                    work: work.to_owned(),
                    episode: episode.to_owned(),
                    version: version.parse().expect("a revision number"),
                    trname: is_video_file(name).then(|| trname_episode(episode).to_owned()),
                },
                known: known.split(',').filter(|k| !k.is_empty()).collect(),
            }
        })
        .collect()
}

/// The parts of `got` that differ from `expected`.
fn differences(expected: &Reading, got: &Reading) -> BTreeSet<&'static str> {
    let mut out = BTreeSet::new();
    if got.work != expected.work {
        out.insert("work");
    }
    if got.episode != expected.episode {
        out.insert("episode");
    }
    if got.version != expected.version {
        out.insert("version");
    }
    if got.trname != expected.trname {
        out.insert("trname");
    }
    out
}

#[test]
fn every_release_name_reads_as_it_means_but_the_known_failures() {
    let lines = lines();
    let mut wrong = Vec::new();
    for line in &lines {
        let got = reading(line.name);
        let differs = differences(&line.expected, &got);
        let known: BTreeSet<&str> = line.known.iter().copied().collect();
        if differs != known {
            wrong.push(format!(
                "{}\n  expected {:?}, known failures {known:?}\n  read     {got:?}",
                line.name, line.expected
            ));
        }
    }
    assert!(lines.len() >= 700, "{} names", lines.len());
    assert!(
        wrong.is_empty(),
        "{} of {} names read otherwise than the file says:\n{}",
        wrong.len(),
        lines.len(),
        wrong.join("\n")
    );
}

/// Prints the file's lines for the names of `TRSS_RELEASE_NAMES` (a name and
/// its source per line, tab-separated), to extend the file: this crate's
/// reading as what the name means, and `trname` as a known failure where it
/// reads another episode. Each line still has to be checked against the name.
#[test]
#[ignore = "writes the file's lines for new names"]
fn print_lines() {
    let path = std::env::var("TRSS_RELEASE_NAMES").expect("TRSS_RELEASE_NAMES");
    for line in std::fs::read_to_string(path).unwrap().lines() {
        let (name, source) = line.split_once('\t').unwrap_or((line, "feed"));
        let got = reading(name);
        let known = match &got.trname {
            Some(trname) if trname != trname_episode(&got.episode) => "trname",
            _ => "",
        };
        println!(
            "{name}\t{source}\t{}\t{}\t{}\t{known}",
            got.work, got.episode, got.version
        );
    }
}
