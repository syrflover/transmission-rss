//! One collection cycle: read the configuration, read the feeds, judge the
//! items, add the selected ones to Transmission and record everything.

use std::{collections::HashSet, path::PathBuf};

use futures::{stream, StreamExt};
use tokio_util::sync::CancellationToken;
use transmission_rpc::TransClient;
use url::Url;

use super::{
    feed::{self, FeedItem},
    plan::{ChannelPlan, Judgement},
};
use crate::{
    store::{
        channels::{ChannelError, ChannelStore},
        history::{HistoryResult, HistoryStore, Millis, Observation, Recorded},
    },
    transmission::{
        add_item, remove_stale, rename_with_retries, AddError, AddKind, Redactor, RemovedTorrent,
        RenamePolicy, SessionConfig,
    },
};

/// How many selected items are added to Transmission at the same time.
const ADD_CONCURRENCY: usize = 100;
/// How many feeds are read at the same time.
const FETCH_CONCURRENCY: usize = 5;
/// Longest failure reason kept in history, in characters.
const MAX_REASON_CHARS: usize = 300;

/// What a cycle needs. Cheap to clone.
#[derive(Clone)]
pub struct CycleContext {
    pub channels: ChannelStore,
    pub history: HistoryStore,
    pub transmission_url: Url,
    pub session: SessionConfig,
    pub http: reqwest::Client,
    pub rename: RenamePolicy,
    /// Knows secrets that do not come from channels, such as credentials in
    /// the Transmission URL.
    pub redactor: Redactor,
}

#[derive(Debug, thiserror::Error)]
pub enum CycleError {
    #[error("cannot read the channels: {0}")]
    Channels(#[from] ChannelError),
}

/// What one cycle did, for logs and tests. The counts are about this cycle's
/// own outcomes (an already-received item that Transmission reports as present
/// again counts as `duplicate`, whatever its stored result stays).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CycleReport {
    pub channels: usize,
    pub channels_read: usize,
    pub channels_failed: usize,
    pub items_seen: usize,
    /// Items that had no record before this cycle.
    pub items_new: usize,
    pub added: usize,
    pub duplicates: usize,
    pub add_failed: usize,
    pub no_match: usize,
    pub excluded: usize,
    /// Torrents taken out of Transmission because they left the feeds.
    pub removed: Vec<RemovedTorrent>,
    /// The cycle stopped early because the worker is shutting down. What was
    /// done is recorded; the rest waits for the next cycle.
    pub interrupted: bool,
}

/// A selected item on its way to Transmission.
struct Job {
    /// The observation to record; `result`, `torrent_hash` and `reason` are
    /// filled in from what Transmission says.
    observation: Observation,
    /// The title with the channel's secret values replaced, for logs.
    title: String,
    /// The item's own link, which Transmission is asked to add.
    link: String,
    save_path: PathBuf,
    episode: isize,
    channel_label: String,
}

/// What happened to a [`Job`].
enum JobOutcome {
    /// Transmission holds the torrent (added now or already there).
    Held {
        hash: String,
        kind: AddKind,
    },
    Failed,
    /// Shutdown began before the item was started; nothing was done or recorded.
    NotStarted,
}

/// Runs one cycle whose records are stamped `at`.
///
/// The channels and rules are read once, first; edits made while the cycle
/// runs apply from the next cycle. `cancel` asks the cycle to wind down: no
/// new item is started, items already handed to Transmission are recorded,
/// and the removal of departed torrents is skipped because it would judge
/// them against an incomplete picture.
pub async fn run_cycle(
    ctx: &CycleContext,
    at: Millis,
    cancel: &CancellationToken,
) -> Result<CycleReport, CycleError> {
    let mut report = CycleReport::default();

    // One consistent snapshot, before anything else.
    let snapshot = ctx.channels.list_channels_with_rules().await?;
    let plans: Vec<ChannelPlan> = snapshot.into_iter().map(ChannelPlan::new).collect();
    report.channels = plans.len();

    let mut redactor = ctx.redactor.clone();
    for plan in &plans {
        redactor.extend(&plan.redactor());
        for problem in plan.rule_problems() {
            eprintln!("{}", redactor.apply(&problem));
        }
    }

    println!("Cycle started: {} channel(s)", plans.len());

    apply_session(ctx, &redactor).await;

    // Read the feeds.
    let requests: Vec<(String, Redactor)> = plans
        .iter()
        .map(|plan| (plan.channel.url.clone(), plan.redactor()))
        .collect();
    let fetched = stream::iter(requests)
        .map(|(url, redactor)| {
            let http = ctx.http.clone();
            let cancel = cancel.clone();
            async move {
                let read = tokio::select! {
                    read = feed::fetch(&http, &url) => read,
                    _ = cancel.cancelled() => return None,
                };
                Some(read.map_err(|err| redactor.apply(&err.to_string())))
            }
        })
        .buffered(FETCH_CONCURRENCY)
        .collect::<Vec<_>>()
        .await;

    if cancel.is_cancelled() {
        report.interrupted = true;
        return Ok(report);
    }

    // Judge the items; record what needs no Transmission and collect the rest.
    let mut jobs: Vec<Job> = Vec::new();

    for (plan, read) in plans.iter().zip(fetched) {
        let channel = &plan.channel;
        let label = channel.masked_url();

        let feed = match read {
            Some(Ok(feed)) => {
                println!("Parsed {label}");
                report.channels_read += 1;
                feed
            }
            Some(Err(reason)) => {
                println!("Failed {label}: {reason}");
                report.channels_failed += 1;
                continue;
            }
            None => continue,
        };

        let mut skipped = Vec::new();

        // What goes into history and logs passes through the channel's redactor.
        let channel_redactor = plan.redactor();

        for item in feed::items(&feed, &channel.secret_query, &channel_redactor) {
            report.items_seen += 1;
            let FeedItem {
                identity_key,
                title,
                stored_title,
                link,
                stored_link,
            } = item;

            let observation = Observation {
                channel_id: channel.id.clone(),
                channel_label: label.clone(),
                identity_key,
                title: stored_title.clone(),
                link: stored_link,
                result: HistoryResult::NoMatch,
                rule_id: None,
                torrent_hash: None,
                reason: None,
            };

            match plan.judge(&title) {
                Judgement::Selected {
                    rule_id,
                    save_path,
                    episode,
                } => jobs.push(Job {
                    observation: Observation {
                        rule_id: Some(rule_id),
                        ..observation
                    },
                    title: stored_title,
                    link,
                    save_path,
                    episode,
                    channel_label: label.clone(),
                }),
                Judgement::Excluded => {
                    report.excluded += 1;
                    skipped.push(Observation {
                        result: HistoryResult::Excluded,
                        ..observation
                    });
                }
                Judgement::NoMatch => {
                    report.no_match += 1;
                    skipped.push(observation);
                }
            }
        }

        match ctx.history.record(at, skipped).await {
            Ok(recorded) => {
                report.items_new += recorded.iter().filter(|r| **r == Recorded::New).count();
            }
            Err(err) => eprintln!("Cannot record history for {label}: {err}"),
        }
    }

    if cancel.is_cancelled() {
        report.interrupted = true;
        return Ok(report);
    }

    // Add the selected items. Each runs in its own task so that a panic while
    // handling one item cannot take the worker down; it is recorded as a failure.
    let outcomes = stream::iter(jobs)
        .map(|job| {
            let ctx = ctx.clone();
            let redactor = redactor.clone();
            let cancel = cancel.clone();
            async move {
                let fallback = job.observation.clone();
                let label = job.channel_label.clone();
                let title = job.title.clone();
                let handle = tokio::spawn(process_job(ctx.clone(), job, at, redactor, cancel));

                match handle.await {
                    Ok(outcome) => outcome,
                    Err(_) => {
                        eprintln!("Internal error while adding {title} ({label})");
                        let observation = Observation {
                            result: HistoryResult::AddFailed,
                            reason: Some("internal error while adding the item".to_owned()),
                            ..fallback
                        };
                        if let Err(err) = ctx.history.record(at, vec![observation]).await {
                            eprintln!("Cannot record history for {label}: {err}");
                        }
                        (JobOutcome::Failed, false)
                    }
                }
            }
        })
        .buffer_unordered(ADD_CONCURRENCY)
        .collect::<Vec<_>>()
        .await;

    let mut kept: HashSet<String> = HashSet::new();
    for (outcome, was_new) in outcomes {
        report.items_new += usize::from(was_new);
        match outcome {
            JobOutcome::Held { hash, kind } => {
                match kind {
                    AddKind::Added => report.added += 1,
                    AddKind::Duplicate => report.duplicates += 1,
                }
                kept.insert(hash);
            }
            JobOutcome::Failed => report.add_failed += 1,
            JobOutcome::NotStarted => {}
        }
    }

    if cancel.is_cancelled() {
        report.interrupted = true;
        return Ok(report);
    }

    // Remove bot-labelled torrents whose item is no longer in any feed. With no
    // feed read at all there is nothing to judge that against (a channel-less
    // database, or every feed down), so nothing is removed.
    if report.channels_read > 0 {
        let mut transmission = TransClient::new(ctx.transmission_url.clone());
        report.removed =
            remove_stale(&mut transmission, |hash| kept.contains(hash), &redactor).await;
    } else if report.channels > 0 {
        println!("No feed could be read; leaving Transmission's torrents alone");
    }

    println!(
        "Cycle finished: {} item(s) seen ({} new), {} added, {} already in Transmission, \
         {} failed, {} without a rule, {} excluded, {} removed",
        report.items_seen,
        report.items_new,
        report.added,
        report.duplicates,
        report.add_failed,
        report.no_match,
        report.excluded,
        report.removed.len()
    );

    Ok(report)
}

/// Applies the session settings. Transmission being down is not fatal here:
/// the items it would have taken are recorded as failed, and the next cycle
/// tries again.
async fn apply_session(ctx: &CycleContext, redactor: &Redactor) {
    let mut transmission = TransClient::new(ctx.transmission_url.clone());
    let args = ctx.session.to_args();
    println!("Applying Transmission settings: {args:?}");

    let result = transmission.session_set(args).await;
    if let Err(err) = result {
        eprintln!(
            "Cannot set the Transmission configuration: {}",
            redactor.apply(&err.to_string())
        );
    }
}

/// Adds one item, records the result, then renames the torrent. The second
/// value tells whether the item had no record before.
async fn process_job(
    ctx: CycleContext,
    job: Job,
    at: Millis,
    redactor: Redactor,
    cancel: CancellationToken,
) -> (JobOutcome, bool) {
    if cancel.is_cancelled() {
        return (JobOutcome::NotStarted, false);
    }

    let mut transmission = TransClient::new(ctx.transmission_url.clone());

    let added = add_item(&mut transmission, &job.link, &job.save_path, &redactor).await;

    let (observation, outcome) = match &added {
        Ok(torrent) => {
            let result = match torrent.kind {
                AddKind::Added => HistoryResult::Received,
                AddKind::Duplicate => HistoryResult::Duplicate,
            };
            (
                Observation {
                    result,
                    torrent_hash: Some(torrent.hash.clone()),
                    ..job.observation
                },
                JobOutcome::Held {
                    hash: torrent.hash.clone(),
                    kind: torrent.kind,
                },
            )
        }
        Err(err) => {
            let reason = failure_reason(err, &redactor);
            eprintln!("Cannot add {} ({}): {reason}", job.title, job.channel_label);
            (
                Observation {
                    result: HistoryResult::AddFailed,
                    reason: Some(reason),
                    ..job.observation
                },
                JobOutcome::Failed,
            )
        }
    };

    // Recorded as soon as Transmission has answered, before the renaming
    // that can take many seconds.
    let was_new = match ctx.history.record(at, vec![observation]).await {
        Ok(recorded) => recorded.first() == Some(&Recorded::New),
        Err(err) => {
            eprintln!("Cannot record history for {}: {err}", job.channel_label);
            false
        }
    };

    if let Ok(torrent) = &added {
        rename_with_retries(
            &mut transmission,
            &torrent.hash,
            &job.save_path,
            job.episode,
            ctx.rename,
            &redactor,
            &cancel,
        )
        .await;
    }

    (outcome, was_new)
}

fn failure_reason(err: &AddError, redactor: &Redactor) -> String {
    let text = match err {
        AddError::Rpc(err) => format!("Transmission unreachable or failed: {err}"),
        AddError::Rejected(result) => format!("Transmission refused the torrent: {result}"),
    };
    redactor
        .apply(&text)
        .chars()
        .take(MAX_REASON_CHARS)
        .collect()
}
