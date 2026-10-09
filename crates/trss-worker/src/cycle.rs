//! One collection cycle: read the configuration, read the feeds, judge the
//! items, add the selected ones to Transmission and record everything.
//!
//! What the cycle decides about each item is `trss_collect::cycle`'s; this is
//! its order, its report, its item tasks and the gate of its removal of
//! departed torrents.

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::Arc,
};

use futures::{stream, StreamExt};
use tokio::{task, task::JoinSet};
use tokio_util::sync::CancellationToken;
use transmission_rpc::types::TorrentGetField;

use trss_collect::{
    context::CollectContext,
    cycle::{self, Fallback, Job, JobOutcome},
    feed,
    plan::ChannelPlan,
    revisions::{self, Listing},
    store::{
        channels::ChannelError,
        status::{ChannelReadResult, StatusStore, TransmissionLook},
    },
};
use trss_core::{settings::SettingsError, Millis};
use trss_transmission::{AddKind, Redactor, RemovedTorrent, SessionConfig};

use crate::removal::Removal;

/// How many selected items are added to Transmission at the same time.
const ADD_CONCURRENCY: usize = 100;
/// How many feeds are read at the same time.
const FETCH_CONCURRENCY: usize = 5;

#[derive(Debug, thiserror::Error)]
pub enum CycleError {
    #[error("cannot read the channels: {0}")]
    Channels(#[from] ChannelError),
    #[error("cannot read the collection settings: {0}")]
    Settings(#[from] SettingsError),
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
    /// Items a rule selected that were left alone because no collect folder is
    /// set. They are not recorded at all, so the next cycle judges them again.
    pub waiting_for_collect_folder: usize,
    /// Items a rule selected that were left alone because, by their turn at
    /// the work folder, the rule was no longer active or had another episode
    /// conversion (a command archived it or undid its offset meanwhile). They
    /// are not recorded at all, so the next cycle judges them again.
    pub rule_changed: usize,
    /// Items a rule selected that were left alone because the archive folder
    /// holds the rule's work folder: nothing is received into a work that is
    /// split across the two folders ([`move_before_receiving`]). They are not
    /// recorded at all, so the next cycle judges them again.
    pub waiting_for_move: usize,
    /// The `start` commands the cycle stored for those rules, each pausing its
    /// rule until the folder is in the collect folder.
    pub moves_asked: usize,
    /// Torrents taken out of Transmission because they left the feeds.
    pub removed: Vec<RemovedTorrent>,
    /// Items whose task panicked. Their torrents may or may not be in
    /// Transmission, so the cycle removes no departed torrents.
    pub job_panics: usize,
    /// Failed adds whose request may still have reached Transmission (the
    /// connection failed or timed out rather than Transmission refusing).
    /// Like a panic, the cycle then removes no departed torrents.
    pub adds_unconfirmed: usize,
    /// Commands an earlier start left `running` when the removal of departed
    /// torrents was about to run (see [`CommandsAtRemoval::running`]); the
    /// cycle then removes none. Zero when the cycle stopped before that point.
    pub commands_running: usize,
    /// Commands with an unconfirmed add at that point (see
    /// [`CommandsAtRemoval::unconfirmed_adds`]); the cycle then removes no
    /// departed torrents.
    pub commands_unconfirmed: usize,
    /// Commands were adding, moving or renaming torrents when the removal of
    /// departed torrents was due (they held the torrent gate, see
    /// [`crate::removal`]), so the cycle removed none; the next cycle tries
    /// again.
    pub commands_at_work: bool,
    /// Selected revisions not added because the folder holds the episode and
    /// the worker could not tell the revisions apart (`버전 미상`), or holds
    /// this revision already.
    pub revisions_withheld: usize,
    /// Selected items left to a video revision replacement this cycle (see
    /// [`trss_collect::revisions`]), or whose decision had to wait.
    pub revisions_left: usize,
    /// The cycle stopped early because the worker is shutting down. What was
    /// done is recorded; the rest waits for the next cycle.
    pub interrupted: bool,
}

pub use crate::removal::CommandsAtRemoval;

/// Runs one cycle whose records are stamped `at`. `removal` is what the
/// removal of departed torrents waits for and asks about the web commands
/// (see [`crate::removal`]).
///
/// The channels and rules are read once, first; edits made while the cycle
/// runs apply from the next cycle, except that an item whose rule was archived,
/// paused or deleted, or got another episode conversion, by the time the item
/// gets its turn at the work folder is left for the next cycle. `cancel` asks
/// the cycle to wind down: no new item is started, items already handed to
/// Transmission are recorded, and the removal of departed torrents is skipped
/// because it would judge them against an incomplete picture.
///
/// Web commands run beside the cycle. Each item takes its turn at its work
/// folder ([`trss_core::folder_locks`]) for its add and rename, so a command
/// that moves or renames in the folder runs before or after it; the removal
/// takes the torrent gate ([`crate::removal`]), or is left for the next cycle
/// when commands hold it.
pub async fn run_cycle(
    ctx: &CollectContext,
    session: &SessionConfig,
    at: Millis,
    removal: &Removal,
    cancel: &CancellationToken,
) -> Result<CycleReport, CycleError> {
    let mut report = CycleReport::default();

    // One consistent snapshot, before anything else.
    let snapshot = ctx.channels.list_channels_with_rules().await?;
    let collect_folder: Option<PathBuf> = ctx
        .settings
        .collection()
        .await?
        .map(|settings| PathBuf::from(settings.folder));
    // The rules whose episode offset the app may still set, before the
    // snapshot goes into the plans.
    let open_rules = cycle::open_rules(&snapshot);
    // Without a collect folder the items are still judged (and the ones no rule
    // takes recorded), but nothing is added (see `cycle::judge_feed`).
    let plans = cycle::make_plans(
        ctx,
        snapshot,
        collect_folder.as_deref().unwrap_or(Path::new("")),
    )
    .await;
    report.channels = plans.len();

    let mut redactor = ctx.redactor.clone();
    for plan in &plans {
        redactor.extend(&plan.redactor());
        for problem in plan.rule_problems() {
            eprintln!("{}", redactor.apply(&problem));
        }
    }

    println!("Cycle started: {} channel(s)", plans.len());

    apply_session(ctx, session, &redactor).await;

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

        let judged = cycle::judge_feed(ctx, plan, &feed, collect_folder.as_deref(), at).await;
        report.items_seen += judged.items_seen;
        report.items_new += judged.items_new;
        report.no_match += judged.no_match;
        report.excluded += judged.excluded;
        report.waiting_for_collect_folder += judged.waiting_for_collect_folder;
        jobs.extend(judged.jobs);
        present.extend(judged.present);
    }

    record_reads(ctx, at, &plans, reads).await;

    if cancel.is_cancelled() {
        report.interrupted = true;
        return Ok(report);
    }

    // Items a revision replacement has decided about are left to it.
    let (mut jobs, left) = cycle::leave_revisions(ctx, jobs, at).await;
    report.revisions_withheld += left.withheld;
    report.revisions_left += left.left;
    report.items_new += left.items_new;

    // A new season's rule gets its episode offset before its first item is
    // named, so that item is named with it.
    if let Some(folder) = collect_folder.as_deref().and_then(Path::to_str) {
        cycle::settle_offsets(ctx, folder, &open_rules, &mut jobs).await;
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

    // Carry the replacements of video revisions on.
    revisions::advance(&ctx.revision_work(), &ctx.folders, at, &redactor, cancel).await;

    if cancel.is_cancelled() {
        report.interrupted = true;
        return Ok(report);
    }

    // Remove finished bot-labelled torrents whose item is no longer in any feed
    // (one still downloading waits until it has finished). A torrent can only be
    // called departed by a feed that was read, so with no feed read at all there
    // is nothing to judge that against (a channel-less database, or every feed
    // down): nothing is removed. Which torrents stay when some feeds were read
    // is `cycle::remove_departed`'s.
    //
    // Nor is anything removed when an item's task ended abnormally: it may have
    // handed a torrent to Transmission before it failed, and the cycle would not
    // know that torrent's hash, so it would look like a departed one. Recording
    // the hashes of accepted torrents as they come would not cover a failure
    // before the hash is known, so the whole removal waits for the next cycle,
    // which meets the torrent again and keeps it. An add that timed out or lost
    // its connection is the same case: Transmission may have finished it.
    //
    // A command left `running` by an earlier start is that case for a retry:
    // its worker may have died after Transmission took the torrent and before
    // the hash was written, so the removal waits until the rerun has met the
    // torrent and recorded it. A command whose add got no answer is the
    // unconfirmed case again; that holds the removal of the first cycle after
    // it. A command running now holds the torrent gate; the removal does not
    // wait for it and is left for the next cycle (see `crate::removal`).
    let removable =
        collect_folder.is_some() && panicked == 0 && unconfirmed == 0 && report.channels_read > 0;
    // From the commands' check through the last removal, no command adds (the
    // check alone, for the report, needs no gate).
    let gate = if removable {
        removal.gate.try_removal()
    } else {
        None
    };
    report.commands_at_work = removable && gate.is_none();
    let commands = removal.commands().await;
    if let Ok(commands) = &commands {
        report.commands_running = commands.running;
        report.commands_unconfirmed = commands.unconfirmed_adds;
    }
    if collect_folder.is_none() {
        // The selected items were not recorded, so a torrent for one of them
        // (an older add, or the legacy cron's) could look like a departed one.
        println!("No collect folder is set; leaving Transmission's torrents alone");
    } else if panicked > 0 {
        println!(
            "{panicked} item(s) ended with an internal error; \
             leaving Transmission's torrents alone this cycle"
        );
    } else if unconfirmed > 0 {
        println!(
            "{unconfirmed} add(s) got no answer from Transmission; \
             leaving Transmission's torrents alone this cycle"
        );
    } else if report.commands_at_work {
        println!(
            "Command(s) are changing Transmission's torrents; \
             leaving them alone this cycle"
        );
    } else if let Err(err) = commands {
        eprintln!(
            "Cannot read the web commands ({err}); \
             leaving Transmission's torrents alone this cycle"
        );
    } else if report.commands_running > 0 {
        println!(
            "{} command(s) were left running by an earlier start; \
             leaving Transmission's torrents alone this cycle",
            report.commands_running
        );
    } else if report.commands_unconfirmed > 0 {
        println!(
            "{} command(s) got no answer from Transmission to their add; \
             leaving Transmission's torrents alone this cycle",
            report.commands_unconfirmed
        );
    } else if report.channels_read > 0 {
        report.removed =
            cycle::remove_departed(ctx, kept, present, unread_channels, &redactor).await;
    } else if report.channels > 0 {
        println!("No feed could be read; leaving Transmission's torrents alone");
    }
    drop(gate);

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
    if report.waiting_for_collect_folder > 0 {
        println!(
            "{} item(s) wait for the collect folder to be set",
            report.waiting_for_collect_folder
        );
    }

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
    ctx: &CollectContext,
    jobs: Vec<Job>,
    at: Millis,
    redactor: &Redactor,
    cancel: &CancellationToken,
    report: &mut CycleReport,
) -> Added {
    let mut tasks: JoinSet<(JobOutcome, bool)> = JoinSet::new();
    let mut fallbacks: HashMap<task::Id, Fallback> = HashMap::new();
    let mut waiting = jobs.into_iter();
    // Transmission's whole file list, read at most once for the revisions
    // among the items.
    let listing = Arc::new(Listing::new());
    let mut added = Added {
        kept: HashSet::new(),
        panicked: 0,
        unconfirmed: 0,
    };

    loop {
        while tasks.len() < ADD_CONCURRENCY {
            let Some(job) = waiting.next() else { break };
            let fallback = job.fallback();
            let handle = tasks.spawn(cycle::process_job(
                ctx.clone(),
                job,
                at,
                redactor.clone(),
                cancel.clone(),
                listing.clone(),
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
                cycle::record_panic(ctx, at, fallback).await;
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
            JobOutcome::Withheld => report.revisions_withheld += 1,
            JobOutcome::Later => report.revisions_left += 1,
            JobOutcome::RuleChanged => report.rule_changed += 1,
            JobOutcome::MovingFirst { asked } => {
                report.waiting_for_move += 1;
                report.moves_asked += usize::from(asked);
            }
        }
    }

    added
}

/// Applies the session settings. Transmission being down is not fatal here:
/// the items it would have taken are recorded as failed, and the next cycle
/// tries again.
async fn apply_session(ctx: &CollectContext, session: &SessionConfig, redactor: &Redactor) {
    let mut transmission = ctx.transmission.client();
    let args = session.to_args();
    println!("Applying Transmission settings: {args:?}");

    let result = transmission.session_set(args).await;
    if let Err(err) = result {
        eprintln!(
            "Cannot set the Transmission configuration: {}",
            redactor.apply(&err.to_string())
        );
    }
}

/// Leaves this cycle's feed reads in the database for the status board. The
/// board is informational, so a failure to write is printed and the cycle goes on.
async fn record_reads(
    ctx: &CollectContext,
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
/// with their kind) and the hashes of the torrents that are downloading for the
/// status board and the weekly schedule, and the hashes of all its torrents for
/// the past episode search. If Transmission cannot be asked, the
/// previous counts, hashes and their time stay as they were, and the cycle goes on.
async fn record_transmission_counts(ctx: &CollectContext, at: Millis, redactor: &Redactor) {
    let mut transmission = ctx.transmission.client();
    let torrents = match transmission
        .torrent_get(
            Some(vec![TorrentGetField::Status, TorrentGetField::HashString]),
            None,
        )
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

    let look = TransmissionLook::of(&torrents, at);
    let status = StatusStore::new(ctx.channels.db().clone());
    if let Err(err) = status
        .record_transmission(look.counts, look.downloading)
        .await
    {
        eprintln!("Cannot record the Transmission counts: {err}");
    }
    if let Err(err) = status.record_listing(at, look.everything).await {
        eprintln!("Cannot record the torrents in Transmission: {err}");
    }
}
