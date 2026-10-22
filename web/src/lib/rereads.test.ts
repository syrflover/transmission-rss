import assert from "node:assert/strict";
import { test } from "node:test";

import { jobChanged, jobListChanged, suggestionsChanged, todosChanged } from "../screens/todo/listens.ts";
import { channelListChanged, historyChanged, rulesChanged, subscriptionsChanged } from "../screens/collect/listens.ts";
import { boardChanged } from "../screens/collect/status/listens.ts";
import {
  applyChanged,
  candidatesChanged,
  cleanupChanged,
  movieLinkChanged,
  seasonInfoChanged,
  workChanged,
  workCoverChanged,
} from "../screens/library/detail/listens.ts";
import { libraryListChanged } from "../screens/library/listens.ts";
import { scheduleChanged } from "../screens/schedule/listens.ts";
import { COLLECTION, POLICY, STORAGE, WATCH_FOLDERS } from "../screens/settings/listens.ts";
import type { Change } from "./api-types";
import { createEvents, type EventSourceLike } from "./events.ts";
import { createRereader, READ_TIMEOUT_MS, ReadTimedOut, type RereaderOptions } from "./rereads.ts";

class FakeSource implements EventSourceLike {
  readyState = 1;
  private readonly listeners = new Map<string, ((event: { data: string }) => void)[]>();
  addEventListener(type: string, listener: (event: { data: string }) => void) {
    this.listeners.set(type, [...(this.listeners.get(type) ?? []), listener]);
  }
  close() {
    this.readyState = 2;
  }
  say(type: string, data = "{}") {
    for (const listener of this.listeners.get(type) ?? []) listener({ data });
  }
  change(kind: string, id: string | null) {
    this.say("change", JSON.stringify({ kind, id }));
  }
}

/** Lets the reads that were started run to their end. */
const settle = () => new Promise<void>((resolve) => setImmediate(resolve));

/** The timers the test runs by hand. */
function timers() {
  const pending: { run: () => void; ms: number; live: boolean }[] = [];
  return {
    setTimer: (run: () => void, ms: number) => {
      const timer = { run, ms, live: true };
      pending.push(timer);
      return () => {
        timer.live = false;
      };
    },
    /** Runs the timers that wait; returns the delays they had. */
    fire: () => {
      const due = pending.filter((t) => t.live);
      for (const timer of due) {
        timer.live = false;
        timer.run();
      }
      return due.map((t) => t.ms);
    },
    waiting: () => pending.filter((t) => t.live).length,
  };
}

/**
 * A screen as the hooks build it: a rereader whose reads the test holds open,
 * listening to the stream through `createEvents` with `match`.
 */
function screen(match: (change: Change) => boolean, options: Partial<RereaderOptions> = {}) {
  const sources: FakeSource[] = [];
  const events = createEvents({
    open: () => {
      const source = new FakeSource();
      sources.push(source);
      return source;
    },
    visibility: { hidden: () => false, onChange: () => () => {} },
    setTimer: () => () => {},
  });
  const log = { started: 0, ended: 0, concurrent: 0, mostAtOnce: 0 };
  let release: (() => void) | null = null;
  let hold = false;
  const rereader = createRereader({
    read: async () => {
      log.started += 1;
      log.concurrent += 1;
      log.mostAtOnce = Math.max(log.mostAtOnce, log.concurrent);
      try {
        if (hold) await new Promise<void>((resolve) => (release = resolve));
      } finally {
        log.concurrent -= 1;
        log.ended += 1;
      }
    },
    ...options,
  });
  // Listening starts before the first read, as the hooks do.
  const stop = events.subscribe({ match, reread: () => rereader.request() });
  return {
    log,
    rereader,
    stop,
    source: () => sources[0],
    /** The next read is held until `finish`. */
    holdReads: () => {
      hold = true;
    },
    finish: async () => {
      release?.();
      release = null;
      await settle();
    },
    stopHolding: () => {
      hold = false;
    },
  };
}

test("a burst of matching events during a read leads to exactly one more read", async () => {
  const s = screen(todosChanged);
  s.holdReads();
  s.rereader.start();
  await settle();
  assert.equal(s.log.started, 1);

  // What a collection cycle and a subtitle job announce within milliseconds (lighter-structure/0164).
  for (let i = 0; i < 250; i++) s.source().change("history", String(i));
  for (let i = 0; i < 12; i++) s.source().change("subtitle_job", "job-1");
  await settle();
  // Nothing starts beside the read on its way.
  assert.equal(s.log.started, 1);

  // The read ends: one more starts, and it is held as well.
  await s.finish();
  assert.equal(s.log.started, 2);
  assert.equal(s.log.ended, 1);

  // Nothing came during the second read: it ends the matter.
  await s.finish();
  assert.deepEqual(s.log, { started: 2, ended: 2, concurrent: 0, mostAtOnce: 1 });
});

test("a change that comes when nothing is read starts a read at once, and each one after the last read ended does", async () => {
  const s = screen(jobListChanged);
  s.rereader.start();
  await settle();
  assert.equal(s.log.started, 1);

  s.source().change("subtitle_job", "a");
  await settle();
  s.source().change("subtitle_job", "b");
  await settle();
  assert.equal(s.log.started, 3);

  // A change the screen does not read is not a read.
  s.source().change("collect_status", null);
  s.source().change("settings", null);
  await settle();
  assert.equal(s.log.started, 3);
});

test("ready and lagged read what the screen shows once, even during a read", async () => {
  const s = screen(() => false);
  s.rereader.start();
  await settle();
  s.source().say("ready");
  await settle();
  s.source().say("lagged");
  await settle();
  assert.equal(s.log.started, 3);

  s.holdReads();
  s.source().say("ready");
  await settle();
  s.source().say("lagged");
  s.source().say("ready");
  await settle();
  assert.equal(s.log.started, 4);
  s.stopHolding();
  await s.finish();
  assert.equal(s.log.started, 5);
});

test("a read that failed is tried again after the retry time, and a change reads it at once", async () => {
  const t = timers();
  let fail = true;
  let reads = 0;
  const rereader = createRereader({
    read: async () => {
      reads += 1;
      if (fail) throw new Error("down");
    },
    retryMs: () => 2000,
    setTimer: t.setTimer,
  });
  rereader.start();
  await settle();
  assert.equal(reads, 1);
  assert.equal(t.waiting(), 1);

  // The retry waits the time the screen used to read at, and reads again.
  assert.deepEqual(t.fire(), [2000]);
  await settle();
  assert.equal(reads, 2);
  assert.equal(t.waiting(), 1);

  // A change reads at once and takes the retry's place; the read after it succeeds and leaves none.
  fail = false;
  rereader.request();
  await settle();
  assert.equal(reads, 3);
  assert.equal(t.waiting(), 0);
  assert.deepEqual(t.fire(), []);
});

test("a change that comes during a read that fails reads again at once, not after the retry time", async () => {
  const t = timers();
  let release: (() => void) | null = null;
  let reads = 0;
  const rereader = createRereader({
    read: async () => {
      reads += 1;
      if (reads === 1) {
        await new Promise<void>((resolve) => (release = resolve));
        throw new Error("down");
      }
    },
    retryMs: () => 2000,
    setTimer: t.setTimer,
  });
  rereader.start();
  await settle();
  rereader.request();
  rereader.request();
  release!();
  await settle();
  assert.equal(reads, 2);
  assert.equal(t.waiting(), 0);
});

test("what is gone is not read again, whatever comes", async () => {
  let reads = 0;
  const rereader = createRereader({
    read: async () => {
      reads += 1;
      throw Object.assign(new Error("no job"), { code: "not_found" });
    },
    gone: (e) => (e as { code?: string }).code === "not_found",
    retryMs: () => 2000,
  });
  rereader.start();
  await settle();
  rereader.request();
  await settle();
  assert.equal(reads, 1);
});

test("nothing is read while the page is hidden, but the first read is", async () => {
  let hidden = true;
  let reads = 0;
  const rereader = createRereader({
    read: async () => void (reads += 1),
    hidden: () => hidden,
  });
  rereader.start();
  await settle();
  rereader.request();
  await settle();
  assert.equal(reads, 1);

  hidden = false;
  rereader.request();
  await settle();
  assert.equal(reads, 2);
});

test("a stopped rereader aborts the read on its way, forgets its retry and reads nothing more", async () => {
  const t = timers();
  let signalSeen: AbortSignal | null = null;
  let reads = 0;
  const rereader = createRereader({
    read: (signal) => {
      reads += 1;
      signalSeen = signal;
      return new Promise<void>((_, reject) => signal.addEventListener("abort", () => reject(new Error("aborted"))));
    },
    retryMs: () => 1000,
    setTimer: t.setTimer,
  });
  rereader.start();
  await settle();
  rereader.request();
  rereader.stop();
  await settle();
  assert.equal(signalSeen!.aborted, true);
  // The aborted read is not a failure to retry, and the request that came during it is dropped.
  assert.equal(t.waiting(), 0);
  rereader.request();
  rereader.start();
  await settle();
  assert.equal(reads, 1);
});

test("a read that takes longer than its time counts as failed: it is aborted, and the retry reads again", async () => {
  const t = timers();
  const signals: AbortSignal[] = [];
  const answers: (() => void)[] = [];
  const rereader = createRereader({
    // Each read is answered when the test says; the first never in time (a connection that died without closing).
    read: (signal) => {
      signals.push(signal);
      return new Promise<void>((resolve) => answers.push(resolve));
    },
    retryMs: () => 2000,
    setTimer: t.setTimer,
  });
  rereader.start();
  await settle();
  assert.equal(t.waiting(), 1);

  assert.deepEqual(t.fire(), [READ_TIMEOUT_MS]);
  await settle();
  assert.equal(signals[0].aborted, true);
  assert.ok(signals[0].reason instanceof ReadTimedOut);
  // Failed like a lost connection: the retry waits, and reads.
  assert.deepEqual(t.fire(), [2000]);
  await settle();
  assert.equal(signals.length, 2);

  // The first read's answer that comes at last does not end the read on its way: a request still waits for it.
  answers[0]();
  await settle();
  rereader.request();
  await settle();
  assert.equal(signals.length, 2);
  answers[1]();
  await settle();
  assert.equal(signals.length, 3);
  assert.equal(signals[1].aborted, false);
});

test("a change that comes during a read that runs out of time reads again at once", async () => {
  const t = timers();
  let reads = 0;
  const rereader = createRereader({
    read: (signal) => {
      reads += 1;
      if (reads > 1) return Promise.resolve();
      // A read that ends when aborted, as `fetch` does.
      return new Promise<void>((_, reject) => signal.addEventListener("abort", () => reject(signal.reason)));
    },
    retryMs: () => 2000,
    setTimer: t.setTimer,
    timeoutMs: 500,
  });
  rereader.start();
  await settle();
  rereader.request();
  rereader.request();
  assert.equal(reads, 1);

  assert.deepEqual(t.fire(), [500]);
  await settle();
  assert.equal(reads, 2);
  assert.equal(t.waiting(), 0);
});

test("a screen that stops listening hears no more, and the connection closes with the last one", async () => {
  const s = screen(todosChanged);
  s.rereader.start();
  await settle();
  const source = s.source();
  s.stop();
  s.rereader.stop();
  source.change("subtitle_job", "a");
  await settle();
  assert.equal(s.log.started, 1);
  assert.equal(source.readyState, 2);
});

const change = (kind: Change["kind"], id: string | null): Change => ({ kind, id });

test("the job list reads again for jobs and for what its rows name of the work, not for others", () => {
  for (const kind of ["subtitle_job", "library", "artwork", "anissia"] as const) {
    assert.equal(jobListChanged(change(kind, "x")), true, kind);
    assert.equal(jobListChanged(change(kind, null)), true, kind);
  }
  for (const kind of ["subtitle_screen", "history", "collect_status", "settings", "command", "rules"] as const) {
    assert.equal(jobListChanged(change(kind, null)), false, kind);
  }
});

test("the job detail reads again for its job and screen, and for its work, not for another job", () => {
  assert.equal(jobChanged(change("subtitle_job", "j1"), "j1", "w1"), true);
  assert.equal(jobChanged(change("subtitle_job", null), "j1", "w1"), true);
  assert.equal(jobChanged(change("subtitle_job", "j2"), "j1", "w1"), false);
  assert.equal(jobChanged(change("subtitle_screen", "j1"), "j1", "w1"), true);
  assert.equal(jobChanged(change("subtitle_screen", "j2"), "j1", "w1"), false);
  assert.equal(jobChanged(change("library", "w1"), "j1", "w1"), true);
  assert.equal(jobChanged(change("library", "w2"), "j1", "w1"), false);
  assert.equal(jobChanged(change("library", null), "j1", "w1"), true);
  assert.equal(jobChanged(change("subtitle_store", "w1"), "j1", "w1"), true);
  assert.equal(jobChanged(change("season_info", "w1"), "j1", "w1"), true);
  assert.equal(jobChanged(change("artwork", "w1"), "j1", "w1"), true);
  assert.equal(jobChanged(change("anissia", "1234"), "j1", "w1"), true);
  assert.equal(jobChanged(change("history", null), "j1", "w1"), false);
  // Before the first read any work may be the job's; a job with no work has none to hear of.
  assert.equal(jobChanged(change("library", "w9"), "j1", undefined), true);
  assert.equal(jobChanged(change("library", "w9"), "j1", null), false);
  assert.equal(jobChanged(change("subtitle_job", "j1"), "j1", null), true);
});

test("the to-dos read again for the kinds they are gathered from and for the old list of torrents, and not for a remote screen", () => {
  for (const kind of [
    "subtitle_job",
    "subtitle_store",
    "subtitle_mapping",
    "library",
    "artwork",
    "history",
    "rules",
    "anissia",
    // The list of Transmission's torrents getting too old, which the server announces with no write behind it.
    "collect_status",
    "settings",
  ] as const) {
    assert.equal(todosChanged(change(kind, null)), true, kind);
  }
  for (const kind of ["subtitle_screen", "command", "past_search", "season_info"] as const) {
    assert.equal(todosChanged(change(kind, null)), false, kind);
  }
  assert.equal(suggestionsChanged(change("collect_status", null)), true);
  assert.equal(suggestionsChanged(change("rules", "r1")), true);
  assert.equal(suggestionsChanged(change("subtitle_job", "j1")), false);
});

test("the candidates read again for their anime, work and jobs, and when Anissia is read with no anime named", () => {
  assert.equal(candidatesChanged(change("anissia", "1234"), "w1", 1234), true);
  assert.equal(candidatesChanged(change("anissia", null), "w1", 1234), true);
  assert.equal(candidatesChanged(change("anissia", "99"), "w1", 1234), false);
  assert.equal(candidatesChanged(change("subtitle_job", "any"), "w1", 1234), true);
  assert.equal(candidatesChanged(change("command", "c1"), "w1", 1234), true);
  assert.equal(candidatesChanged(change("season_info", "w1"), "w1", 1234), true);
  assert.equal(candidatesChanged(change("subtitle_mapping", "w2"), "w1", 1234), false);
  assert.equal(candidatesChanged(change("library", null), "w1", 1234), true);
  assert.equal(candidatesChanged(change("history", null), "w1", 1234), false);
  // A season with no Anissia link has no candidates to read.
  assert.equal(candidatesChanged(change("anissia", null), "w1", null), false);
});

test("the stored-file clean and the apply line read again for their work and job", () => {
  assert.equal(cleanupChanged(change("subtitle_store", "w1"), "w1"), true);
  assert.equal(cleanupChanged(change("subtitle_store", "w2"), "w1"), false);
  assert.equal(cleanupChanged(change("subtitle_store", null), "w1"), true);
  assert.equal(cleanupChanged(change("subtitle_job", "j1"), "w1"), true);
  assert.equal(cleanupChanged(change("library", "w1"), "w1"), false);

  assert.equal(applyChanged(change("subtitle_job", "j1"), "j1", "w1"), true);
  assert.equal(applyChanged(change("subtitle_job", "j2"), "j1", "w1"), false);
  assert.equal(applyChanged(change("library", "w1"), "j1", "w1"), true);
  assert.equal(applyChanged(change("library", "w2"), "j1", "w1"), false);
  assert.equal(applyChanged(change("subtitle_store", null), "j1", "w1"), true);
});

test("the schedule reads again for what its cards are made of and for the clock, not for jobs or a remote screen", () => {
  for (const kind of [
    "collect_status",
    "library",
    "rules",
    "anissia",
    "season_info",
    "artwork",
    "history",
    "settings",
  ] as const) {
    assert.equal(scheduleChanged(change(kind, null)), true, kind);
    assert.equal(scheduleChanged(change(kind, "x")), true, kind);
  }
  for (const kind of [
    "subtitle_job",
    "subtitle_screen",
    "subtitle_store",
    "subtitle_mapping",
    "command",
    "past_search",
  ] as const) {
    assert.equal(scheduleChanged(change(kind, null)), false, kind);
  }
});

test("the board reads again for the cycle, the channels, the history and the settings, not for the library", () => {
  for (const kind of ["collect_status", "rules", "history", "settings"] as const) {
    assert.equal(boardChanged(change(kind, null)), true, kind);
    assert.equal(boardChanged(change(kind, "x")), true, kind);
  }
  for (const kind of ["library", "anissia", "season_info", "artwork", "subtitle_job", "command"] as const) {
    assert.equal(boardChanged(change(kind, null)), false, kind);
  }
});

test("the cover and the season info read again for their work, and for a change that names no work", () => {
  assert.equal(workCoverChanged(change("artwork", "w1"), "w1"), true);
  assert.equal(workCoverChanged(change("artwork", null), "w1"), true);
  assert.equal(workCoverChanged(change("artwork", "w2"), "w1"), false);
  assert.equal(workCoverChanged(change("library", "w1"), "w1"), false);
  assert.equal(workCoverChanged(change("season_info", "w1"), "w1"), false);

  assert.equal(seasonInfoChanged(change("season_info", "w1"), "w1"), true);
  // An AniList entry belongs to no work: its change is every work's.
  assert.equal(seasonInfoChanged(change("season_info", null), "w1"), true);
  assert.equal(seasonInfoChanged(change("season_info", "w2"), "w1"), false);
  assert.equal(seasonInfoChanged(change("artwork", "w1"), "w1"), false);
});

test("the library list reads again for its works, their titles, covers and badges, not for a remote screen or a command", () => {
  for (const kind of [
    "library",
    "season_info",
    "anissia",
    "artwork",
    "settings",
    // What the to-do badges are gathered from.
    "subtitle_job",
    "subtitle_store",
    "subtitle_mapping",
    "rules",
    "history",
  ] as const) {
    assert.equal(libraryListChanged(change(kind, null)), true, kind);
    assert.equal(libraryListChanged(change(kind, "x")), true, kind);
  }
  for (const kind of ["subtitle_screen", "command", "past_search", "collect_status"] as const) {
    assert.equal(libraryListChanged(change(kind, null)), false, kind);
  }
});

test("a work's page reads again for its own work and for what names no work, and its movie link for its link and anime", () => {
  for (const kind of ["library", "season_info", "artwork", "subtitle_store", "subtitle_mapping"] as const) {
    assert.equal(workChanged(change(kind, "w1"), "w1"), true, kind);
    assert.equal(workChanged(change(kind, null), "w1"), true, kind);
    assert.equal(workChanged(change(kind, "w2"), "w1"), false, kind);
  }
  // A subscription that lands in the work's folder, a received revision, the folder move: their IDs name no work.
  for (const kind of ["rules", "anissia", "history", "subtitle_job", "settings", "command", "collect_status"] as const) {
    assert.equal(workChanged(change(kind, "x"), "w1"), true, kind);
  }
  for (const kind of ["subtitle_screen", "past_search"] as const) {
    assert.equal(workChanged(change(kind, null), "w1"), false, kind);
  }

  assert.equal(movieLinkChanged(change("season_info", "w1"), "w1"), true);
  assert.equal(movieLinkChanged(change("season_info", "w2"), "w1"), false);
  assert.equal(movieLinkChanged(change("anissia", "1234"), "w1"), true);
  assert.equal(movieLinkChanged(change("library", "w1"), "w1"), false);
});

test("the collection tabs read again for what their lists are made of, the history only where it is shown", () => {
  for (const kind of ["rules", "anissia", "collect_status"] as const) {
    assert.equal(subscriptionsChanged(change(kind, "x")), true, kind);
  }
  // A collection cycle's history does not make the subscriptions read again.
  for (const kind of ["history", "library", "artwork", "subtitle_job"] as const) {
    assert.equal(subscriptionsChanged(change(kind, null)), false, kind);
  }

  for (const kind of ["rules", "anissia", "history", "settings", "command", "library", "season_info", "artwork"] as const) {
    assert.equal(rulesChanged(change(kind, "x")), true, kind);
  }
  for (const kind of ["collect_status", "subtitle_job", "subtitle_store"] as const) {
    assert.equal(rulesChanged(change(kind, null)), false, kind);
  }

  for (const kind of ["history", "rules", "anissia", "command", "settings", "collect_status"] as const) {
    assert.equal(historyChanged(change(kind, "x")), true, kind);
  }
  for (const kind of ["library", "artwork", "subtitle_job", "season_info"] as const) {
    assert.equal(historyChanged(change(kind, null)), false, kind);
  }

  assert.equal(channelListChanged(change("rules", null)), true);
  assert.equal(channelListChanged(change("rules", "r1")), true);
  for (const kind of ["history", "collect_status", "settings"] as const) {
    assert.equal(channelListChanged(change(kind, null)), false, kind);
  }
});

test("the settings read again for what each shows, the folders for the library, the policy and the storage for the titles", () => {
  assert.equal(COLLECTION.match(change("settings", null)), true);
  assert.equal(COLLECTION.match(change("library", null)), false);

  assert.equal(WATCH_FOLDERS.match(change("library", "w1")), true);
  assert.equal(WATCH_FOLDERS.match(change("library", null)), true);
  assert.equal(WATCH_FOLDERS.match(change("settings", null)), false);

  for (const kind of ["settings", "library", "season_info", "anissia"] as const) {
    assert.equal(POLICY.match(change(kind, "x")), true, kind);
  }
  assert.equal(POLICY.match(change("subtitle_store", "w1")), false);

  for (const kind of ["subtitle_store", "subtitle_job", "artwork", "library", "season_info", "anissia"] as const) {
    assert.equal(STORAGE.match(change(kind, "x")), true, kind);
  }
  for (const kind of ["settings", "history", "rules"] as const) {
    assert.equal(STORAGE.match(change(kind, null)), false, kind);
  }
});

test("a collection cycle's burst reads the schedule and the board once more, and the server's deadline event reads them", async () => {
  for (const match of [scheduleChanged, boardChanged]) {
    const s = screen(match);
    s.holdReads();
    s.rereader.start();
    await settle();
    assert.equal(s.log.started, 1);

    // What one cycle announces within milliseconds, none of it new (lighter-structure/0164).
    s.source().change("collect_status", null);
    for (let i = 0; i < 250; i++) s.source().change("history", String(i));
    for (let i = 0; i < 3; i++) s.source().change("collect_status", null);
    await settle();
    assert.equal(s.log.started, 1);
    await s.finish();
    assert.equal(s.log.started, 2);
    await s.finish();
    assert.deepEqual(s.log, { started: 2, ended: 2, concurrent: 0, mostAtOnce: 1 });

    // The server announces the cycle's stall with no write behind it: `collect_status` without an ID.
    s.stopHolding();
    s.source().change("collect_status", null);
    await settle();
    assert.equal(s.log.started, 3);
    s.stop();
  }
});

test("a cover or season info that arrives during a read is read once more after it", async () => {
  const cover = screen((c) => workCoverChanged(c, "w1"));
  cover.holdReads();
  cover.rereader.start();
  await settle();
  for (let i = 0; i < 20; i++) cover.source().change("artwork", "w1");
  cover.source().change("artwork", "w2");
  await settle();
  assert.equal(cover.log.started, 1);
  await cover.finish();
  assert.equal(cover.log.started, 2);
  await cover.finish();
  assert.equal(cover.log.started, 2);
  cover.stop();

  const info = screen((c) => seasonInfoChanged(c, "w1"));
  info.holdReads();
  info.rereader.start();
  await settle();
  for (let i = 0; i < 20; i++) info.source().change("season_info", i % 2 === 0 ? "w1" : null);
  await settle();
  await info.finish();
  assert.equal(info.log.started, 2);
  await info.finish();
  assert.equal(info.log.started, 2);
  info.stop();
});

test("the to-dos read once more when the server announces that the list of torrents got too old", async () => {
  const s = screen(todosChanged);
  s.rereader.start();
  await settle();
  assert.equal(s.log.started, 1);

  // No write is behind it: `collect_status` without an ID (`crates/trss/src/deadlines.rs`).
  s.source().change("collect_status", null);
  await settle();
  assert.equal(s.log.started, 2);
  s.stop();
  s.rereader.stop();
});
