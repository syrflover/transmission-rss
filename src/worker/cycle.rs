//! One collection cycle: read the configuration, read the feeds, judge the
//! items, add the selected ones to Transmission and record everything.

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
};

use futures::{stream, StreamExt};
use tokio::{task, task::JoinSet};
use tokio_util::sync::CancellationToken;
use transmission_rpc::types::{TorrentGetField, TorrentStatus};
use url::Url;

use super::{
    feed::{self, FeedItem},
    plan::{ChannelPlan, Judgement},
};
use crate::{
    store::{
        channels::{ChannelError, ChannelStore},
        history::{HistoryResult, HistoryStore, Millis, Observation, Recorded},
        status::{ChannelReadResult, StatusStore, TransmissionCounts},
    },
    transmission::{
        self, add_item, remove_stale, rename_with_retries, AddError, AddKind, Redactor,
        RemovedTorrent, RenameMode, RenamePolicy, SessionConfig,
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
    /// The client for Transmission's requests; they time out
    /// (see [`crate::transmission::http_client`]).
    pub transmission_http: reqwest012::Client,
    pub session: SessionConfig,
    pub http: reqwest::Client,
    pub rename: RenamePolicy,
    /// Knows secrets that do not come from channels, such as credentials in
    /// the Transmission URL.
    pub redactor: Redactor,
}

impl CycleContext {
    /// A client for Transmission, with timeouts.
    fn transmission(&self) -> transmission_rpc::TransClient {
        transmission::client(self.transmission_url.clone(), &self.transmission_http)
    }
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
    /// Items whose task panicked. Their torrents may or may not be in
    /// Transmission, so the cycle removes no departed torrents.
    pub job_panics: usize,
    /// Failed adds whose request may still have reached Transmission (the
    /// connection failed or timed out rather than Transmission refusing).
    /// Like a panic, the cycle then removes no departed torrents.
    pub adds_unconfirmed: usize,
    /// Commands left `running` when the cycle started (see
    /// [`CommandsAtStart::running`]). The cycle then removes no departed torrents.
    pub commands_running: usize,
    /// The cycle stopped early because the worker is shutting down. What was
    /// done is recorded; the rest waits for the next cycle.
    pub interrupted: bool,
}

/// What the web commands looked like when the cycle started, as far as they
/// bear on removing departed torrents.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CommandsAtStart {
    /// Commands still `running`. Under the worker lock that is a command a
    /// worker died in, or stopped to retry later: it may have handed a torrent
    /// to Transmission without history learning its hash yet.
    pub running: usize,
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
    Held { hash: String, kind: AddKind },
    /// Transmission does not hold the torrent as far as the worker knows.
    /// `unconfirmed` is set when the request failed without an answer (a
    /// connection error or timeout): Transmission may have added the torrent
    /// all the same, under a hash the worker never learned.
    Failed { unconfirmed: bool },
    /// Shutdown began before the item was started; nothing was done or recorded.
    NotStarted,
}

/// Runs one cycle whose records are stamped `at`. `commands` is what the
/// caller found among the web commands before the cycle started.
///
/// The channels and rules are read once, first; edits made while the cycle
/// runs apply from the next cycle. `cancel` asks the cycle to wind down: no
/// new item is started, items already handed to Transmission are recorded,
/// and the removal of departed torrents is skipped because it would judge
/// them against an incomplete picture.
pub async fn run_cycle(
    ctx: &CycleContext,
    at: Millis,
    commands: CommandsAtStart,
    cancel: &CancellationToken,
) -> Result<CycleReport, CycleError> {
    let mut report = CycleReport {
        commands_running: commands.running,
        ..CycleReport::default()
    };

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
    // Channels whose feed was not read, for the cleanup of departed torrents.
    let mut unread_channels: Vec<String> = Vec::new();
    // Which feeds could be read, for the status board.
    let mut reads: Vec<ChannelReadResult> = Vec::new();
    // `(channel_id, identity_key)` of every item in a feed read this cycle.
    let mut present: Vec<(String, String)> = Vec::new();

    for (plan, read) in plans.iter().zip(fetched) {
        let channel = &plan.channel;
        let label = channel.masked_url();

        let feed = match read {
            Some(Ok(feed)) => {
                println!("Parsed {label}");
                report.channels_read += 1;
                reads.push(ChannelReadResult {
                    channel_id: channel.id.clone(),
                    ok: true,
                });
                feed
            }
            Some(Err(reason)) => {
                println!("Failed {label}: {reason}");
                report.channels_failed += 1;
                unread_channels.push(channel.id.clone());
                reads.push(ChannelReadResult {
                    channel_id: channel.id.clone(),
                    ok: false,
                });
                continue;
            }
            None => {
                unread_channels.push(channel.id.clone());
                continue;
            }
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
            present.push((channel.id.clone(), identity_key.clone()));

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

    record_reads(ctx, at, &plans, reads).await;

    if cancel.is_cancelled() {
        report.interrupted = true;
        return Ok(report);
    }

    // Add the selected items.
    let Added {
        kept,
        panicked,
        unconfirmed,
    } = add_jobs(ctx, jobs, at, &redactor, cancel, &mut report).await;
    report.job_panics = panicked;
    report.adds_unconfirmed = unconfirmed;

    if cancel.is_cancelled() {
        report.interrupted = true;
        return Ok(report);
    }

    // Remove bot-labelled torrents whose item is no longer in any feed. A torrent
    // can only be called departed by a feed that was read, so:
    //
    // - With no feed read at all there is nothing to judge that against (a
    //   channel-less database, or every feed down): nothing is removed.
    // - A torrent that history records as coming from a channel whose feed was
    //   not read this cycle stays: that feed may have dropped it, or may just
    //   have been down.
    // - A torrent with no history link (for example one the legacy cron added
    //   before the switch) has no channel to wait for and follows the plain rule.
    // - A torrent that history records as received (or found already there) for
    //   an item that is still in a feed read this cycle stays, though no rule
    //   selected it now. That is
    //   how a torrent received by hand (a receive-once command) survives: it
    //   was never selected by a rule.
    //
    // Nor is anything removed when an item's task ended abnormally: it may have
    // handed a torrent to Transmission before it failed, and the cycle would not
    // know that torrent's hash, so it would look like a departed one. Recording
    // the hashes of accepted torrents as they come would not cover a failure
    // before the hash is known, so the whole removal waits for the next cycle,
    // which meets the torrent again and keeps it. An add that timed out or lost
    // its connection is the same case: Transmission may have finished it.
    //
    // A command left `running` is that case for a receive-once: its worker may
    // have died after Transmission took the torrent and before the hash was
    // written. A restarted worker runs its cycle before it looks for commands,
    // so the removal waits until the rerun has met the torrent and recorded it.
    if panicked > 0 {
        println!(
            "{panicked} item(s) ended with an internal error; \
             leaving Transmission's torrents alone this cycle"
        );
    } else if unconfirmed > 0 {
        println!(
            "{unconfirmed} add(s) got no answer from Transmission; \
             leaving Transmission's torrents alone this cycle"
        );
    } else if commands.running > 0 {
        println!(
            "{} command(s) were left running by an earlier worker; \
             leaving Transmission's torrents alone this cycle",
            commands.running
        );
    } else if report.channels_read > 0 {
        let mut kept = kept;
        let recorded = async {
            let mut hashes = ctx
                .history
                .torrent_hashes_of_channels(unread_channels)
                .await?;
            hashes.extend(ctx.history.held_hashes_of_items(present).await?);
            Ok::<_, crate::store::history::HistoryError>(hashes)
        }
        .await;
        match recorded {
            Ok(hashes) => {
                kept.extend(hashes);
                let mut transmission = ctx.transmission();
                report.removed =
                    remove_stale(&mut transmission, |hash| kept.contains(hash), &redactor).await;
            }
            Err(err) => eprintln!(
                "Cannot tell which torrents came from unread channels ({err}); \
                 leaving Transmission's torrents alone this cycle"
            ),
        }
    } else if report.channels > 0 {
        println!("No feed could be read; leaving Transmission's torrents alone");
    }

    record_transmission_counts(ctx, at, &redactor).await;

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

/// What the item tasks of a cycle came to.
struct Added {
    /// Hashes of the torrents Transmission holds for the selected items.
    kept: HashSet<String>,
    /// Tasks that ended in a panic instead of an outcome.
    panicked: usize,
    /// Failed adds that got no answer from Transmission.
    unconfirmed: usize,
}

/// What is needed to record a task's item as failed when the task panics.
struct Fallback {
    observation: Observation,
    title: String,
    channel_label: String,
}

/// Adds the selected items, up to [`ADD_CONCURRENCY`] at a time, and tallies
/// the outcomes into `report`.
///
/// Each item runs in its own task so that a panic while handling it cannot take
/// the worker down: the item is recorded as failed and counted in
/// [`Added::panicked`]. The tasks belong to a [`JoinSet`] owned by this call,
/// so when the cycle is dropped (its task panicked or was aborted) they are
/// aborted with it instead of carrying on with Transmission after the worker
/// lock has been released.
async fn add_jobs(
    ctx: &CycleContext,
    jobs: Vec<Job>,
    at: Millis,
    redactor: &Redactor,
    cancel: &CancellationToken,
    report: &mut CycleReport,
) -> Added {
    let mut tasks: JoinSet<(JobOutcome, bool)> = JoinSet::new();
    let mut fallbacks: HashMap<task::Id, Fallback> = HashMap::new();
    let mut waiting = jobs.into_iter();
    let mut added = Added {
        kept: HashSet::new(),
        panicked: 0,
        unconfirmed: 0,
    };

    loop {
        while tasks.len() < ADD_CONCURRENCY {
            let Some(job) = waiting.next() else { break };
            let fallback = Fallback {
                observation: job.observation.clone(),
                title: job.title.clone(),
                channel_label: job.channel_label.clone(),
            };
            let handle = tasks.spawn(process_job(
                ctx.clone(),
                job,
                at,
                redactor.clone(),
                cancel.clone(),
            ));
            fallbacks.insert(handle.id(), fallback);
        }

        let Some(joined) = tasks.join_next_with_id().await else {
            break;
        };

        let (outcome, was_new) = match joined {
            Ok((id, done)) => {
                fallbacks.remove(&id);
                done
            }
            Err(err) => {
                added.panicked += 1;
                let Some(fallback) = fallbacks.remove(&err.id()) else {
                    continue;
                };
                eprintln!(
                    "Internal error while adding {} ({})",
                    fallback.title, fallback.channel_label
                );
                let observation = Observation {
                    result: HistoryResult::AddFailed,
                    reason: Some("internal error while adding the item".to_owned()),
                    ..fallback.observation
                };
                if let Err(err) = ctx.history.record(at, vec![observation]).await {
                    eprintln!(
                        "Cannot record history for {}: {err}",
                        fallback.channel_label
                    );
                }
                (JobOutcome::Failed { unconfirmed: false }, false)
            }
        };

        report.items_new += usize::from(was_new);
        match outcome {
            JobOutcome::Held { hash, kind } => {
                match kind {
                    AddKind::Added => report.added += 1,
                    AddKind::Duplicate => report.duplicates += 1,
                }
                added.kept.insert(hash);
            }
            JobOutcome::Failed { unconfirmed } => {
                report.add_failed += 1;
                added.unconfirmed += usize::from(unconfirmed);
            }
            JobOutcome::NotStarted => {}
        }
    }

    added
}

/// Applies the session settings. Transmission being down is not fatal here:
/// the items it would have taken are recorded as failed, and the next cycle
/// tries again.
async fn apply_session(ctx: &CycleContext, redactor: &Redactor) {
    let mut transmission = ctx.transmission();
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

/// Adds one item, records the result, then renames the torrent (see
/// [`rename_mode`]). The second value tells whether the item had no record
/// before.
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

    let mut transmission = ctx.transmission();

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
                JobOutcome::Failed {
                    unconfirmed: matches!(err, AddError::Rpc(_)),
                },
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
        if let Some(mode) = rename_mode(&ctx, torrent.kind, &torrent.hash).await {
            rename_with_retries(
                &mut transmission,
                &torrent.hash,
                &job.save_path,
                job.episode,
                mode,
                ctx.rename,
                &redactor,
                &cancel,
            )
            .await;
        }
    }

    (outcome, was_new)
}

/// How the rule path may rename a torrent Transmission holds for an item,
/// or `None` to leave it alone.
///
/// - A torrent this cycle has just added gets the legacy treatment, including
///   removal with its data when `trname` has no name for it.
/// - A torrent that was there already is never removed, and is renamed only
///   while its name is not in the `trname` form ([`RenameMode::Existing`]),
///   which finishes a rename an earlier run did not get to.
/// - A torrent that history records as received by hand is not touched at
///   all: a person chose its folder, and the rule's folder may say otherwise.
async fn rename_mode(ctx: &CycleContext, kind: AddKind, hash: &str) -> Option<RenameMode> {
    match kind {
        AddKind::Added => Some(RenameMode::Added),
        AddKind::Duplicate => match ctx.history.received_by_hand(hash).await {
            Ok(false) => Some(RenameMode::Existing),
            Ok(true) => None,
            Err(err) => {
                eprintln!("Cannot tell whether torrent {hash} was received by hand ({err}); leaving its name alone");
                None
            }
        },
    }
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

/// Leaves this cycle's feed reads in the database for the status board. The
/// board is informational, so a failure to write is printed and the cycle goes on.
async fn record_reads(
    ctx: &CycleContext,
    at: Millis,
    plans: &[ChannelPlan],
    reads: Vec<ChannelReadResult>,
) {
    let existing = plans.iter().map(|plan| plan.channel.id.clone()).collect();
    let status = StatusStore::new(ctx.channels.db().clone());
    if let Err(err) = status.record_reads(at, reads, existing).await {
        eprintln!("Cannot record the feed read status: {err}");
    }
}

/// Leaves Transmission's downloading and seeding counts (queued torrents count
/// with their kind) for the status board. If Transmission cannot be asked, the
/// previous counts and their time stay as they were, and the cycle goes on.
async fn record_transmission_counts(ctx: &CycleContext, at: Millis, redactor: &Redactor) {
    let mut transmission = ctx.transmission();
    let torrents = match transmission
        .torrent_get(Some(vec![TorrentGetField::Status]), None)
        .await
    {
        Ok(response) => response.arguments.torrents,
        Err(err) => {
            eprintln!(
                "Cannot count the torrents in Transmission: {}",
                redactor.apply(&err.to_string())
            );
            return;
        }
    };

    let count = |kinds: [TorrentStatus; 2]| {
        torrents
            .iter()
            .filter(|torrent| torrent.status.is_some_and(|status| kinds.contains(&status)))
            .count() as u32
    };
    let counts = TransmissionCounts {
        downloading: count([TorrentStatus::Downloading, TorrentStatus::QueuedToDownload]),
        seeding: count([TorrentStatus::Seeding, TorrentStatus::QueuedToSeed]),
        taken_at: at,
    };
    let status = StatusStore::new(ctx.channels.db().clone());
    if let Err(err) = status.record_transmission(counts).await {
        eprintln!("Cannot record the Transmission counts: {err}");
    }
}
