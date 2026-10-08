//! What a collection cycle decides, item by item: which items of a feed a rule
//! takes, which of them a video revision replacement keeps for itself, and how
//! one selected item goes to Transmission and into history. The cycle itself
//! (its order, its report, its tasks and the removal's gate) is `trss-worker`'s
//! `run_cycle`, which calls these.

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::Arc,
};

use tokio_util::sync::CancellationToken;

use crate::{
    commands::rule_archive::{move_before_receiving, Receiving},
    context::{CollectContext, ReceiveContext},
    episode_offset::may_decide,
    feed::{self, FeedItem},
    offsets,
    plan::{work_folder_of, ChannelPlan, Judgement},
    receive::{self, Original, RenameJob, RenameMode, Underivable},
    revisions::{self, Decided, Listing, Plan, Replaced, Selected},
    store::{
        channels::{ChannelWithRules, Rule, RuleState},
        history::{HistoryResult, KnownItem, Observation, Recorded},
        revisions::{HistoryWrite, Mark, NewRevision, Revision, RevisionState, RowWrite},
    },
};
use trss_core::{folder_locks::Section, Millis};
use trss_transmission as transmission;
use trss_transmission::{remove_stale, AddKind, AddLabels, Redactor, RemovedTorrent};

/// The plan of each channel. A channel with a subscription gets one more
/// look at history, for its first read: when history has no first read of it
/// yet (an item the past search left there is not one), this cycle reads it for
/// the first time and its subscriptions sit out
/// ([`ChannelPlan::for_first_read`]); otherwise the items the first read
/// recorded say for themselves that the feed held them then
/// ([`ChannelPlan::is_past`]). Channels without a subscription need neither, and
/// cost no query.
///
/// When history cannot be read for that, a channel with a subscription is
/// planned as if it were read for the first time: its subscriptions sit out
/// this cycle. Without the first read there is no telling what the feed
/// already held from what is new, and a subscription that took the whole feed
/// cannot be undone. A channel that did have a history only waits: what the
/// subscription sat out is recorded as no match after the first read, so the
/// next cycle, which can read the history, receives it.
pub async fn make_plans(
    ctx: &CollectContext,
    snapshot: Vec<ChannelWithRules>,
    collect_folder: &Path,
) -> Vec<ChannelPlan> {
    let subscribed: Vec<String> = snapshot
        .iter()
        .filter(|cwr| {
            cwr.rules
                .iter()
                .any(|rule| rule.state == RuleState::Active && rule.subscription.is_some())
        })
        .map(|cwr| cwr.channel.id.clone())
        .collect();
    let first_reads = match ctx.history.first_sightings(subscribed.clone()).await {
        Ok(found) => Some(found),
        Err(err) => {
            eprintln!(
                "Cannot read the first reads of the channels from history; \
                 their subscriptions sit this cycle out: {err}"
            );
            None
        }
    };

    snapshot
        .into_iter()
        .map(|cwr| {
            if !subscribed.contains(&cwr.channel.id) {
                return ChannelPlan::new(cwr, collect_folder);
            }
            match first_reads
                .as_ref()
                .map(|found| found.contains_key(&cwr.channel.id))
            {
                // No record yet, or none could be read: this cycle's read is
                // taken as the first.
                None | Some(false) => ChannelPlan::for_first_read(cwr, collect_folder),
                Some(true) => ChannelPlan::new(cwr, collect_folder),
            }
        })
        .collect()
}

/// The rules of `snapshot` whose episode offset the app may still set (see
/// [`offsets::settle`]), by ID: read before the snapshot goes into the plans.
pub fn open_rules(snapshot: &[ChannelWithRules]) -> HashMap<String, Rule> {
    snapshot
        .iter()
        .flat_map(|cwr| &cwr.rules)
        .filter(|rule| rule.state == RuleState::Active && may_decide(rule))
        .map(|rule| (rule.id.clone(), rule.clone()))
        .collect()
}

/// A selected item on its way to Transmission.
pub struct Job {
    /// The observation to record; `result`, `torrent_hash` and `reason` are
    /// filled in from what Transmission says.
    observation: Observation,
    /// The title with the channel's secret values replaced, for logs.
    title: String,
    /// The item's own link, which Transmission is asked to add.
    link: String,
    save_path: PathBuf,
    /// The folder whose turn the item takes ([`work_folder_of`]).
    work_folder: PathBuf,
    episode: isize,
    channel_label: String,
    /// The replacement of this revision failed before it was received, and
    /// receiving the item again carries it on as the row decided.
    retry: Option<Box<Revision>>,
}

impl Job {
    /// What recording the item as failed needs, should its task panic.
    pub fn fallback(&self) -> Fallback {
        Fallback {
            observation: self.observation.clone(),
            title: self.title.clone(),
            channel_label: self.channel_label.clone(),
        }
    }
}

/// What happened to a [`Job`].
pub enum JobOutcome {
    /// Transmission holds the torrent (added now or already there).
    Held { hash: String, kind: AddKind },
    /// Transmission does not hold the torrent as far as the worker knows.
    /// `unconfirmed` is set when the request failed without an answer (a
    /// connection error or timeout): Transmission may have added the torrent
    /// all the same, under a hash the worker never learned.
    Failed { unconfirmed: bool },
    /// Shutdown began before the item was started; nothing was done or recorded.
    NotStarted,
    /// A revision that was not added (see [`Plan::Unknown`], [`Plan::Skip`]).
    Withheld,
    /// A revision whose decision waits for the next cycle; nothing was done or
    /// recorded.
    Later,
    /// The rule changed while the item waited for its turn; nothing was done
    /// or recorded.
    RuleChanged,
    /// The rule's work folder is in the archive folder, so nothing was added;
    /// nothing was recorded either. `asked` is set when this item's turn stored
    /// the `start` command that brings it in and paused the rule.
    MovingFirst { asked: bool },
}

/// What judging the items of one channel's feed came to.
#[derive(Default)]
pub struct Judged {
    /// The items a rule selected, to add.
    pub jobs: Vec<Job>,
    /// `(channel_id, identity_key)` of every item in the feed.
    pub present: Vec<(String, String)>,
    pub items_seen: usize,
    /// Items recorded here that had no record before.
    pub items_new: usize,
    pub no_match: usize,
    pub excluded: usize,
    /// Items a rule selected that were left alone because no collect folder is
    /// set. They are not recorded at all, so the next cycle judges them again.
    pub waiting_for_collect_folder: usize,
}

/// Judges the items of the feed `feed` read for `plan`'s channel at `at`:
/// records the ones that need no Transmission and returns the selected ones.
///
/// Without a collect folder the items are still judged (and the ones no rule
/// takes recorded), but nothing is selected.
pub async fn judge_feed(
    ctx: &CollectContext,
    plan: &ChannelPlan,
    feed: &rss::Channel,
    collect_folder: Option<&Path>,
    at: Millis,
) -> Judged {
    let channel = &plan.channel;
    let label = channel.masked_url();
    let mut judged = Judged::default();
    let mut skipped = Vec::new();

    // What goes into history and logs passes through the channel's redactor.
    let channel_redactor = plan.redactor();

    let feed_items = feed::items(feed, &channel.secret_query, &channel_redactor);
    // What history already knows of the items, read only when a rule that
    // holds back past items could take one of them: the items it recorded
    // before the subscription began, or while the rule was paused, are
    // past, and are not received.
    let known = if plan.has_past_holders() {
        let keys = feed_items.iter().map(|i| i.identity_key.clone()).collect();
        match ctx.history.known_items(channel.id.clone(), keys).await {
            Ok(known) => Some(known),
            Err(err) => {
                eprintln!("Cannot read history for {label}: {err}");
                None
            }
        }
    } else {
        Some(HashMap::new())
    };

    for item in feed_items {
        judged.items_seen += 1;
        let FeedItem {
            identity_key,
            title,
            stored_title,
            link,
            stored_link,
        } = item;
        judged
            .present
            .push((channel.id.clone(), identity_key.clone()));

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

        let judgement = plan.judge(&title);
        if let Judgement::Selected { rule_id, .. } = &judgement {
            match &known {
                // Without history there is no telling a past item from a
                // new one: a rule that holds back past items leaves the item for the
                // next cycle.
                None if plan.holds_past(rule_id) => continue,
                Some(known) => {
                    let record = known.get(&observation.identity_key).copied();
                    if plan.is_past(rule_id, record) {
                        if let Some(KnownItem { result: stored, .. }) = record {
                            skipped.push(Observation {
                                result: stored,
                                ..observation
                            });
                        }
                        continue;
                    }
                }
                None => {}
            }
        }
        match judgement {
            // No collect folder, so there is nowhere to save. The item is
            // neither added nor recorded: a record (an `add_failed` above
            // all) would pile up as a failure the user did nothing wrong
            // to cause, and would need a retry. Left unrecorded it is new
            // again next cycle and is judged afresh once the folder is set.
            Judgement::Selected { .. } if collect_folder.is_none() => {
                judged.waiting_for_collect_folder += 1;
            }
            Judgement::Selected {
                rule_id,
                save_path,
                episode,
            } => judged.jobs.push(Job {
                observation: Observation {
                    rule_id: Some(rule_id),
                    ..observation
                },
                title: stored_title,
                link,
                work_folder: work_folder_of(collect_folder.unwrap_or(Path::new("")), &save_path),
                save_path,
                episode,
                channel_label: label.clone(),
                retry: None,
            }),
            Judgement::Excluded => {
                judged.excluded += 1;
                skipped.push(Observation {
                    result: HistoryResult::Excluded,
                    ..observation
                });
            }
            Judgement::NoMatch => {
                judged.no_match += 1;
                skipped.push(observation);
            }
        }
    }

    match ctx.history.record(at, skipped).await {
        Ok(recorded) => {
            judged.items_new += recorded.iter().filter(|r| **r == Recorded::New).count();
        }
        Err(err) => eprintln!("Cannot record history for {label}: {err}"),
    }
    judged
}

/// What [`leave_revisions`] left out of the jobs.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Left {
    /// Lower revisions of a release that replaced a video in the folder,
    /// recorded as its duplicates or not received again.
    pub withheld: usize,
    /// Items a replacement has decided about, or whose rows could not be read.
    pub left: usize,
    /// Of the lower revisions recorded, those history had no record of.
    pub items_new: usize,
}

/// The jobs a cycle adds: those of `jobs` that no video revision replacement
/// has decided about (see [`crate::revisions`]). A revision with a row is the
/// replacement's to carry on, unless its replacement failed before the new
/// video was received: then the item is received again, as the row decided.
/// The old video of a replacement that removed its torrent is not received
/// again, through any channel, nor is a lower revision of a release that
/// replaced a video in the same folder. Without the rows the jobs wait for
/// the next cycle: adding the old video's item again could bring it back.
pub async fn leave_revisions(ctx: &CollectContext, jobs: Vec<Job>, at: Millis) -> (Vec<Job>, Left) {
    let mut left = Left::default();
    let mut keys: HashMap<String, Vec<String>> = HashMap::new();
    for job in &jobs {
        keys.entry(job.observation.channel_id.clone())
            .or_default()
            .push(job.observation.identity_key.clone());
    }
    let mut marks: HashMap<String, Option<HashMap<String, Mark>>> = HashMap::new();
    for (channel, keys) in keys {
        let read = match ctx.revisions.marks(channel.clone(), keys).await {
            Ok(found) => Some(found),
            Err(err) => {
                eprintln!("Cannot read the video revisions of a channel; its items wait: {err}");
                None
            }
        };
        marks.insert(channel, read);
    }
    let replaced = match ctx.revisions.replacements().await {
        Ok(found) => Replaced::new(found),
        Err(err) => {
            eprintln!("Cannot read the video revisions; the selected items wait: {err}");
            left.left += jobs.len();
            return (Vec::new(), left);
        }
    };
    // A lower revision of a release that replaced a video in the job's folder.
    let replaced = |job: &Job| replaced.holds_higher(&job.save_path, &job.title);
    let mut kept = Vec::new();
    // Lower revisions of a release in place, recorded as the folder's
    // duplicates as `plan` records them.
    let mut lower = Vec::new();
    for mut job in jobs {
        let mark = match marks.get(&job.observation.channel_id) {
            Some(Some(found)) => Ok(found.get(&job.observation.identity_key)),
            _ => Err(()),
        };
        match mark {
            Ok(None) if replaced(&job) => {
                println!(
                    "Not adding {} ({}): {}",
                    job.title,
                    job.channel_label,
                    revisions::NOT_HIGHER
                );
                left.withheld += 1;
                lower.push(Observation {
                    result: HistoryResult::Duplicate,
                    reason: Some(revisions::NOT_HIGHER.to_owned()),
                    ..job.observation
                });
            }
            Ok(None) => kept.push(job),
            // A failure before the revision was received is not received
            // again once a higher revision replaced the video: the
            // replacement steps skip it.
            Ok(Some(Mark::Retry(_))) if replaced(&job) => {
                println!(
                    "Not adding {} ({}) again: {}",
                    job.title,
                    job.channel_label,
                    revisions::NOT_HIGHER
                );
                left.withheld += 1;
            }
            Ok(Some(Mark::Retry(row))) => {
                job.retry = Some(row.clone());
                kept.push(job);
            }
            _ => left.left += 1,
        }
    }
    match ctx.history.record(at, lower).await {
        Ok(recorded) => {
            left.items_new += recorded.iter().filter(|r| **r == Recorded::New).count();
        }
        Err(err) => eprintln!("Cannot record history for the lower revisions: {err}"),
    }
    (kept, left)
}

/// Gives the jobs of a new season's rule (one of `open_rules`) the episode
/// offset the rule gets from its first items ([`offsets::settle`]), before the
/// first of them is named, so it is named with it.
pub async fn settle_offsets(
    ctx: &CollectContext,
    collect_folder: &str,
    open_rules: &HashMap<String, Rule>,
    jobs: &mut [Job],
) {
    let mut firsts: HashMap<String, Vec<String>> = HashMap::new();
    for job in jobs.iter() {
        if let Some(rule_id) = &job.observation.rule_id {
            if open_rules.contains_key(rule_id) {
                firsts
                    .entry(rule_id.clone())
                    .or_default()
                    .push(job.title.clone());
            }
        }
    }
    let offsets = offsets::settle(&ctx.offsets(), collect_folder, open_rules, &firsts).await;
    for job in jobs.iter_mut() {
        if let Some(offset) = job
            .observation
            .rule_id
            .as_ref()
            .and_then(|id| offsets.get(id))
        {
            job.episode = *offset as isize;
        }
    }
}

/// What is needed to record a job's item as failed when its task panics
/// ([`Job::fallback`]).
pub struct Fallback {
    observation: Observation,
    title: String,
    channel_label: String,
}

/// Records the item of a job whose task panicked as failed.
pub async fn record_panic(ctx: &CollectContext, at: Millis, fallback: Fallback) {
    eprintln!(
        "Internal error while adding {} ({})",
        fallback.title, fallback.channel_label
    );
    let observation = Observation {
        result: HistoryResult::AddFailed,
        reason: Some(receive::ADD_PANICKED.to_owned()),
        ..fallback.observation
    };
    let receiving = ctx.receive();
    let written = receive::record(&receiving, at, HistoryWrite::Observe(observation), None);
    if let Err(err) = written.await {
        eprintln!(
            "Cannot record history for {}: {err}",
            fallback.channel_label
        );
    }
}

/// Adds one item, records the result, then renames the torrent (see
/// [`rename_mode`]). The second value tells whether the item had no record
/// before.
///
/// The item takes its turn at its work folder here, for its add and its
/// rename.
pub async fn process_job(
    ctx: CollectContext,
    job: Job,
    at: Millis,
    redactor: Redactor,
    cancel: CancellationToken,
    listing: Arc<Listing>,
) -> (JobOutcome, bool) {
    if cancel.is_cancelled() {
        return (JobOutcome::NotStarted, false);
    }
    // The item's turn at its work folder, for its add and its rename: a
    // command that moves the folder (an archive) or renames in it (an undo)
    // runs before or after, never during, and so does a retry of the same
    // item.
    let _turn = ctx
        .folders
        .lock(
            Section::new()
                .read(&job.work_folder)
                .item(&job.observation.channel_id, &job.observation.identity_key),
        )
        .await;
    if cancel.is_cancelled() {
        return (JobOutcome::NotStarted, false);
    }
    // Such a command may have changed the rule while the item waited: an
    // archived rule receives nothing, and an undone offset names nothing.
    if let Some(rule_id) = &job.observation.rule_id {
        let found = ctx.channels.get_rule(rule_id).await;
        let unchanged = match &found {
            Ok(Some(rule)) => {
                rule.state == RuleState::Active && rule.episode as isize == job.episode
            }
            Ok(None) => false,
            Err(err) => {
                eprintln!(
                    "Cannot read the rule of {} ({}): {err}",
                    job.title, job.channel_label
                );
                false
            }
        };
        if !unchanged {
            println!(
                "{} ({}) waits for the next cycle: its rule changed while the cycle ran",
                job.title, job.channel_label
            );
            return (JobOutcome::RuleChanged, false);
        }
        // Right before the add, under the turn: a work is never split across
        // the collect folder and the archive folder. A rule whose work folder
        // is in the archive folder (a title given to a subscription that
        // waited for one, a rule changed as its sibling was archived) is
        // paused and its folder brought in first; the item waits for a later
        // cycle. The command needs write turns on the folders, so nothing
        // waits for it here.
        if let Ok(Some(rule)) = &found {
            match move_before_receiving(&ctx.channels, &ctx.settings, &ctx.commands, rule, at).await
            {
                Ok(Receiving::Go) => {}
                Ok(Receiving::MoveFirst { work, asked }) => {
                    println!(
                        "{} ({}) waits for the next cycle: its work folder {work:?} is in the \
                         archive folder and moves into the collect folder first",
                        job.title, job.channel_label
                    );
                    return (JobOutcome::MovingFirst { asked }, false);
                }
                Err(err) => {
                    eprintln!(
                        "{} ({}) waits for the next cycle: cannot tell where its work folder is: {err}",
                        job.title, job.channel_label
                    );
                    return (JobOutcome::RuleChanged, false);
                }
            }
        }
    }

    let plan = if let Some(row) = &job.retry {
        // Decided before; receiving it again goes on with that.
        Plan::Replace(Decided::of(row))
    } else if revisions::is_revision(&job.title) {
        revisions::plan(
            &ctx.revision_work(),
            &Selected {
                channel_id: &job.observation.channel_id,
                identity_key: &job.observation.identity_key,
                title: &job.title,
                save_path: &job.save_path,
                episode: job.episode,
            },
            &listing,
        )
        .await
    } else {
        Plan::Normal
    };
    let replacing = match plan {
        Plan::Normal => None,
        Plan::Replace(decided) => Some(decided),
        Plan::Unknown(decided, reason) => {
            return withhold(&ctx, job, at, decided, RevisionState::Unknown, reason).await
        }
        Plan::Skip(decided, reason) => {
            return withhold(&ctx, job, at, decided, RevisionState::Skipped, reason).await
        }
        Plan::Later(why) => {
            println!(
                "Revision {} ({}) waits for the next cycle: {}",
                job.title,
                job.channel_label,
                redactor.apply(&why)
            );
            return (JobOutcome::Later, false);
        }
    };

    let receiving = ctx.receive();
    let mut transmission = ctx.transmission.client();

    let label =
        transmission::item_label(&job.observation.channel_id, &job.observation.identity_key);
    let added = receive::add(
        &mut transmission,
        &job.link,
        &job.save_path,
        AddLabels {
            item: Some(&label),
            command: None,
        },
        &redactor,
    )
    .await;

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
                    ..job.observation.clone()
                },
                JobOutcome::Held {
                    hash: torrent.hash.clone(),
                    kind: torrent.kind,
                },
            )
        }
        Err(failure) => {
            let reason = failure.reason.clone();
            eprintln!("Cannot add {} ({}): {reason}", job.title, job.channel_label);
            (
                Observation {
                    result: HistoryResult::AddFailed,
                    reason: Some(reason),
                    ..job.observation.clone()
                },
                // Only a refusal says this add did not leave the torrent in
                // Transmission. One that could not connect sent nothing, but
                // the item may be one whose hash only this add's `duplicate`
                // answer would have given (an earlier add of it got no answer).
                JobOutcome::Failed {
                    unconfirmed: !failure.refused(),
                },
            )
        }
    };

    if let (Ok(torrent), Some(decided)) = (&added, &replacing) {
        // A revision keeps its received name until the old video is gone
        // (see [`revisions::advance`]). Its replacement is written with the
        // history record.
        let was_new = start_replacement(
            &receiving,
            &job,
            at,
            observation,
            decided.clone(),
            torrent.kind,
            &torrent.hash,
        )
        .await;
        return (outcome, was_new);
    }

    // Recorded as soon as Transmission has answered, before the renaming
    // that can take many seconds.
    let written = receive::record(&receiving, at, HistoryWrite::Observe(observation), None).await;
    let was_new = match written {
        Ok(stored) => stored.recorded == Some(Recorded::New),
        Err(err) => {
            eprintln!("Cannot record history for {}: {err}", job.channel_label);
            false
        }
    };

    if let Ok(torrent) = &added {
        #[cfg(feature = "test-support")]
        fault::check(&ctx.transmission.url);
        if let Some(mode) = rename_mode(&ctx, torrent.kind, &torrent.hash).await {
            let rename = RenameJob {
                hash: &torrent.hash,
                save_path: &job.save_path,
                episode: job.episode,
                mode,
                original: Original::Current,
                // A torrent this cycle has just added gets the legacy
                // treatment (ticket 0128 ends it); one that was there already
                // is never removed.
                underivable: Underivable::Remove,
                // The cycle notes nothing on the item (0128 adds it).
                note: None,
                until_renamed: true,
                redactor: &redactor,
            };
            receive::rename(&receiving, &rename, &cancel).await;
        }
    }

    (outcome, was_new)
}

/// Test support: a panic in a cycle's item task right after its torrent is
/// recorded, before the rename, for the tests of what the worker makes of an
/// item task that panics. Nothing on that path panics on what Transmission
/// answers, so a test cannot get there through the fake Transmission.
#[cfg(feature = "test-support")]
pub mod fault {
    use std::sync::{Mutex, PoisonError};

    use url::Url;

    /// The Transmission URLs whose cycles panic so.
    static PANIC_AFTER_RECORD: Mutex<Vec<String>> = Mutex::new(Vec::new());

    /// Until the returned guard is dropped, every item task of a cycle that
    /// reaches Transmission at `url` and gets a torrent from it panics right
    /// after the item is recorded. Keyed by the URL, so tests running side by
    /// side, each with its own fake Transmission, are left alone.
    pub fn panic_after_record(url: &str) -> PanicAfterRecord {
        let url = Url::parse(url).map_or_else(|_| url.to_owned(), String::from);
        PANIC_AFTER_RECORD
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(url.clone());
        PanicAfterRecord { url }
    }

    /// Stops the panics of [`panic_after_record`] when dropped.
    pub struct PanicAfterRecord {
        url: String,
    }

    impl Drop for PanicAfterRecord {
        fn drop(&mut self) {
            let mut urls = PANIC_AFTER_RECORD
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if let Some(at) = urls.iter().position(|url| *url == self.url) {
                urls.swap_remove(at);
            }
        }
    }

    pub(super) fn check(url: &Url) {
        let asked = PANIC_AFTER_RECORD
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .any(|asked| asked == url.as_str());
        if asked {
            panic!("a panic asked for after the item was recorded");
        }
    }
}

/// The row a cycle decides for `job` (its item ID is filled in when the row
/// is written with the item's history record).
fn new_revision(
    job: &Job,
    decided: Decided,
    state: RevisionState,
    reason: Option<String>,
    hash: Option<String>,
) -> NewRevision {
    decided.row(
        0,
        job.observation.rule_id.clone().unwrap_or_default(),
        &job.save_path,
        state,
        reason,
        hash,
    )
}

/// Records a revision that is not received (`버전 미상`, or the folder holds
/// it already) and its decision, together.
async fn withhold(
    ctx: &CollectContext,
    job: Job,
    at: Millis,
    decided: Decided,
    state: RevisionState,
    reason: &'static str,
) -> (JobOutcome, bool) {
    let result = match state {
        RevisionState::Unknown => HistoryResult::VersionUnknown,
        _ => HistoryResult::Duplicate,
    };
    println!("Not adding {} ({}): {reason}", job.title, job.channel_label);
    let observation = Observation {
        result,
        reason: Some(reason.to_owned()),
        ..job.observation.clone()
    };
    let row = new_revision(&job, decided, state, Some(reason.to_owned()), None);
    let written = ctx
        .revisions
        .write_with_history(
            at,
            HistoryWrite::Observe(observation),
            RowWrite::Create {
                new: row,
                reopen: false,
            },
        )
        .await;
    match written {
        Ok(written) => (
            JobOutcome::Withheld,
            written.recorded == Some(Recorded::New),
        ),
        Err(err) => {
            eprintln!(
                "Cannot record history and the revision of {} ({}): {err}",
                job.title, job.channel_label
            );
            (JobOutcome::Later, false)
        }
    }
}

/// Records `observation` of a revision Transmission now holds as `hash`
/// (`kind` says whether it was added now) and starts its replacement, in one
/// transaction; returns whether the item was new to history. Without them
/// the next cycle decides again, and the torrent keeps its received name
/// meanwhile. A row that failed before its video was received starts over
/// when the torrent was added again (it had gone); one Transmission still
/// holds is looked at by the replacement steps.
async fn start_replacement(
    ctx: &ReceiveContext,
    job: &Job,
    at: Millis,
    observation: Observation,
    decided: Decided,
    kind: AddKind,
    hash: &str,
) -> bool {
    let row = new_revision(
        job,
        decided,
        RevisionState::Receiving,
        None,
        Some(hash.to_owned()),
    );
    let reopened = job.retry.is_some() && kind == AddKind::Added;
    let written = receive::record(
        ctx,
        at,
        HistoryWrite::Observe(observation),
        Some(RowWrite::Create {
            new: row,
            reopen: kind == AddKind::Added,
        }),
    )
    .await;
    let written = match written {
        Ok(written) => written,
        Err(err) => {
            eprintln!(
                "Cannot record history and the revision of {} ({}): {err}",
                job.title, job.channel_label
            );
            return false;
        }
    };
    if written
        .row
        .as_ref()
        .is_some_and(|row| row.state == RevisionState::Receiving)
    {
        if reopened {
            println!("Receiving {} again for its replacement", job.title);
        } else {
            println!(
                "Replacing with {}: received under its own name until checked",
                job.title
            );
        }
    }
    written.recorded == Some(Recorded::New)
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
async fn rename_mode(ctx: &CollectContext, kind: AddKind, hash: &str) -> Option<RenameMode> {
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

/// Removes the finished bot-labelled torrents nothing accounts for: not
/// `kept` (held for this cycle's items), not in history for a channel in
/// `unread_channels` or an item in `present`, not held by a replacement under
/// way, and not labelled for such an item. The caller runs it with the
/// torrent gate held, only when the whole picture is in (see `trss-worker`'s
/// `run_cycle`). A history that cannot be read removes nothing.
///
/// - A torrent that history records as coming from a channel whose feed was
///   not read this cycle stays: that feed may have dropped it, or may just
///   have been down.
/// - A torrent whose item label ([`transmission::item_label`]) names an item
///   in a feed read this cycle, or a channel whose feed was not read, stays
///   the same way, whether or not history learned its hash: the label went
///   in with the add, so it holds even for an add whose answer was lost.
/// - A torrent with neither (for example one the legacy cron added before
///   the switch, until a cycle meets it and labels it) has no channel to
///   wait for and follows the plain rule.
/// - A torrent that history records as received (or found already there) for
///   an item that is still in a feed read this cycle stays, though no rule
///   selects it now (the rule was edited or archived since a retry added it,
///   or an older receive by hand, which no rule ever selected).
pub async fn remove_departed(
    ctx: &CollectContext,
    mut kept: HashSet<String>,
    present: Vec<(String, String)>,
    unread_channels: Vec<String>,
    redactor: &Redactor,
) -> Vec<RemovedTorrent> {
    let present_items: HashSet<(String, String)> = present.iter().cloned().collect();
    let unread: HashSet<String> = unread_channels.iter().cloned().collect();
    let labelled_for_a_waiting_item = |labels: &[String]| {
        labels
            .iter()
            .filter_map(|label| transmission::item_of_label(label))
            .any(|(channel, key)| {
                unread.contains(channel)
                    || present_items.contains(&(channel.to_owned(), key.to_owned()))
            })
    };
    let recorded = async {
        let mut hashes = ctx
            .history
            .torrent_hashes_of_channels(unread_channels)
            .await?;
        hashes.extend(ctx.history.held_hashes_of_items(present).await?);
        Ok::<_, crate::store::history::HistoryError>(hashes)
    }
    .await;
    // A replacement under way keeps its new torrent, which it checks,
    // removes the old video next to and renames, whatever the feeds say.
    let recorded = match (recorded, ctx.revisions.held_hashes().await) {
        (Ok(mut hashes), Ok(held)) => {
            hashes.extend(held);
            Ok(hashes)
        }
        (Err(err), _) => Err(err.to_string()),
        (_, Err(err)) => Err(err.to_string()),
    };
    match recorded {
        Ok(hashes) => {
            kept.extend(hashes);
            let mut transmission = ctx.transmission.client();
            remove_stale(
                &mut transmission,
                |hash, labels| kept.contains(hash) || labelled_for_a_waiting_item(labels),
                redactor,
            )
            .await
        }
        Err(err) => {
            eprintln!(
                "Cannot tell which torrents came from unread channels ({err}); \
                 leaving Transmission's torrents alone this cycle"
            );
            Vec::new()
        }
    }
}
