//! What a cycle hands a new season's rule before its first item is named
//! ([`settle`]): the offset the app sets from the first release and the
//! earlier seasons, the rules it leaves alone, and a rule the user saved
//! while the cycle ran. The grounds themselves (`decide`) are tested in
//! [`crate::episode_offset`]; here the library, the history and the rule's
//! stored offset are real, on the database of a [`World`].

use std::collections::HashMap;

use super::*;
use crate::{
    store::{
        channels::RuleInput,
        history::{HistoryResult, Observation},
    },
    test_world::{show_hash, show_title, World},
};

/// What the user does to the rule while the cycle reads its feed.
#[derive(Clone, Copy)]
enum Meanwhile {
    Nothing,
    /// Saves the rule as it was (it still makes a new version).
    SavesItAsItWas,
    /// Types this offset.
    TypesOffset(i64),
    /// Changes what the rule matches.
    ChangesMatch(&'static str),
}

/// A rule of `Show` and the library the earlier seasons are read from.
struct Case {
    name: &'static str,
    /// The AniList episodes of seasons 1 and 2, which are linked.
    counts: [Option<u32>; 2],
    /// The videos the rule's season 3 folder holds already.
    held: &'static [&'static str],
    directory: &'static str,
    /// The offset in the rule's field.
    field: i64,
    /// The app set `-24` and the user typed this over it.
    decided_then_typed: Option<i64>,
    /// The release the rule picked in an earlier cycle.
    picked: Option<u32>,
    /// The releases the cycle is about to receive for the rule.
    firsts: &'static [u32],
    meanwhile: Meanwhile,
    /// The offset the cycle's items take beside the one the rule was read
    /// with.
    handed: Option<i64>,
    /// `(episode, episode_auto)` stored after.
    stored: (i64, bool),
    /// The offset the app replaced, kept while the app's is in force.
    replaced: Option<i64>,
    /// Pieces of the grounds stored with the app's offset.
    grounds: &'static [&'static str],
}

impl Case {
    const fn new(name: &'static str, firsts: &'static [u32]) -> Case {
        Case {
            name,
            counts: [Some(12), Some(12)],
            held: &[],
            directory: "Show/Season 03",
            field: 1,
            decided_then_typed: None,
            picked: None,
            firsts,
            meanwhile: Meanwhile::Nothing,
            handed: None,
            stored: (1, false),
            replaced: None,
            grounds: &[],
        }
    }
}

impl World {
    /// A history record of the release `n` picked by the rule.
    async fn picked_by(&self, rule: &Rule, n: u32) {
        self.ctx
            .history
            .record(
                self.now(),
                vec![Observation {
                    channel_id: self.channel_id.clone(),
                    channel_label: "https://feeds.example.test/show".into(),
                    identity_key: format!("guid:{n}"),
                    title: show_title(n),
                    link: crate::test_world::magnet(&show_hash(n), &show_title(n)),
                    result: HistoryResult::Received,
                    rule_id: Some(rule.id.clone()),
                    torrent_hash: Some(show_hash(n)),
                    reason: None,
                }],
            )
            .await
            .unwrap();
    }
}

/// The rule is looked at once, by the first items it is about to receive; a
/// rule the app may not decide, or has decided, or that picked before, is
/// left as it is.
#[tokio::test]
async fn the_first_items_of_a_rule_decide_its_offset_once() {
    let cases = [
        Case {
            handed: Some(-24),
            stored: (-24, true),
            replaced: Some(1),
            grounds: &["24화", "25화"],
            ..Case::new(
                "a_third_seasons_first_release_gets_the_sum_of_the_earlier_ones",
                &[25],
            )
        },
        // The field's `1` names the release as `0` would: nothing changes,
        // and there is nothing to tell (decision of 2026-10-02: a field that
        // already has the value is left alone).
        Case {
            directory: "Fresh/Season 01",
            ..Case::new(
                "a_new_works_first_release_is_not_converted_and_nobody_is_asked",
                &[1],
            )
        },
        // A third season's rule copied from the second keeps the second's
        // `−24`; its first release `- 49` follows the 48 episodes of seasons
        // 1 and 2.
        Case {
            counts: [Some(24), Some(24)],
            field: -24,
            handed: Some(-48),
            stored: (-48, true),
            replaced: Some(-24),
            grounds: &["49화", "−48"],
            ..Case::new(
                "a_value_carried_over_from_the_previous_season_gives_way_to_the_whole_sum",
                &[49],
            )
        },
        Case {
            counts: [Some(24), Some(24)],
            field: -48,
            stored: (-48, false),
            ..Case::new("a_field_that_holds_the_sum_already_is_left_as_it_is", &[49])
        },
        // Received as it is, with nothing decided: the grounds only suggest.
        Case::new(
            "a_first_release_in_the_middle_of_a_season_is_suggested_and_received_unconverted",
            &[27],
        ),
        // Season 2's entry does not know its episode count.
        Case {
            counts: [Some(12), None],
            ..Case::new(
                "earlier_seasons_without_a_known_count_are_never_guessed",
                &[25],
            )
        },
        // A split cour: the first cour's episodes are in the third season
        // already.
        Case {
            held: &["01", "02"],
            ..Case::new(
                "a_season_folder_that_has_videos_already_keeps_the_app_from_choosing",
                &[25],
            )
        },
        // Before the first item the app decides whatever the field holds
        // (user decision, 2026-10-02): a `−12` the user typed gives way to
        // the sum, and the grounds say what it was.
        Case {
            field: -12,
            handed: Some(-24),
            stored: (-24, true),
            replaced: Some(-12),
            grounds: &["25화", "−24"],
            ..Case::new(
                "an_offset_typed_before_the_first_item_gives_way_to_the_sum",
                &[25],
            )
        },
        // The app set `−24` with the first item and the user typed `−12`
        // over it: the rule is the user's, and its next items change
        // nothing.
        Case {
            decided_then_typed: Some(-12),
            picked: Some(25),
            stored: (-12, false),
            ..Case::new(
                "an_automatic_value_the_user_changes_loses_its_mark_and_stays_changed",
                &[26],
            )
        },
        // The same, though nothing is recorded of the items the app decided
        // from: it is the app's decision that is never made again.
        Case {
            decided_then_typed: Some(-12),
            stored: (-12, false),
            ..Case::new(
                "an_automatic_value_the_user_changes_is_never_decided_again_whatever_the_history",
                &[26],
            )
        },
        // The first thing the rule picked is episode 27: nothing is decided,
        // and a later release that would have been a season's first changes
        // nothing.
        Case {
            picked: Some(27),
            ..Case::new(
                "a_rule_that_picked_items_before_is_not_decided_by_later_ones",
                &[25],
            )
        },
        // Two seasons of 24, a third that starts at `- 25`: counted on from
        // season 2, which is not set by the app (`- 25` could as well be a
        // season that restarts).
        Case {
            counts: [Some(24), Some(24)],
            field: 0,
            stored: (0, false),
            ..Case::new(
                "numbers_run_on_from_the_season_before_are_offered_and_never_set",
                &[25],
            )
        },
        // A save that changes nothing the decision rests on (the form saved
        // as it was) still makes a new version: the rule is decided again.
        Case {
            meanwhile: Meanwhile::SavesItAsItWas,
            handed: Some(-24),
            stored: (-24, true),
            replaced: Some(1),
            ..Case::new(
                "a_rule_saved_while_its_first_release_is_read_still_gets_the_offset",
                &[25],
            )
        },
        // The user's offset wins, and the items are named with it.
        Case {
            meanwhile: Meanwhile::TypesOffset(0),
            handed: Some(0),
            stored: (0, false),
            ..Case::new(
                "an_offset_the_user_saves_while_the_first_release_is_read_is_kept",
                &[25],
            )
        },
        // The items of this cycle were picked by the phrase the cycle read.
        Case {
            meanwhile: Meanwhile::ChangesMatch("Show -"),
            ..Case::new(
                "a_rule_whose_match_changes_while_its_first_release_is_read_is_not_decided",
                &[25],
            )
        },
        // The user's value saved while the first release is read wins, and
        // this cycle's items are named with it, not with the value the cycle
        // read.
        Case {
            field: -12,
            meanwhile: Meanwhile::TypesOffset(-20),
            handed: Some(-20),
            stored: (-20, false),
            ..Case::new(
                "the_first_items_take_the_value_the_user_saves_meanwhile_not_the_one_read",
                &[25],
            )
        },
    ];

    for case in cases {
        let s = World::bare().await;
        let mut spec: Vec<(u32, &[&str])> = vec![(1, &["01", "02"]), (2, &["01", "02"])];
        if !case.held.is_empty() {
            spec.push((3, case.held));
        }
        let place = s.library_of(&spec).await;
        place.link(1, &[case.counts[0]]).await;
        place.link(2, &[case.counts[1]]).await;
        s.advance(1_000);
        let mut rule = s.subscribe("Show", case.directory, 7, case.field).await;
        if let Some(typed) = case.decided_then_typed {
            let set = s
                .ctx
                .channels
                .set_auto_episode(
                    &rule.id,
                    rule.version,
                    -24,
                    "첫 화가 25화라서 −24로 정했어요.",
                )
                .await
                .unwrap()
                .expect("the rule was at the version read");
            rule = s
                .ctx
                .channels
                .update_rule(
                    &set.id,
                    set.version,
                    &s.channel_id,
                    RuleInput {
                        episode: typed,
                        episode_auto: false,
                        ..set.to_input()
                    },
                )
                .await
                .unwrap();
        }
        if let Some(n) = case.picked {
            s.picked_by(&rule, n).await;
        }
        let name = case.name;

        // The cycle reads the rule, then the user saves it.
        let open = HashMap::from([(rule.id.clone(), rule.clone())]);
        let firsts = HashMap::from([(
            rule.id.clone(),
            case.firsts
                .iter()
                .map(|n| show_title(*n))
                .collect::<Vec<_>>(),
        )]);
        match case.meanwhile {
            Meanwhile::Nothing => {}
            Meanwhile::SavesItAsItWas => {
                let saved = s
                    .ctx
                    .channels
                    .update_rule(&rule.id, rule.version, &s.channel_id, rule.to_input())
                    .await
                    .unwrap();
                assert_ne!(saved.version, rule.version, "{name}");
            }
            Meanwhile::TypesOffset(offset) => {
                let mut input = rule.to_input();
                input.episode = offset;
                s.ctx
                    .channels
                    .update_rule(&rule.id, rule.version, &s.channel_id, input)
                    .await
                    .unwrap();
            }
            Meanwhile::ChangesMatch(phrase) => {
                let mut input = rule.to_input();
                input.r#match = Some(phrase.to_owned());
                s.ctx
                    .channels
                    .update_rule(&rule.id, rule.version, &s.channel_id, input)
                    .await
                    .unwrap();
            }
        }

        let handed = settle(&s.ctx.offsets(), s.media.to_str().unwrap(), &open, &firsts).await;

        assert_eq!(handed.get(&rule.id).copied(), case.handed, "{name}");
        let stored = s.stored_rule(&rule).await;
        assert_eq!((stored.episode, stored.episode_auto), case.stored, "{name}");
        let mark = s.mark_of(&rule).await;
        assert_eq!(mark.previous, case.replaced, "{name}");
        let basis = mark.basis.unwrap_or_default();
        for piece in case.grounds {
            assert!(basis.contains(piece), "{name}: {basis}");
        }
        assert_eq!(
            mark.decided,
            case.stored.1 || case.decided_then_typed.is_some(),
            "{name}"
        );
        if case.handed.is_none() && matches!(case.meanwhile, Meanwhile::Nothing) {
            assert_eq!(stored.version, rule.version, "{name}");
        }
    }
}

/// The cycle names the first item with the offset it decided before adding
/// it, and the release after it with the same value; the item already
/// received keeps its name.
#[tokio::test]
async fn the_first_item_of_a_third_season_is_named_from_the_sum_of_the_earlier_ones() {
    let s = World::bare().await;
    let place = s
        .library_of(&[(1, &["01", "02"]), (2, &["01", "02"])])
        .await;
    place.link(1, &[Some(12)]).await;
    place.link(2, &[Some(12)]).await;
    s.advance(1_000);
    let rule = s.subscribe("Show", "Show/Season 03", 7, 1).await;

    // The first read leaves what the feed held; the new season's first
    // release comes after it.
    s.feed_shows(&[24]);
    s.cycle_later().await;
    assert!(s.torrent_names().is_empty());
    s.feed_shows(&[24, 25]);
    s.cycle_later().await;

    assert_eq!(s.torrent_names(), ["Show S03E01.mkv"]);
    let stored = s.stored_rule(&rule).await;
    assert_eq!((stored.episode, stored.episode_auto), (-24, true));

    // The decision is made once: the next release is converted by the same
    // value, and the item already received keeps its name.
    s.feed_shows(&[24, 25, 26]);
    s.cycle_later().await;
    assert_eq!(s.torrent_names(), ["Show S03E01.mkv", "Show S03E02.mkv"]);
    assert_eq!(s.stored_rule(&rule).await.episode, -24);
}

/// A second cour whose first release restarts at `- 01` is received as it is
/// (`S02E01` is the first cour's), and once the user sets the offset the
/// cycle names it too: a rename an earlier cycle could not make is finished.
#[tokio::test]
async fn a_cycle_finishes_the_rename_of_a_restarted_cour_it_could_not_make_before() {
    let s = World::bare().await;
    let season = s.media.join("Show/Season 02");
    std::fs::create_dir_all(&season).unwrap();
    for e in 1..=12 {
        std::fs::write(season.join(format!("Show S02E{e:02}.mkv")), "x").unwrap();
    }
    let twelve: Vec<String> = (1..=12).map(|e| format!("{e:02}")).collect();
    let twelve: Vec<&str> = twelve.iter().map(String::as_str).collect();
    let place = s.library_of(&[(1, &["01"]), (2, &twelve)]).await;
    place.link(1, &[Some(12)]).await;
    place.link(2, &[Some(12), Some(12)]).await;
    s.advance(1_000);
    let rule = s.subscribe("Show", "Show/Season 02", 8, 0).await;
    s.feed_shows(&[]);
    s.cycle_later().await;
    s.feed_shows(&[1]);
    s.cycle_later().await;

    // Never set by the app: received as it is, so the video keeps its
    // release name.
    assert_eq!(
        s.torrent_names(),
        ["[SubsPlease] Show - 01 (1080p) [ABCD0001].mkv"]
    );
    let stored = s.stored_rule(&rule).await;
    assert_eq!((stored.episode, stored.episode_auto), (0, false));

    // The user applies the suggestion (the offset `13`): the cour's releases
    // are named on from the first cour's twelve, and `- 01`, which the feed
    // still shows, is named now too.
    s.ctx
        .channels
        .set_episode(&rule.id, stored.version, 13)
        .await
        .unwrap();
    s.feed_shows(&[1, 2]);
    s.cycle_later().await;
    assert_eq!(s.torrent_names(), ["Show S02E13.mkv", "Show S02E14.mkv"]);
}
