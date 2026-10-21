import assert from "node:assert/strict";
import { test } from "node:test";

import { jobChanged, jobListChanged, suggestionsChanged, todosChanged } from "../screens/todo/listens.ts";
import { applyChanged, candidatesChanged, cleanupChanged } from "../screens/library/detail/listens.ts";
import type { Change } from "./api-types";
import { createEvents, type EventSourceLike } from "./events.ts";
import { createRereader, type RereaderOptions } from "./rereads.ts";

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

  // What a collection cycle and a subtitle job announce within milliseconds (docs/tickets/0164-db-change-events.md).
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

test("the to-dos read again for the kinds they are gathered from, and not for a remote screen or the board", () => {
  for (const kind of [
    "subtitle_job",
    "subtitle_store",
    "subtitle_mapping",
    "library",
    "artwork",
    "history",
    "rules",
    "anissia",
    "settings",
  ] as const) {
    assert.equal(todosChanged(change(kind, null)), true, kind);
  }
  for (const kind of ["subtitle_screen", "collect_status", "command", "past_search", "season_info"] as const) {
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
