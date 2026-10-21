use tokio::sync::broadcast::error::{RecvError, TryRecvError};

use super::*;

#[tokio::test]
async fn an_announced_change_reaches_every_subscriber() {
    let changes = Changes::default();
    let mut first = changes.subscribe();
    let mut second = changes.clone().subscribe();

    changes.announce(ChangeKind::Command, "c1");

    let expected = Change {
        kind: ChangeKind::Command,
        id: "c1".to_owned(),
    };
    assert_eq!(first.recv().await.unwrap(), expected);
    assert_eq!(second.recv().await.unwrap(), expected);
}

#[test]
fn announcing_with_no_listener_is_no_failure_and_a_late_listener_hears_only_what_follows() {
    let changes = Changes::default();
    changes.announce(ChangeKind::PastSearch, "before");

    let mut late = changes.subscribe();
    assert!(matches!(late.try_recv(), Err(TryRecvError::Empty)));

    changes.announce(ChangeKind::PastSearch, "after");
    assert_eq!(late.try_recv().unwrap().id, "after");
}

#[tokio::test]
async fn a_listener_that_falls_behind_is_told_how_many_it_lost() {
    let changes = Changes::with_capacity(2);
    let mut listener = changes.subscribe();

    for n in 0..5 {
        changes.announce(ChangeKind::Command, format!("c{n}"));
    }

    assert!(matches!(listener.recv().await, Err(RecvError::Lagged(3))));
    // What the channel kept is still heard, in order.
    assert_eq!(listener.recv().await.unwrap().id, "c3");
    assert_eq!(listener.recv().await.unwrap().id, "c4");
}

#[test]
fn a_change_is_sent_as_its_kind_in_snake_case_and_its_id() {
    let change = Change {
        kind: ChangeKind::PastSearch,
        id: "s1".to_owned(),
    };
    assert_eq!(
        serde_json::to_string(&change).unwrap(),
        r#"{"kind":"past_search","id":"s1"}"#
    );
}
