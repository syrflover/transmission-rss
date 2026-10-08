//! The episode file names and season folders `trname` writes
//! (`<title>/Season 02/<title> S02E05.mkv`, `S02E05.5`, `S02E105`), and the
//! ways trss reads them back, each named for what it reads them for.
//!
//! For every name `trname` writes (a season of 1 to 99, an episode of two to
//! four digits or a `.5` half) the readers agree. They differ only on names a
//! person gave by hand or another program wrote, and each keeps the reading
//! its caller was built on:
//!
//! - [`season_episode`] reads the season and episode out of a name trss derived
//!   or found on disk, whatever comes before them;
//! - [`is_trname_name`] tells whether a name is the one `trname` gives under a
//!   title, which a rename must not convert a second time;
//! - [`listed_episode`] and [`season_folder`] read a library folder the way a
//!   person may have kept it (`S1E5`, `.25`, a subtitle's language fragments,
//!   `season 2`).
//!
//! The reading of a *release* name (`[Group] Show - 05 (1080p).mkv`) is not
//! here; it belongs to `trss-collect`. `trname` reads the season folder of a
//! rule's save path itself, and the callers that predict its name call it.

use std::sync::LazyLock;

use regex::Regex;

/// The season and episode a name `trname` wrote (`Show S01E14.mkv`) is of, as
/// written (`("S02E05.5.mp4")` is `(2, "05.5")`), whatever precedes the `S`.
pub fn season_episode(name: &str) -> Option<(u32, String)> {
    static NAME: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)S(\d{2,})E(\d{2,}(?:\.\d)?)\.\w+$").unwrap());
    let found = NAME.captures(name)?;
    Some((found[1].parse().ok()?, found[2].to_owned()))
}

/// Whether `name` is the name `trname` gives under the work folder `title`
/// (`<title>/Season NN`): the title and an episode, as in `<title> S01E05.mkv`,
/// `S01E05.5` or a three-digit `S01E105`, with the title's case not counting.
/// `trname` itself takes its form only under the title as written, with a
/// two-digit season and at most four digits of episode, and reads any other
/// name as a release's. A release that merely ends in `SxxEyy` under another
/// title is not one.
pub fn is_trname_name(name: &str, title: &str) -> bool {
    static EPISODE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)^ S\d{2,}E\d{2,}(?:\.\d)?\.\w+$").unwrap());
    match name.get(..title.len()) {
        Some(head) if head.to_lowercase() == title.to_lowercase() => {
            EPISODE.is_match(&name[title.len()..])
        }
        _ => false,
    }
}

/// The `SxxEyy` a library file name ends with (before its extension and, for a
/// subtitle, before language fragments): the season and the episode as
/// written. A letter or digit before the `S` makes it no `SxxEyy`.
pub fn listed_episode(name: &str, subtitle: bool) -> Option<(u32, String)> {
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
        if !subtitle || fragments == 2 {
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

/// The season of a library folder named `Season NN`, in either case.
pub fn season_folder(name: &str) -> Option<u32> {
    static SEASON: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)^Season\s+(\d{1,4})$").unwrap());
    SEASON.captures(name)?[1].parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_episode_name_gives_its_season_and_episode() {
        assert_eq!(
            season_episode("Show S01E14.mkv"),
            Some((1, "14".to_owned()))
        );
        assert_eq!(
            season_episode("Show S02E05.5.mp4"),
            Some((2, "05.5".to_owned()))
        );
        assert_eq!(season_episode("[SubsPlease] Show - 14.mkv"), None);
    }

    #[test]
    fn episodes_are_read_as_written() {
        let video = |n: &str| listed_episode(n, false);
        assert_eq!(video("Show S01E01.mkv"), Some((1, "01".into())));
        assert_eq!(video("Show S02E17.5.mkv"), Some((2, "17.5".into())));
        assert_eq!(video("Show S01E105.mkv"), Some((1, "105".into())));
        assert_eq!(video("show s01e03.MP4"), Some((1, "03".into())));
        assert_eq!(video("Show S01E01 1080p.mkv"), None);
        assert_eq!(video("ShowS01E01.mkv"), None);
        assert_eq!(video("Show.mkv"), None);
        // Only a subtitle has a language fragment.
        assert_eq!(video("Show S01E01.ko.mkv"), None);
        let sub = |n: &str| listed_episode(n, true);
        assert_eq!(sub("Show S01E01.ass"), Some((1, "01".into())));
        assert_eq!(sub("Show S01E01.ko.smi"), Some((1, "01".into())));
        assert_eq!(sub("Show S01E02.5.ko.forced.ass"), Some((1, "02.5".into())));
        assert_eq!(sub("Show S01E02.a.b.c.ass"), None);
        assert_eq!(sub("Show S01E02.한국어.ass"), None);
    }

    #[test]
    fn season_folders_are_season_and_a_number() {
        assert_eq!(season_folder("Season 01"), Some(1));
        assert_eq!(season_folder("Season 0"), Some(0));
        assert_eq!(season_folder("season 12"), Some(12));
        assert_eq!(season_folder("Season"), None);
        assert_eq!(season_folder("Season 1 extras"), None);
        assert_eq!(season_folder("Specials"), None);
        assert_eq!(season_folder("Season -1"), None);
    }
}
