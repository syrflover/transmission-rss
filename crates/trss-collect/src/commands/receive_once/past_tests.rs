//! What a rule leaves to the user, and what `받기` with that rule then does
//! (`receive_once::run` on a payload that names a rule). A subscription holds
//! back what the cycle recorded before it, and so does a rule that was turned
//! off or archived and is on again: the cycle leaves such an item as it is, and
//! the user's `받기` receives it with the rule. A plain rule made after the
//! item was recorded takes it as before. The cycle that leaves them alone is
//! the world's [`World::cycle`], which judges the feed as the worker does.

use trss_core::commands::CommandState;

use super::fixtures::*;
use crate::{
    store::{channels::RuleState, history::HistoryResult},
    test_world::World,
};

/// The title of the n-th release of the work the rules are about.
fn liar_title(n: u32) -> String {
    format!("[SubsPlease] LIAR GAME - {n} (1080p) [ABCD12{n}].mkv")
}

/// How the rule came to leave the item to the user, or not.
#[derive(Debug, Clone, Copy)]
enum Made {
    /// A subscription made after the cycle recorded the item without a rule.
    Subscription,
    /// A plain rule made after the cycle recorded the item without a rule:
    /// only subscriptions hold back the past.
    PlainRule,
    /// The rule was off when the item came, and is on again.
    Resumed,
    /// The rule was archived when the item came, and is restored.
    Restored,
    /// A subscription that waited for its title when the item came, and is
    /// given one.
    Titled,
}

#[tokio::test]
async fn a_cycle_leaves_the_past_to_the_user_and_the_user_receives_it_with_the_rule() {
    for made in [
        Made::Subscription,
        Made::PlainRule,
        Made::Resumed,
        Made::Restored,
        Made::Titled,
    ] {
        let (title25, title26) = (liar_title(25), liar_title(26));
        let s = World::with_rules(match made {
            Made::Resumed | Made::Restored => vec![rule("LIAR GAME", LIAR_DIR)],
            Made::Subscription | Made::PlainRule | Made::Titled => unrelated_rule(),
        })
        .await;
        let mut waiting = None;
        match made {
            Made::Titled => {
                // A channel read once holds something already, and the
                // subscription waits for its title from then on.
                s.feed_after_other(&[]);
                s.cycle().await;
                s.advance(1_000);
                waiting = Some(s.subscribe_waiting(LIAR_DIR, 3320).await);
            }
            Made::Resumed => {
                let rule = s.rule_of(0).await;
                s.ctx
                    .channels
                    .set_video_receiving(&rule.id, rule.version, false, s.now())
                    .await
                    .unwrap();
            }
            Made::Restored => {
                let rule = s.rule_of(0).await;
                s.ctx
                    .channels
                    .set_rule_state(&rule.id, RuleState::Archived, s.now())
                    .await
                    .unwrap();
            }
            Made::Subscription | Made::PlainRule => {}
        }
        s.feed(&[(&hash(25), &title25), (&hash(26), &title26)]);
        s.cycle().await;
        assert!(
            s.tr.torrents().is_empty(),
            "{made:?}: nothing picks them yet"
        );
        s.advance(1_000);
        let rule = match made {
            Made::Subscription => s.subscribe("LIAR GAME", LIAR_DIR, 3320, 0).await,
            Made::Titled => {
                let waiting = waiting.take().expect("the subscription that waited");
                s.ctx
                    .channels
                    .give_title(&waiting.id, waiting.version, "LIAR GAME", None, s.now())
                    .await
                    .unwrap()
            }
            Made::PlainRule => s
                .ctx
                .channels
                .create_rule(&s.channel_id, rule("LIAR GAME", LIAR_DIR))
                .await
                .unwrap(),
            Made::Resumed => {
                let rule = s.rule_of(0).await;
                s.ctx
                    .channels
                    .set_video_receiving(&rule.id, rule.version, true, s.now())
                    .await
                    .unwrap()
            }
            Made::Restored => {
                let rule = s.rule_of(0).await;
                s.ctx
                    .channels
                    .set_rule_state(&rule.id, RuleState::Active, s.now())
                    .await
                    .unwrap()
                    .unwrap()
            }
        };
        assert_eq!(
            rule.resumed_at.is_some(),
            matches!(made, Made::Resumed | Made::Restored),
            "{made:?}: turning a rule on notes the time"
        );

        // The next cycle sees both in the feed, and the rule matches them.
        s.cycle().await;

        if let Made::PlainRule = made {
            assert_eq!(s.tr.torrents().len(), 2, "{made:?}");
            for n in [25, 26] {
                let item = s.item_containing(&format!("LIAR GAME - {n}")).await;
                assert_eq!(item.result, HistoryResult::Received, "{made:?}");
                assert_eq!(item.rule_id.as_deref(), Some(rule.id.as_str()), "{made:?}");
            }
            continue;
        }
        // Creating or turning on the rule received nothing.
        assert!(s.tr.calls_of("torrent-add").is_empty(), "{made:?}");
        assert!(s.tr.torrents().is_empty(), "{made:?}");
        for n in [25, 26] {
            let item = s.item_containing(&format!("LIAR GAME - {n}")).await;
            assert_eq!(item.result, HistoryResult::NoMatch, "{made:?}");
            assert_eq!(item.rule_id, None, "{made:?}");
        }

        // The user ticked the 25th only.
        let item = s.item_containing("LIAR GAME - 25").await;
        let finished = s.receive_for(item.id, &rule.id, CMD).await.unwrap();

        assert_eq!(finished.state, CommandState::Done, "{made:?}");
        assert_eq!(finished.outcome.result, "received", "{made:?}");
        let torrents = s.tr.torrents();
        assert_eq!(torrents.len(), 1, "{made:?}");
        assert_eq!(torrents[0].hash, hash(25), "{made:?}");
        assert_eq!(
            torrents[0].download_dir,
            s.media.join(LIAR_DIR).to_str().unwrap(),
            "{made:?}"
        );
        assert_eq!(torrents[0].name, "LIAR GAME S01E25.mkv", "{made:?}");
        let received = s.item_containing("LIAR GAME - 25").await;
        assert_eq!(received.result, HistoryResult::Received, "{made:?}");
        assert_eq!(
            received.rule_id.as_deref(),
            Some(rule.id.as_str()),
            "{made:?}"
        );

        // The 26th stays unreceived, cycle after cycle.
        s.cycle().await;
        assert_eq!(s.tr.torrents().len(), 1, "{made:?}");
        assert_eq!(
            s.item_containing("LIAR GAME - 26").await.result,
            HistoryResult::NoMatch,
            "{made:?}"
        );

        // A release that appears after that is collected as usual.
        let title27 = liar_title(27);
        s.feed(&[
            (&hash(25), &title25),
            (&hash(26), &title26),
            (&hash(27), &title27),
        ]);
        s.cycle().await;
        let hashes: Vec<String> = s.tr.torrents().into_iter().map(|t| t.hash).collect();
        assert_eq!(hashes.len(), 2, "{made:?}: {hashes:?}");
        assert!(hashes.contains(&hash(27)), "{made:?}");
        assert_eq!(
            s.item_containing("LIAR GAME - 27").await.result,
            HistoryResult::Received,
            "{made:?}"
        );
        s.assert_secret_nowhere(CMD).await;
    }
}
