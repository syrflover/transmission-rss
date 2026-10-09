//! A past item given to a rule that has picked nothing: the rule's episode
//! offset is decided from it before it is named (`offsets::settle_one`).

use std::path::Path;

use trss_core::{
    commands::{Accepted, NewCommand},
    folder_locks::Section,
};

use crate::{
    commands::{
        receive_once::{self, ReceiveOnce},
        receive_past::{self, ReceivePast},
    },
    plan::rule_work_folder,
    store::{channels::Rule, history::HistoryItem},
    test_world::World,
};

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

/// What a receive of `item` for `rule` waits for when the item may decide the
/// rule's offset: the rule's work folder, the rule alone and the item alone.
async fn deciding_section(s: &World, rule: &Rule, item: &HistoryItem) -> Section {
    let collect = s
        .ctx
        .settings
        .collection()
        .await
        .unwrap()
        .expect("a collect folder");
    Section::new()
        .read(rule_work_folder(Path::new(&collect.folder), rule))
        .rule(&rule.id)
        .item(&item.channel_id, &item.identity_key)
}

/// The same without the rule alone.
async fn other_section(s: &World, rule: &Rule, item: &HistoryItem) -> Section {
    let collect = s
        .ctx
        .settings
        .collection()
        .await
        .unwrap()
        .expect("a collect folder");
    Section::new()
        .read(rule_work_folder(Path::new(&collect.folder), rule))
        .item(&item.channel_id, &item.identity_key)
}

#[tokio::test]
async fn only_the_receives_that_may_decide_the_offset_take_the_rule_alone() {
    let s = World::bare().await;
    s.feed_shows(&[25, 26]);
    s.cycle().await;
    s.advance(1_000);
    let rule = s.subscribe("Show", "Show/Season 03", 7, 1).await;
    s.cycle_later().await;
    let (first, second) = (
        s.item_containing("- 25").await,
        s.item_containing("- 26").await,
    );

    // The rule has picked nothing: both requests, accepted before either runs,
    // may decide it, so they go one at a time.
    let two_id = "00000000-0000-4000-8000-000000000c12";
    let one = s
        .start_receive_for(first.id, &rule.id, "00000000-0000-4000-8000-000000000c11")
        .await;
    let payload = ReceiveOnce::by_rule(second.id, &rule.id).canonical();
    s.accept_receive(two_id, &payload, second.id).await;
    let two = s.command(two_id).await;
    let ctx = s.ctx.receive();
    assert_eq!(
        receive_once::section(&ctx, &one).await.unwrap(),
        deciding_section(&s, &rule, &first).await
    );
    assert_eq!(
        receive_once::section(&ctx, &two).await.unwrap(),
        deciding_section(&s, &rule, &second).await
    );

    // Once the rule has received an item, its next receive decides nothing.
    s.run_command(&one).await.expect("the request ran");
    assert_eq!(
        receive_once::section(&ctx, &two).await.unwrap(),
        other_section(&s, &rule, &second).await
    );
}

#[tokio::test]
async fn a_past_episode_search_does_not_take_the_rule_alone() {
    let s = World::bare().await;
    s.feed_shows(&[25]);
    s.cycle().await;
    s.advance(1_000);
    let rule = s.subscribe("Show", "Show/Season 03", 7, 1).await;
    s.cycle_later().await;
    let item = s.item_containing("- 25").await;
    let payload = ReceivePast {
        rule_id: rule.id.clone(),
        key: item.identity_key.clone(),
        title: item.title.clone(),
        link: item.link.clone(),
    };
    let id = "00000000-0000-4000-8000-000000000c21";
    let accepted = s
        .ctx
        .commands
        .accept(
            NewCommand {
                id: id.to_owned(),
                kind: receive_past::KIND.to_owned(),
                payload: payload.canonical(),
                subject: Some(payload.subject()),
            },
            s.now(),
        )
        .await
        .unwrap();
    assert!(matches!(accepted, Accepted::Created(_)));
    let command = s.command(id).await;

    // The rule has picked nothing, but the search keeps the offset it has.
    assert_eq!(
        receive_past::section(&s.ctx.receive(), &command)
            .await
            .unwrap(),
        other_section(&s, &rule, &item).await
    );
}
