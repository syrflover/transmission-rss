use trss_core::Db;

use super::*;

const NOON_UTC: Millis = 1_790_942_400_000;
const MIN: i64 = 60_000;

fn store() -> AnissiaStore {
    AnissiaStore::new(Db::open_blocking(":memory:").unwrap())
}

fn line(anime_no: i64, creator: &str, episode: &str, post: &str, updated_at: Millis) -> Line {
    Line {
        anime_no,
        creator: creator.into(),
        episode: episode.into(),
        post_url: post.into(),
        updated: format!("at {updated_at}"),
        updated_at: Some(updated_at),
    }
}

#[tokio::test]
async fn a_line_is_observed_once_until_the_episode_the_post_or_the_update_changes() {
    let store = store();
    let first = line(3441, "에루샤", "3", "https://a.test/a", NOON_UTC);

    let done = store.observe(vec![first.clone()], 1000).await.unwrap();
    assert_eq!((done.added, done.unchanged), (1, 0));
    // The same line again, later: nothing new.
    let done = store.observe(vec![first.clone()], 2000).await.unwrap();
    assert_eq!((done.added, done.unchanged), (0, 1));

    // Each of the three differences is an observation.
    for next in [
        Line {
            episode: "4".into(),
            ..first.clone()
        },
        Line {
            post_url: "https://a.test/b".into(),
            ..first.clone()
        },
        Line {
            updated_at: Some(NOON_UTC + MIN),
            updated: "later".into(),
            ..first.clone()
        },
    ] {
        let store = self::store();
        store.observe(vec![first.clone()], 1000).await.unwrap();
        let done = store.observe(vec![next], 2000).await.unwrap();
        assert_eq!((done.added, done.unchanged), (1, 0));
        assert_eq!(store.candidates(3441, Vec::new()).await.unwrap().len(), 2);
    }
}

#[tokio::test]
async fn the_same_moment_written_another_way_is_no_change_but_an_unreadable_date_is_compared_as_text(
) {
    let store = store();
    let mut a = line(1, "가", "1", "https://a.test/a", NOON_UTC);
    a.updated = "2026-10-02T21:00:00".into();
    store.observe(vec![a.clone()], 1000).await.unwrap();
    let mut b = a.clone();
    b.updated = "2026-10-02 21:00:00".into();
    let done = store.observe(vec![b], 2000).await.unwrap();
    assert_eq!((done.added, done.unchanged), (0, 1));

    let mut bad = line(1, "나", "1", "https://a.test/a", 0);
    bad.updated_at = None;
    bad.updated = "soon".into();
    store.observe(vec![bad.clone()], 3000).await.unwrap();
    let done = store.observe(vec![bad.clone()], 4000).await.unwrap();
    assert_eq!((done.added, done.unchanged), (0, 1));
    bad.updated = "later".into();
    let done = store.observe(vec![bad], 5000).await.unwrap();
    assert_eq!((done.added, done.unchanged), (1, 0));
}

#[tokio::test]
async fn a_creators_lines_share_a_source_of_the_app_per_anime() {
    let store = store();
    store
        .observe(
            vec![
                line(1, "에루샤", "1", "https://a.test/1", NOON_UTC),
                line(2, "에루샤", "1", "https://a.test/2", NOON_UTC),
                line(1, "코코렛", "1", "https://a.test/3", NOON_UTC),
            ],
            1000,
        )
        .await
        .unwrap();
    store
        .observe(
            vec![line(1, "에루샤", "2", "https://a.test/4", NOON_UTC + MIN)],
            2000,
        )
        .await
        .unwrap();

    let one = store.candidates(1, Vec::new()).await.unwrap();
    assert_eq!(one.len(), 3);
    let erusha: Vec<_> = one.iter().filter(|c| c.creator == "에루샤").collect();
    assert_eq!(erusha.len(), 2);
    assert_eq!(erusha[0].source_id, erusha[1].source_id);
    let kokoret = one.iter().find(|c| c.creator == "코코렛").unwrap();
    let other_anime = &store.candidates(2, Vec::new()).await.unwrap()[0];
    assert_ne!(kokoret.source_id, erusha[0].source_id);
    assert_ne!(other_anime.source_id, erusha[0].source_id);
    // The ID is the app's: not the name and not the address.
    assert!(!erusha[0].source_id.contains("에루샤") && !erusha[0].source_id.contains("a.test"));
}

/// What a job received from `candidate`.
fn received(candidate: &Candidate) -> Received {
    Received {
        source_id: candidate.source_id.clone(),
        episode: candidate.episode.clone(),
        observation_id: candidate.id,
        post_url: candidate.post_url.clone(),
    }
}

#[tokio::test]
async fn a_candidate_revises_the_creators_episode_only_once_its_subtitle_was_received() {
    let store = store();
    store
        .observe(vec![line(1, "가", "3", "https://a.test/a", NOON_UTC)], 1000)
        .await
        .unwrap();
    store
        .observe(
            vec![line(1, "가", "4", "https://a.test/b", NOON_UTC + MIN)],
            2000,
        )
        .await
        .unwrap();
    // The creator fixes episode 4's post: only `updDt` changes.
    store
        .observe(
            vec![line(1, "가", "4", "https://a.test/b", NOON_UTC + 5 * MIN)],
            3000,
        )
        .await
        .unwrap();
    // And posts episode 4 again somewhere else.
    store
        .observe(
            vec![line(1, "가", "4", "https://a.test/c", NOON_UTC + 6 * MIN)],
            4000,
        )
        .await
        .unwrap();
    // Another creator's episode 4.
    store
        .observe(
            vec![line(1, "나", "4", "https://a.test/d", NOON_UTC + 7 * MIN)],
            5000,
        )
        .await
        .unwrap();

    let read = |received: Vec<Received>| store.candidates(1, received);
    let all = read(Vec::new()).await.unwrap();
    let by = |all: &[Candidate], post: &str, minute: i64| {
        all.iter()
            .find(|c| c.post_url.ends_with(post) && c.updated_at == Some(NOON_UTC + minute * MIN))
            .unwrap()
            .clone()
    };
    // Observed with the same episode before, but nothing received: no revision.
    assert!(all.iter().all(|c| c.revision.is_none()));
    assert_eq!(all.len(), 5);

    // Episode 4's first post was received.
    let first = by(&all, "/b", 1);
    let all = read(vec![received(&first)]).await.unwrap();
    // Its own receipt does not make it a revision, nor episode 3 or the other creator's.
    assert_eq!(by(&all, "/b", 1).revision, None);
    assert_eq!(by(&all, "/a", 0).revision, None);
    assert_eq!(by(&all, "/d", 7).revision, None);
    assert_eq!(
        by(&all, "/b", 5).revision,
        Some(Revision {
            of: Some(first.id),
            same_post: Some(true)
        })
    );
    assert_eq!(
        by(&all, "/c", 6).revision,
        Some(Revision {
            of: Some(first.id),
            same_post: Some(false)
        })
    );

    // The fix was received too: the repost revises the latest earlier receipt,
    // and the received fix is no revision of the post received after it.
    let fixed = by(&all, "/b", 5);
    let all = read(vec![received(&first), received(&fixed)])
        .await
        .unwrap();
    assert_eq!(
        by(&all, "/c", 6).revision,
        Some(Revision {
            of: Some(fixed.id),
            same_post: Some(false)
        })
    );
    let all = read(vec![received(&by(&all, "/c", 6))]).await.unwrap();
    assert_eq!(by(&all, "/b", 5).revision, None);
    // Newest update first.
    assert_eq!(all[0].post_url, "https://a.test/d");
}

#[tokio::test]
async fn another_creators_post_of_a_received_episode_is_not_a_revision() {
    let store = store();
    store
        .observe(vec![line(1, "가", "3", "https://a.test/a", NOON_UTC)], 1000)
        .await
        .unwrap();
    store
        .observe(
            vec![line(1, "나", "3", "https://a.test/b", NOON_UTC + MIN)],
            2000,
        )
        .await
        .unwrap();
    let first = store.candidates(1, Vec::new()).await.unwrap()[1].clone();
    assert_eq!(first.creator, "가");
    assert!(store
        .candidates(1, vec![received(&first)])
        .await
        .unwrap()
        .iter()
        .all(|c| c.revision.is_none()));
}

#[tokio::test]
async fn the_same_episode_written_another_way_is_a_revision_and_a_half_episode_is_not() {
    let store = store();
    store
        .observe(vec![line(1, "가", "3", "https://a.test/a", NOON_UTC)], 1000)
        .await
        .unwrap();
    store
        .observe(
            vec![
                line(1, "가", "03", "https://a.test/a", NOON_UTC + MIN),
                line(1, "가", "3.5", "https://a.test/h", NOON_UTC + MIN),
            ],
            2000,
        )
        .await
        .unwrap();
    let all = store.candidates(1, Vec::new()).await.unwrap();
    let first = all.iter().find(|c| c.episode == "3").unwrap().clone();
    let shown = store.candidates(1, vec![received(&first)]).await.unwrap();
    let of = |episode: &str| {
        shown
            .iter()
            .find(|c| c.episode == episode)
            .unwrap()
            .revision
            .clone()
    };
    assert_eq!(
        of("03"),
        Some(Revision {
            of: Some(first.id),
            same_post: Some(true)
        })
    );
    assert_eq!(of("3.5"), None);
    assert_eq!(
        ["013", "13", "13.0", "13.50", "SP"].map(episode_key),
        ["n:13", "n:13", "n:13", "n:13.5", "t:SP"]
    );
}

#[tokio::test]
async fn episodes_are_kept_as_written_and_an_unreadable_date_sorts_by_when_it_was_first_seen() {
    let store = store();
    let mut zero = line(1, "가", "0", "https://a.test/0", 0);
    zero.updated_at = None;
    zero.updated = "unknown".into();
    store.observe(vec![zero], 5 * 60 * MIN).await.unwrap();
    store
        .observe(
            vec![line(1, "나", "13.5", "https://a.test/1", 3 * 60 * MIN)],
            6 * 60 * MIN,
        )
        .await
        .unwrap();
    store
        .observe(
            vec![line(1, "다", "07", "https://a.test/2", 9 * 60 * MIN)],
            7 * 60 * MIN,
        )
        .await
        .unwrap();

    let all = store.candidates(1, Vec::new()).await.unwrap();
    let episodes: Vec<&str> = all.iter().map(|c| c.episode.as_str()).collect();
    // 9h (episode 07), seen-at 5h (the unreadable date), 3h (13.5).
    assert_eq!(episodes, ["07", "0", "13.5"]);
    let unreadable = &all[1];
    assert_eq!(unreadable.updated_at, None);
    assert_eq!(unreadable.updated, "unknown");
    assert_eq!(unreadable.first_seen_at, 5 * 60 * MIN);
}

#[tokio::test]
async fn the_reading_schedule_keeps_when_it_is_due_and_when_it_last_read_everything() {
    let store = store();
    assert_eq!(store.caption_poll().await.unwrap(), None);
    store.schedule_caption_poll(5000, None).await.unwrap();
    assert_eq!(store.caption_poll().await.unwrap(), Some((5000, None)));
    store.schedule_caption_poll(9000, Some(4000)).await.unwrap();
    // A later schedule without a reading keeps the last one.
    store.schedule_caption_poll(12_000, None).await.unwrap();
    assert_eq!(
        store.caption_poll().await.unwrap(),
        Some((12_000, Some(4000)))
    );
}

fn held(source_id: &str, episode: &str) -> Attributed {
    Attributed {
        source_id: source_id.into(),
        episode: episode.into(),
    }
}

async fn observed_of(store: &AnissiaStore, creator: &str, episode: &str) -> Candidate {
    store
        .candidates(1, Vec::new())
        .await
        .unwrap()
        .into_iter()
        .find(|c| c.creator == creator && c.episode == episode)
        .unwrap()
}

#[tokio::test]
async fn a_creator_the_user_named_before_any_line_of_it_was_observed_gets_the_source_the_line_finds(
) {
    let store = store();
    let named = store
        .source_of_creator(1, "하느".into(), 500)
        .await
        .unwrap();
    // Asked again, or for another anime's creator of that name: one source per
    // creator and anime.
    assert_eq!(
        store
            .source_of_creator(1, "하느".into(), 900)
            .await
            .unwrap(),
        named
    );
    assert_ne!(
        store
            .source_of_creator(2, "하느".into(), 900)
            .await
            .unwrap(),
        named
    );

    // The creator's line, observed later, is of the same source: it is the
    // candidate of the creator the user named.
    store
        .observe(
            vec![line(1, "하느", "5", "https://a.test/a", NOON_UTC)],
            1000,
        )
        .await
        .unwrap();
    assert_eq!(observed_of(&store, "하느", "5").await.source_id, named);
}

#[tokio::test]
async fn a_candidate_of_the_creator_of_a_subtitle_file_for_the_same_episode_is_a_revision() {
    let store = store();
    store
        .observe(
            vec![
                line(1, "하느", "5", "https://a.test/a", NOON_UTC),
                line(1, "카이란", "5", "https://a.test/b", NOON_UTC),
            ],
            1000,
        )
        .await
        .unwrap();
    let hanu = observed_of(&store, "하느", "5").await;
    let kairan = observed_of(&store, "카이란", "5").await;
    let mark = Some(Revision {
        of: None,
        same_post: None,
    });

    // 5화 of 하느 is in the library, written `05`: 하느's 5화 revises it and
    // 카이란's does not, because the file is not 카이란's.
    let files = [held(&hanu.source_id, "05"), held(&hanu.source_id, "02")];
    assert_eq!(revision_by_attribution(&hanu, 0, &files), mark);
    assert_eq!(revision_by_attribution(&kairan, 0, &files), None);

    // Another episode, a half episode, and a text that is no number are other
    // episodes.
    let other = [held(&hanu.source_id, "6"), held(&hanu.source_id, "SP")];
    assert_eq!(revision_by_attribution(&hanu, 0, &other), None);
    store
        .observe(
            vec![line(1, "하느", "5.5", "https://a.test/h", NOON_UTC + MIN)],
            2000,
        )
        .await
        .unwrap();
    let half = observed_of(&store, "하느", "5.5").await;
    assert_eq!(revision_by_attribution(&half, 0, &files), None);
    assert_eq!(
        revision_by_attribution(&half, 0, &[held(&hanu.source_id, "5.50")]),
        mark
    );
}

#[tokio::test]
async fn the_sources_mapping_moves_the_candidates_episode_to_the_seasons_before_it_is_compared() {
    let store = store();
    store
        .observe(
            vec![
                line(1, "하느", "13", "https://a.test/a", NOON_UTC),
                line(1, "하느", "0", "https://a.test/z", NOON_UTC + MIN),
            ],
            1000,
        )
        .await
        .unwrap();
    let thirteen = observed_of(&store, "하느", "13").await;
    let zero = observed_of(&store, "하느", "0").await;
    let id = thirteen.source_id.clone();
    let mark = Some(Revision {
        of: None,
        same_post: None,
    });

    // A cumulative 13 is the season's episode 1 (offset -12).
    assert_eq!(
        revision_by_attribution(&thirteen, -12, &[held(&id, "01")]),
        mark
    );
    assert_eq!(
        revision_by_attribution(&thirteen, -12, &[held(&id, "13")]),
        None
    );
    // Without the mapping the numbers are compared as they are.
    assert_eq!(
        revision_by_attribution(&thirteen, 0, &[held(&id, "13")]),
        mark
    );
    // The line registered before the first episode (`0`) is no episode of the
    // season, and neither is a mapped number that is not above `0`: the
    // subscribed creator's receipt reads them the same way.
    assert_eq!(revision_by_attribution(&zero, 0, &[held(&id, "0")]), None);
    assert_eq!(revision_by_attribution(&zero, 5, &[held(&id, "5")]), None);
    assert_eq!(
        revision_by_attribution(&zero, -12, &[held(&id, "-12")]),
        None
    );
    assert_eq!(
        revision_by_attribution(&thirteen, -13, &[held(&id, "0")]),
        None
    );
    assert_eq!(
        revision_by_attribution(&thirteen, -20, &[held(&id, "-7")]),
        None
    );
    // An episode mapped out of the season's reach matches nothing.
    assert_eq!(
        revision_by_attribution(&thirteen, i64::MAX, &[held(&id, "1")]),
        None
    );
}
