//! A past item given to a rule that has picked nothing: the rule's episode
//! offset is decided from it before it is named (`offsets::settle_one`).

use crate::test_world::World;

#[tokio::test]
async fn a_past_item_the_user_picks_first_is_named_with_the_decided_offset() {
    let s = World::bare().await;
    let place = s
        .library_of(&[(1, &["01", "02"]), (2, &["01", "02"])])
        .await;
    place.link(1, &[Some(12)]).await;
    place.link(2, &[Some(12)]).await;
    // The feed already holds the new season's first release: it is past.
    s.feed_shows(&[25]);
    s.cycle().await;
    s.advance(1_000);
    let rule = s.subscribe("Show", "Show/Season 03", 7, 1).await;
    s.cycle_later().await;
    assert!(s.torrent_names().is_empty());

    let item = s.item_containing("- 25").await;
    s.receive_for(item.id, &rule.id, "00000000-0000-4000-8000-000000000b11")
        .await
        .expect("the request ran");

    assert_eq!(s.torrent_names(), ["Show S03E01.mkv"]);
    let stored = s.stored_rule(&rule).await;
    assert_eq!((stored.episode, stored.episode_auto), (-24, true));
}
