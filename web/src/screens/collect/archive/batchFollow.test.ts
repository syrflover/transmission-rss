import assert from "node:assert/strict";
import { test } from "node:test";

import { createEvents, type EventSourceLike } from "../../../lib/events.ts";
import { CommandWatch } from "../../../lib/watchCommands.ts";
import {
  BatchTracker,
  followBatch,
  MISSES,
  READ_RETRY_MS,
  RESEND_MS,
  RESENDS,
  sendUntilAnswered,
  UNCONFIRMED,
  type Update,
} from "./batchFollow.ts";
import { LOST, type ArchiveBatchView } from "./batchRun.ts";

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
  change(kind: string, id: string) {
    this.say("change", JSON.stringify({ kind, id }));
  }
}

function setup() {
  const sources: FakeSource[] = [];
  const timers: { run: () => void; ms: number; live: boolean }[] = [];
  const events = createEvents({
    open: () => {
      const source = new FakeSource();
      sources.push(source);
      return source;
    },
    visibility: { hidden: () => false, onChange: () => () => {} },
    setTimer: () => () => {},
  });
  const watching = new CommandWatch(events, (run, ms) => {
    const timer = { run, ms, live: true };
    timers.push(timer);
    return () => {
      timer.live = false;
    };
  });
  /** Runs the waits that are due, and returns how long each was. */
  const fire = () => {
    const due = timers.filter((t) => t.live);
    for (const timer of due) {
      timer.live = false;
      timer.run();
    }
    return due.map((t) => t.ms);
  };
  return { watching, sources, fire };
}

const settle = () => new Promise<void>((resolve) => setImmediate(resolve));

const view = (state: ArchiveBatchView["state"], finished: number): ArchiveBatchView => ({
  id: "b1",
  state,
  total: 2,
  finished,
  current: state === "running" ? { rule_id: `r${finished}`, name: `n${finished}` } : null,
  failure: null,
  results: [],
});

function follower(reads: Array<ArchiveBatchView | Error>) {
  const { watching, sources, fire } = setup();
  const seen: ArchiveBatchView[] = [];
  const calls: string[] = [];
  let lost = 0;
  followBatch(
    {
      watching,
      read: async (id) => {
        calls.push(id);
        const next = reads.shift();
        if (next === undefined) throw new Error("no more reads");
        if (next instanceof Error) throw next;
        return next;
      },
      onView: (v) => void seen.push(v),
      onLost: () => void (lost += 1),
    },
    "b1",
  );
  return { sources, fire, seen, calls, lost: () => lost };
}

test("the batch is read at once, and again for each change the server announces for it, until it ended", async () => {
  const f = follower([view("running", 0), view("running", 1), view("done", 2)]);
  await settle();
  assert.deepEqual(f.seen.map((v) => v.finished), [0]);

  f.sources[0].change("command", "other");
  f.sources[0].change("command", "b1");
  await settle();
  assert.deepEqual(f.seen.map((v) => v.finished), [0, 1]);

  f.sources[0].change("command", "b1");
  await settle();
  assert.deepEqual(f.seen.map((v) => v.state), ["running", "running", "done"]);

  // It stopped following: a later change reads nothing.
  f.sources[0].change("command", "b1");
  await settle();
  assert.equal(f.calls.length, 3);
});

test("no read waits on a timer while the batch goes on", async () => {
  const f = follower([view("running", 0)]);
  await settle();
  assert.deepEqual(f.fire(), []);
});

test("a read that fails is made again after a second, and a read that works clears the misses", async () => {
  const f = follower([new Error("down"), new Error("down"), view("running", 0)]);
  await settle();
  assert.deepEqual(f.fire(), [READ_RETRY_MS]);
  await settle();
  assert.deepEqual(f.fire(), [READ_RETRY_MS]);
  await settle();
  assert.deepEqual(f.seen.map((v) => v.state), ["running"]);
  assert.equal(f.lost(), 0);
});

test("the screen gives up on a batch whose reads keep failing, after the same number of misses as before", async () => {
  const f = follower(Array.from({ length: MISSES + 2 }, () => new Error("down")));
  await settle();
  let retries = 0;
  while (f.fire().length > 0) {
    retries += 1;
    await settle();
  }
  assert.equal(retries, MISSES);
  assert.equal(f.calls.length, MISSES + 1);
  assert.equal(f.lost(), 1);
  assert.deepEqual(f.seen, []);
});

test("a stream that says ready reads the batch again, so a change missed while the tab was hidden is not lost", async () => {
  const f = follower([view("running", 0), view("done", 2)]);
  await settle();

  f.sources[0].say("ready");
  await settle();

  assert.deepEqual(f.seen.map((v) => v.state), ["running", "done"]);
});

test("a request is sent once when the server answers, and again with the same call when the answer is lost", async () => {
  let sent = 0;
  const waits: number[] = [];
  const result = await sendUntilAnswered(
    async () => {
      sent += 1;
      if (sent < 3) throw new Error("network");
      return "batch-1";
    },
    () => true,
    String,
    async (ms) => void waits.push(ms),
  );
  assert.deepEqual(result, { ok: true, value: "batch-1" });
  assert.equal(sent, 3);
  assert.deepEqual(waits, [RESEND_MS, RESEND_MS]);
});

test("a request the server refused is not sent again, and says the server's sentence", async () => {
  let sent = 0;
  const result = await sendUntilAnswered(
    async () => {
      sent += 1;
      throw new Error("다른 일괄 보관이 진행 중이에요.");
    },
    () => false,
    (e) => (e as Error).message,
    async () => {},
  );
  assert.deepEqual(result, { ok: false, message: "다른 일괄 보관이 진행 중이에요." });
  assert.equal(sent, 1);
});

test("a request nobody answered is sent again a few times, then reported as unconfirmed", async () => {
  let sent = 0;
  const result = await sendUntilAnswered(
    async () => {
      sent += 1;
      throw new Error("network");
    },
    () => true,
    String,
    async () => {},
  );
  assert.deepEqual(result, { ok: false, message: UNCONFIRMED });
  assert.equal(sent, RESENDS + 1);
});

function tracking(reads: Array<ArchiveBatchView | Error>) {
  const { watching, sources, fire } = setup();
  const log: string[] = [];
  const tracker = new BatchTracker({
    watching,
    read: async () => {
      const next = reads.shift();
      if (next === undefined) throw new Error("no more reads");
      if (next instanceof Error) throw next;
      return next;
    },
    gone: (ruleId) => void log.push(`gone ${ruleId}`),
    archived: () => void log.push("cache archived"),
  });
  tracker.setHooks({
    archived: () => void log.push("screen archived"),
    reload: () => void log.push("screen reload"),
  });
  const updates: Update[] = [];
  const entries = [
    { rule_id: "r0", name: "n0" },
    { rule_id: "r1", name: "n1" },
  ];
  return { tracker, sources, fire, log, updates, entries };
}

test("a batch goes on being followed after the screen that sent it is gone, and the caches are put right at its end", async () => {
  const t = tracking([view("running", 0), view("running", 1), view("done", 2)]);
  const stop = t.tracker.subscribe((update) => void t.updates.push(update));
  await t.tracker.start(t.entries, async () => ({ ok: true, value: "b1" }));
  await settle();
  assert.equal(t.tracker.active, true);

  // The screen closes: nothing listens, and the batch still moves on.
  stop();
  const before = t.updates.length;
  t.sources[0].change("command", "b1");
  await settle();
  assert.deepEqual(t.log, ["gone r0"]);
  t.sources[0].change("command", "b1");
  await settle();

  assert.deepEqual(t.log, ["gone r0", "gone r1", "cache archived", "screen archived", "screen reload"]);
  assert.equal(t.updates.length, before);
  assert.equal(t.tracker.active, false);
  assert.deepEqual(t.tracker.run, { total: 2, finished: 2, current: null, error: null, results: [], errorWorkId: null });
});

test("a screen that opens during a batch is shown it, told when it ends, and finds the run it left", async () => {
  const t = tracking([view("running", 0), view("failed", 1)]);
  t.tracker.adopt(view("running", 0));
  const stop = t.tracker.subscribe((update) => void t.updates.push(update));
  assert.deepEqual(t.tracker.run, { total: 2, finished: 0, current: "n0", error: null, results: [], errorWorkId: null });
  // A second batch is not started or adopted beside it.
  await t.tracker.start(t.entries, async () => assert.fail("a batch is active"));
  t.tracker.adopt(view("running", 1));
  await settle();

  t.sources[0].change("command", "b1");
  await settle();

  // The first update is the read made when it began to follow the batch.
  assert.deepEqual(t.updates.map((u) => u.ended), [false, true]);
  assert.equal(t.updates[1].run?.finished, 1);
  assert.equal(t.updates[1].run?.current, null);
  assert.match(t.updates[1].run?.error ?? "", /옮기지 못했어요|일괄 보관이/);
  // One rule moved on from the one that was in turn: it is named; the caches learn something was archived.
  assert.deepEqual(t.log, ["gone r0", "cache archived", "screen archived", "screen reload"]);
  stop();
  assert.equal(t.tracker.run?.finished, 1);
  t.tracker.dismiss();
  assert.equal(t.tracker.run, null);
});

test("a request the server refused ends the run at once and reads the suggestions again", async () => {
  const t = tracking([]);
  t.tracker.subscribe((update) => void t.updates.push(update));
  await t.tracker.start(t.entries, async () => ({ ok: false, message: "다른 일괄 보관이 진행 중이에요." }));
  assert.equal(t.tracker.active, false);
  assert.deepEqual(t.tracker.run, {
    total: 2,
    finished: 0,
    current: null,
    error: "다른 일괄 보관이 진행 중이에요.",
    results: [],
    errorWorkId: null,
  });
  assert.deepEqual(t.log, ["screen reload"]);
  assert.deepEqual(t.updates.map((u) => u.ended), [false, true]);
});

test("a batch given up on after what it finished is read says so and still cleans up what it archived", async () => {
  const t = tracking([view("running", 1), ...Array.from({ length: MISSES + 1 }, () => new Error("down"))]);
  await t.tracker.start(t.entries, async () => ({ ok: true, value: "b1" }));
  await settle();
  assert.deepEqual(t.log, ["gone r0"]);

  t.sources[0].change("command", "b1");
  await settle();
  while (t.fire().length > 0) await settle();

  assert.equal(t.tracker.active, false);
  assert.equal(t.tracker.run?.error, LOST);
  assert.deepEqual(t.log, ["gone r0", "cache archived", "screen archived", "screen reload"]);
});

test("a batch given up on before anything finished only reads the suggestions again", async () => {
  const t = tracking(Array.from({ length: MISSES + 1 }, () => new Error("down")));
  await t.tracker.start(t.entries, async () => ({ ok: true, value: "b1" }));
  await settle();
  while (t.fire().length > 0) await settle();
  assert.deepEqual(t.log, ["screen reload"]);
  assert.equal(t.tracker.active, false);
});
