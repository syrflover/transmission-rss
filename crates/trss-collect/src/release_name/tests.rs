use super::*;

fn without_version(name: &str) -> String {
    ReleaseName::read(name).without_revision().to_owned()
}

/// The groups are the bracketed names before the work; the work and the
/// episode of a name are checked in the release-name corpus.
#[test]
fn the_groups_are_the_bracketed_names_before_the_work() {
    assert_eq!(
        ReleaseName::read("[SubsPlease] Work - 01 (1080p)").groups,
        ["SubsPlease"]
    );
}

#[test]
fn a_blank_title_has_no_work() {
    assert!(ReleaseName::read("   ").work.is_none());
}

#[test]
fn the_release_carries_the_stem_the_version_and_the_crc() {
    let read = ReleaseName::read("[SubsPlease] Show - 14v2 (1080p) [1A2B3C4D].mkv");
    assert_eq!(read.stem, "[SubsPlease] Show - 14 (1080p)");
    assert_eq!(read.version, 2);
    assert_eq!(read.crc, Some(0x1A2B3C4D));
}

#[test]
fn the_notation_is_read_from_the_title() {
    let width = |title: &str| ReleaseName::read(title).notation;
    assert_eq!(
        width("[SubsPlease] One Piece - 1000 (1080p) [AAAA1111].mkv"),
        Some(Notation::Dash { width: 4 })
    );
    assert_eq!(
        width("[SubsPlease] Show - 05 (1080p) [AAAA1111].mkv"),
        Some(Notation::Dash { width: 2 })
    );
    assert_eq!(
        width("[SubsPlease] Show - 5 (1080p) [AAAA1111].mkv"),
        Some(Notation::Dash { width: 1 })
    );
    assert_eq!(
        width("Show S02E05 1080p WEB.mkv"),
        Some(Notation::SeasonEpisode {
            season: 2,
            season_width: 2,
            width: 2
        })
    );
    assert_eq!(width("[SubsPlease] Show (01-12) (1080p) [Batch]"), None);
}

#[test]
fn a_notation_writes_a_search_for_episodes_as_the_release_does() {
    assert_eq!(
        Notation::Dash { width: 4 }.alternatives(&[1000, 1001, 1002]),
        " - (1000|1001|1002)"
    );
    assert_eq!(
        Notation::Dash { width: 2 }.alternatives(&[1, 12]),
        " - (01|12)"
    );
    assert_eq!(Notation::Dash { width: 1 }.alternatives(&[7]), " - (7)");
    assert_eq!(
        Notation::SeasonEpisode {
            season: 2,
            season_width: 2,
            width: 2
        }
        .alternatives(&[5, 6]),
        " (S02E05|S02E06)"
    );
}

#[test]
fn a_subsplease_name_gives_its_crc_and_its_revision() {
    let v1 = ReleaseName::read("[SubsPlease] Sono Bisque Doll - 14 (1080p) [E2675E51].mkv");
    let v2 = ReleaseName::read("[SubsPlease] Sono Bisque Doll - 14v2 (1080p) [1A2B3C4D].mkv");
    assert_eq!(v1.stem, "[SubsPlease] Sono Bisque Doll - 14 (1080p)");
    assert_eq!((v1.version, v1.crc), (1, Some(0xE2675E51)));
    assert_eq!(v2.stem, v1.stem);
    assert_eq!((v2.version, v2.crc), (2, Some(0x1A2B3C4D)));
}

#[test]
fn an_erai_raws_name_takes_the_hex_bracket_before_the_extension() {
    let release = ReleaseName::read(
        "[Erai-raws] Kimi to Idol Precure - 06v2 [1080p CR WEBRip HEVC AAC][MultiSub][1BBD34E6].mkv",
    );
    assert_eq!(release.crc, Some(0x1BBD34E6));
    assert_eq!(release.version, 2);
    assert_eq!(
        release.stem,
        "[Erai-raws] Kimi to Idol Precure - 06 [1080p CR WEBRip HEVC AAC][MultiSub]"
    );
    // An RSS title without the extension reads the same.
    let title = ReleaseName::read(
        "[Erai-raws] Kimi to Idol Precure - 06v2 [1080p CR WEBRip HEVC AAC][MultiSub][1BBD34E6]",
    );
    assert_eq!(
        (title.stem, title.version, title.crc),
        (release.stem, release.version, release.crc)
    );
}

/// Erai-raws' magnet feed writes a revision as `- 01 (V2)`, with no
/// extension and no CRC32, and lists the subtitle languages at the end.
#[test]
fn a_revision_in_parentheses_after_the_episode_is_read_like_nvm() {
    let first = "[Magnet] Kusuriya no Hitorigoto 3rd Season - 01 [1080p CR WEB-DL AVC AAC][us][br][mx][es][sa][fr][de][it][ru][Airing]";
    let second = "[Magnet] Kusuriya no Hitorigoto 3rd Season - 01 (V2) [1080p CR WEB-DL AVC AAC][us][br][mx][es][sa][fr][de][it][ru][pl][Airing]";
    let (v1, v2) = (ReleaseName::read(first), ReleaseName::read(second));
    assert_eq!((v1.version, v2.version), (1, 2));
    assert_eq!(v2.kind, Kind::Episode(Episode::whole(1)));
    assert_eq!(
        v2.work.as_deref(),
        Some("Kusuriya no Hitorigoto 3rd Season")
    );
    assert_eq!((v2.crc, v2.notation), (None, v1.notation));
    // The mark is left out of the name like `v2` is.
    assert_eq!(
        v2.stem,
        "[Magnet] Kusuriya no Hitorigoto 3rd Season - 01 [1080p CR WEB-DL AVC AAC][us][br][mx][es][sa][fr][de][it][ru][pl][Airing]"
    );
    assert_eq!(
        v2.without_revision(),
        v2.stem,
        "no extension and no CRC32 to take away"
    );
    assert_eq!(v1.without_revision(), first);
    // The language list differs between the two; the release does not.
    assert_ne!(v1.stem, v2.stem);
    assert_eq!(v1.release_key(), v2.release_key());
    assert_eq!(
        v1.release_key(),
        "[Magnet] Kusuriya no Hitorigoto 3rd Season - 01 [1080p CR WEB-DL AVC AAC][Airing]"
    );

    for (name, version) in [
        ("[Magnet] Show - 01 (v3) [1080p][us][Airing]", 3),
        ("[Magnet] Show - 12.5 (V2) [1080p][us]", 2),
        ("[Magnet] Show - 01 (V2)", 2),
        ("[Magnet] Show - 01(V2) [1080p]", 2),
        // The mark counts right after the episode's number only.
        ("[Magnet] Show (V2) - 01 [1080p][us]", 1),
        ("[Magnet] Show - 01 (NF) [1080p][us]", 1),
        ("[Magnet] Show - 01 [1080p (V2)][us]", 1),
        // `NvM` is the revision of a name that has it.
        ("[Group] Show - 01v4 (V2) [1080p]", 4),
        // The show's own `NvM` is not the revision.
        ("[Group] Show 3v3 - 06 (V2) [1080p]", 2),
        // Nor is a mark that ` - ` and a number follow: it is in the name.
        ("[Group] Show - 2 (V3) - 05 [1080p]", 1),
        ("[Group] Show - 2 (V3) - 05 (V2) [1080p]", 2),
        ("[Group] Show - 2 (V3) - 05 [1080p - 2]", 1),
        ("[Group] Show - 2 (V3) [1080p - 3]", 3),
    ] {
        assert_eq!(ReleaseName::read(name).version, version, "{name}");
    }
    assert_eq!(
        without_version("[Magnet] Show - 01 (V2)"),
        "[Magnet] Show - 01"
    );
    assert_eq!(
        ReleaseName::read("[Group] Show 3v3 - 06 (V2) [1080p]").stem,
        "[Group] Show 3v3 - 06 [1080p]"
    );
}

/// The language tags are left out of the comparison only, and only the
/// bracketed two-letter lower-case ones.
#[test]
fn the_release_key_leaves_out_the_bracketed_language_tags_only() {
    let key = |name: &str| ReleaseName::read(name).release_key().into_owned();
    assert_eq!(
        key("[Magnet] Show - 01 [1080p CR WEB-DL AVC AAC][us][br][pl][Airing]"),
        "[Magnet] Show - 01 [1080p CR WEB-DL AVC AAC][Airing]"
    );
    // The revision, the CRC32 and the extension are not in the stem either.
    assert_eq!(
        key("[Erai-raws] Show - 06v2 [1080p CR WEBRip HEVC AAC][MultiSub][1BBD34E6].mkv"),
        "[Erai-raws] Show - 06 [1080p CR WEBRip HEVC AAC][MultiSub]"
    );
    // A name without such tags compares as its stem.
    for name in [
        "[SubsPlease] Show - 14v2 (1080p) [1A2B3C4D].mkv",
        "[Group] Show - 05 [1080p][MultiSub][Airing]",
        "[Group] Show - 05 [US][1080p][eng][e5]",
    ] {
        let read = ReleaseName::read(name);
        assert_eq!(read.release_key(), read.stem, "{name}");
    }
    // Another group's release of the episode is still another release.
    assert_ne!(
        key("[Magnet] Show - 01 [1080p][us]"),
        key("[Other] Show - 01 [1080p][us]")
    );
}

#[test]
fn a_number_v_number_in_the_show_name_is_not_the_revision() {
    let name = "[SubsPlease] Show 3v3 - 06v2 (1080p) [1A2B3C4D].mkv";
    let release = ReleaseName::read(name);
    assert_eq!(release.version, 2);
    assert_eq!(release.stem, "[SubsPlease] Show 3v3 - 06 (1080p)");
    assert_eq!(
        without_version(name),
        "[SubsPlease] Show 3v3 - 06 (1080p) [1A2B3C4D].mkv"
    );
}

/// A show named with `NvM` whose episode carries no revision marker is
/// the first revision of that episode: only a marker on the episode's
/// number counts.
#[test]
fn a_number_v_number_in_the_show_name_of_an_unversioned_episode_is_no_revision() {
    let first = "Show 3v3 - 06 [1080p].mkv";
    let second = "Show 3v3 - 06v2 [1080p].mkv";
    let v1 = ReleaseName::read(first);
    let v2 = ReleaseName::read(second);
    assert_eq!((v1.version, v1.stem.as_str()), (1, "Show 3v3 - 06 [1080p]"));
    assert_eq!((v2.version, v2.stem.as_str()), (2, "Show 3v3 - 06 [1080p]"));
    assert_eq!(without_version(first), first);
    assert_eq!(without_version(second), first);
    // The same with a CRC32, and with the extension left out.
    let named = "[SubsPlease] Show 3v3 - 06 (1080p) [1A2B3C4D].mkv";
    assert_eq!(ReleaseName::read(named).version, 1);
    assert_eq!(without_version(named), named);
    let title = "[SubsPlease] Show 3v3 - 06 (1080p) [1A2B3C4D]";
    assert_eq!(ReleaseName::read(title).version, 1);
    assert_eq!(
        ReleaseName::read(title).stem,
        "[SubsPlease] Show 3v3 - 06 (1080p)"
    );
    // A revision marker in brackets right after the show is still read.
    let bracketed = "[Group] Show [06v2][1080p].mkv";
    assert_eq!(ReleaseName::read(bracketed).version, 2);
}

/// The `NvM` right after ` - ` is the episode's and its revision, whatever
/// numbers follow it (audio channels, a part); one in the show's name is
/// followed by ` - ` and the episode's number.
#[test]
fn the_revision_on_the_episode_number_is_read_whatever_follows() {
    for (name, version, stem) in [
        (
            "[Group] Show - 03v2 1080p WEB AAC 2.0 x264.mkv",
            2,
            "[Group] Show - 03 1080p WEB AAC 2.0 x264",
        ),
        (
            "[Group] Show - 03v2 - Part 2.mkv",
            2,
            "[Group] Show - 03 - Part 2",
        ),
        ("Show 2 - 03v2.mkv", 2, "Show 2 - 03"),
        ("Show - 03v2 (2024).mkv", 2, "Show - 03 (2024)"),
        ("Show S2 - 03v2 [1080p].mkv", 2, "Show S2 - 03 [1080p]"),
        ("86 - 03v2.mkv", 2, "86 - 03"),
        ("Re:Zero 3v3 - 06.mkv", 1, "Re:Zero 3v3 - 06"),
        ("Re:Zero 3v3 - 06v2.mkv", 2, "Re:Zero 3v3 - 06"),
        (
            "[Group] Show 14v2 (1080p).mkv",
            2,
            "[Group] Show 14 (1080p)",
        ),
    ] {
        let release = ReleaseName::read(name);
        assert_eq!(
            (release.version, release.stem.as_str()),
            (version, stem),
            "{name}"
        );
    }
    assert_eq!(
        without_version("[Group] Show - 03v2 - Part 2.mkv"),
        "[Group] Show - 03 - Part 2.mkv"
    );
}

#[test]
fn a_v_number_that_does_not_follow_a_number_is_no_revision() {
    for name in [
        "[Group] Gundam V2 - 06 (1080p) [1A2B3C4D].mkv",
        "[Group] Show Ver.2 - 06 (1080p) [1A2B3C4D].mkv",
        "[Group] Show S01E06v2 (1080p) [1A2B3C4D].mkv",
        "[Group] Show - 06 (x264v2) [1A2B3C4D].mkv",
    ] {
        let release = ReleaseName::read(name);
        assert_eq!(release.version, 1, "{name}");
        assert_eq!(without_version(name), name, "{name}");
    }
}

#[test]
fn a_name_without_its_revision_keeps_everything_else() {
    assert_eq!(
        without_version(
            "[Erai-raws] Show - 06v2 [1080p CR WEBRip HEVC AAC][MultiSub][1BBD34E6].mkv"
        ),
        "[Erai-raws] Show - 06 [1080p CR WEBRip HEVC AAC][MultiSub][1BBD34E6].mkv"
    );
    assert_eq!(
        without_version("[SubsPlease] Show - 14v2 (1080p) [8F2EFECC].mkv"),
        "[SubsPlease] Show - 14 (1080p) [8F2EFECC].mkv"
    );
    let first = "[SubsPlease] Show - 14 (1080p) [8F2EFECC].mkv";
    assert_eq!(without_version(first), first);
}

#[test]
fn a_last_bracket_that_is_not_eight_hex_digits_is_no_crc() {
    for name in [
        "[Erai-raws] Show - 06 [1080p][1BBD34E6][MultiSub].mkv",
        "[SubsPlease] Show - 14v2 (1080p).mkv",
        "[Group] Show - 14 [1BBD34E].mkv",
        "[Group] Show - 14 [1BBD34EG].mkv",
    ] {
        assert_eq!(ReleaseName::read(name).crc, None, "{name}");
    }
    assert_eq!(
        ReleaseName::read("[SubsPlease] Show - 14v2 (1080p).mkv").version,
        2
    );
}

#[test]
fn another_groups_release_of_the_episode_is_another_release() {
    let a = ReleaseName::read("[SubsPlease] Show - 14 (1080p) [E2675E51].mkv");
    let b = ReleaseName::read("[Erai-raws] Show - 14 [1080p][E2675E51].mkv");
    assert_ne!(a.stem, b.stem);
    let c = ReleaseName::read("[SubsPlease] Show - 15 (1080p) [E2675E51].mkv");
    assert_ne!(a.stem, c.stem);
}

/// One extension, whichever reader asks: a dot and two to four letters or
/// digits, or `torrent`.
#[test]
fn every_reading_takes_the_same_extension_off() {
    let titled = "[Group] Show - 05v2 (1080p) [ABCD1234]";
    for ext in [".torrent", ".Torrent", ".mkv", ".webm", ""] {
        let read = ReleaseName::read(&format!("{titled}{ext}"));
        assert_eq!(read.crc, Some(0xABCD1234), "{ext}");
        assert_eq!(read.stem, "[Group] Show - 05 (1080p)", "{ext}");
        assert_eq!(read.version, 2, "{ext}");
        assert_eq!(read.work.as_deref(), Some("Show"), "{ext}");
        assert_eq!(read.whole_episode(), Some(5), "{ext}");
    }
    // The work reader loses the same suffixes the revision reader does, so
    // `.webm` and a dot that starts two to four characters (`Vol.12`,
    // `H.264`) are no part of a name.
    assert_eq!(ReleaseName::read("Show - 05.webm").whole_episode(), Some(5));
    assert_eq!(
        ReleaseName::read("Show Vol.12").work.as_deref(),
        Some("Show Vol")
    );
    assert_eq!(
        ReleaseName::read("[AnoZu] One Piece S23E22 1080p CR WEB-DL AAC 2.0 H.264").stem,
        "[AnoZu] One Piece S23E22 1080p CR WEB-DL AAC 2.0 H"
    );
}

/// One revision mark, whichever reader asks: `v` or `V` and one or two digits.
#[test]
fn every_reading_takes_the_same_revision_mark() {
    for mark in ["v2", "V2", "v12"] {
        let read = ReleaseName::read(&format!("[Group] Show - 05{mark} (1080p)"));
        assert_eq!(read.version, mark[1..].parse::<u32>().unwrap(), "{mark}");
        assert_eq!(read.whole_episode(), Some(5), "{mark}");
        assert_eq!(read.kind, Kind::Episode(Episode::whole(5)), "{mark}");
        assert_eq!(
            read.without_revision(),
            "[Group] Show - 05 (1080p)",
            "{mark}"
        );
        let bare = ReleaseName::read(&format!("[Group] Show 05{mark} (1080p)"));
        assert_eq!(bare.work.as_deref(), Some("Show"), "{mark}");
        assert_eq!(bare.whole_episode(), Some(5), "{mark}");
    }
    // Three digits are no revision mark, so the number is no episode.
    let read = ReleaseName::read("[Group] Show - 05v123 (1080p)");
    assert_eq!(read.version, 1);
    assert_eq!(read.whole_episode(), None);
    assert_eq!(read.kind, Kind::Unnumbered);
    assert_eq!(read.work.as_deref(), Some("Show - 05v123"));
    assert_eq!(read.without_revision(), "[Group] Show - 05v123 (1080p)");
}

/// One season, `S` and one or two digits, for the episode and its notation.
#[test]
fn a_season_has_one_or_two_digits() {
    assert_eq!(
        ReleaseName::read("Show S12E05 1080p").notation,
        Some(Notation::SeasonEpisode {
            season: 12,
            season_width: 2,
            width: 2
        })
    );
    // `S100E05` is no season, so only the dash number gives the notation.
    let read = ReleaseName::read("Show S100E05 - 05 (1080p)");
    assert_eq!(read.notation, Some(Notation::Dash { width: 2 }));
    assert_eq!(
        ReleaseName::read("Show S100E05 (1080p)").kind,
        Kind::Unnumbered
    );
}

/// One dash, spaces on both sides, sets an episode's number apart from the
/// work, for the episode, its revision and its notation.
#[test]
fn an_episode_follows_a_dash_with_a_space_on_each_side() {
    for dash in [" - ", "  -  ", " -  "] {
        let read = ReleaseName::read(&format!("Show{dash}05 (1080p)"));
        assert_eq!(read.notation, Some(Notation::Dash { width: 2 }), "{dash:?}");
        assert_eq!(read.whole_episode(), Some(5), "{dash:?}");
    }
    // Without the spaces the dash is part of the work, and the number after
    // it is not an episode's: a `3v3` before it is the work's, not a revision.
    for name in ["Show -05 (1080p)", "Show-05 (1080p)"] {
        let read = ReleaseName::read(name);
        assert_eq!(read.whole_episode(), None, "{name}");
        assert_eq!(read.notation, None, "{name}");
    }
    let spaced = ReleaseName::read("Show 3v3 - 06 (1080p)");
    assert_eq!(
        (spaced.version, spaced.stem.as_str()),
        (1, "Show 3v3 - 06 (1080p)")
    );
    let tight = ReleaseName::read("Show 3v3 -06 (1080p)");
    assert_eq!(
        (tight.version, tight.stem.as_str()),
        (3, "Show 3 -06 (1080p)")
    );
}

/// What `trname` reads an episode from: the name without its revision, and
/// nothing for a name this crate reads as no episode or as a batch.
#[test]
fn trname_reads_only_the_names_of_episodes() {
    assert_eq!(
        name_for_trname("[SubsPlease] Show - 14v2 (1080p) [8F2EFECC].mkv").as_deref(),
        Some("[SubsPlease] Show - 14 (1080p) [8F2EFECC].mkv")
    );
    assert_eq!(
        name_for_trname("[SubsPlease] Show - 14.5 (1080p) [8F2EFECC].mkv").as_deref(),
        Some("[SubsPlease] Show - 14.5 (1080p) [8F2EFECC].mkv")
    );
    assert_eq!(
        name_for_trname("Show S01E14.mkv").as_deref(),
        Some("Show S01E14.mkv")
    );
    for name in [
        "[Group] Show Movie (BD 1080p) [ABCD1234].mkv",
        "[Group] Show - 01-12 (1080p).mkv",
        "[Group] Show (01-12) [Batch].mkv",
        "[Group] Show Complete [Batch].mkv",
        "[Group] Show - 12.0 (1080p).mkv",
    ] {
        assert_eq!(name_for_trname(name), None, "{name}");
    }
}
