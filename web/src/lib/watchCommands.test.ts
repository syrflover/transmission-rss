import assert from "node:assert/strict";
import { test } from "node:test";

import { createEvents, type EventSourceLike } from "./events.ts";
import { CommandWatch, isCommand, RETRY_MS } from "./watchCommands.ts";

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

function setup() {
  const sources: FakeSource[] = [];
  const timers: { run: () => void; ms: number; live: boolean }[] = [];
  const setTimer = (run: () => void, ms: number) => {
    const timer = { run, ms, live: true };
    timers.push(timer);
    return () => {
      timer.live = false;
    };
  };
  const events = createEvents({
    open: () => {
      const source = new FakeSource();
      sources.push(source);
      return source;
    },
    visibility: { hidden: () => false, onChange: () => () => {} },
    // The stream's watchdog never runs here: these tests are about the reads.
    setTimer: () => () => {},
  });
  const watch = new CommandWatch(events, setTimer);
  const fire = () => {
    const due = timers.filter((t) => t.live);
    for (const timer of due) {
      timer.live = false;
      timer.run();
    }
    return due.map((t) => t.ms);
  };
  return { watch, sources, fire };
}

/** Lets the reads that were started run to their end. */
const settle = () => new Promise<void>((resolve) => setImmediate(resolve));

test("a command is read as soon as it is followed, and again for each change of it", async () => {
  const { watch, sources } = setup();
  const reads: string[] = [];

  watch.watch("c1", () => void reads.push("c1"));
  await settle();
  assert.deepEqual(reads, ["c1"]);

  sources[0].change("command", "c1");
  await settle();
  sources[0].change("command", "other");
  sources[0].change("past_search", "c1");
  // A change that names no ID is about no command in particular.
  sources[0].change("command", null);
  await settle();
  assert.deepEqual(reads, ["c1", "c1"]);
});

test("the stream is listened to before the first read, so a change during it is not lost", async () => {
  const { watch, sources } = setup();
  const reads: string[] = [];
  let release = () => {};
  let streamsAtFirstRead: number | null = null;

  watch.watch("c1", async () => {
    reads.push("start");
    streamsAtFirstRead ??= sources.length;
    await new Promise<void>((resolve) => (release = resolve));
    reads.push("end");
  });
  await settle();
  sources[0].change("command", "c1");
  sources[0].change("command", "c1");
  release();
  await settle();
  release();
  await settle();

  // The stream was open and heard before the first read began.
  assert.equal(streamsAtFirstRead, 1);
  // The changes that came during the read make one more read after it, not two beside it.
  assert.deepEqual(reads, ["start", "end", "start", "end"]);
});

test("a stream that says ready or lagged reads every followed command again", async () => {
  const { watch, sources } = setup();
  const reads: string[] = [];
  watch.watch("c1", () => void reads.push("c1"));
  watch.watch("c2", () => void reads.push("c2"));
  await settle();
  reads.length = 0;

  sources[0].say("ready");
  await settle();
  sources[0].say("lagged");
  await settle();

  assert.deepEqual(reads.sort(), ["c1", "c1", "c2", "c2"]);
});

test("following a command again does nothing, and a command that is let go is no longer read", async () => {
  const { watch, sources } = setup();
  const reads: string[] = [];
  watch.watch("c1", () => void reads.push("first"));
  watch.watch("c1", () => void reads.push("second"));
  await settle();
  assert.deepEqual(reads, ["first"]);

  watch.unwatch("c1");
  sources[0].change("command", "c1");
  await settle();
  assert.deepEqual(reads, ["first"]);
  // Nobody listens any more: the connection is closed.
  assert.equal(sources[0].readyState, 2);
});

test("a read that failed is tried again after the retry time, once", async () => {
  const { watch, fire } = setup();
  const reads: string[] = [];
  watch.watch("c1", () => void reads.push("first"));
  await settle();

  watch.retry("c1", () => void reads.push("retry-a"));
  watch.retry("c1", () => void reads.push("retry-b"));
  assert.deepEqual(reads, ["first"]);
  assert.deepEqual(fire(), [RETRY_MS]);
  await settle();
  assert.deepEqual(reads, ["first", "retry-b"]);
  assert.deepEqual(fire(), []);
});

test("a screen that has its own retry time gets it, and a later retry replaces the delay too", async () => {
  const { watch, fire } = setup();
  const reads: string[] = [];

  watch.retry("c1", () => void reads.push("slow"), 5000);
  watch.retry("c1", () => void reads.push("quick"), 1000);

  assert.deepEqual(fire(), [1000]);
  await settle();
  assert.deepEqual(reads, ["quick"]);
});

test("a retry needs no watch, and stopping forgets it and every followed command", async () => {
  const { watch, sources, fire } = setup();
  const reads: string[] = [];

  watch.retry("lost", () => void reads.push("lost"));
  assert.equal(sources.length, 0);
  assert.deepEqual(fire(), [RETRY_MS]);
  await settle();
  assert.deepEqual(reads, ["lost"]);

  watch.watch("c1", () => void reads.push("c1"));
  watch.retry("c1", () => void reads.push("retry"));
  await settle();
  watch.stop();
  assert.deepEqual(fire(), []);
  sources[0].change("command", "c1");
  await settle();
  assert.deepEqual(reads, ["lost", "c1"]);
  assert.equal(sources[0].readyState, 2);
});

test("sync follows the commands listed and lets the others go", async () => {
  const { watch, sources } = setup();
  const reads: string[] = [];
  const read = (id: string) => void reads.push(id);

  watch.sync(["a", "b"], read);
  await settle();
  assert.deepEqual(reads.sort(), ["a", "b"]);
  reads.length = 0;

  watch.sync(["b", "c"], read);
  await settle();
  // Only the new one is read now; the one that stays is not read again.
  assert.deepEqual(reads, ["c"]);
  reads.length = 0;

  sources[0].change("command", "a");
  sources[0].change("command", "b");
  await settle();
  assert.deepEqual(reads, ["b"]);

  watch.sync([], read);
  assert.equal(sources[0].readyState, 2);
});

test("a read that throws does not stop the following", async () => {
  const { watch, sources } = setup();
  let reads = 0;
  watch.watch("c1", () => {
    reads += 1;
    throw new Error("the server is down");
  });
  await settle();
  sources[0].change("command", "c1");
  await settle();
  assert.equal(reads, 2);
});

test("a change is about a command when its kind and ID are", () => {
  assert.equal(isCommand({ kind: "command", id: "c1" }, "c1"), true);
  assert.equal(isCommand({ kind: "command", id: "c2" }, "c1"), false);
  assert.equal(isCommand({ kind: "past_search", id: "c1" }, "c1"), false);
  // A change with no ID says that anything of its kind changed, which is no
  // command in particular.
  assert.equal(isCommand({ kind: "command", id: null }, "c1"), false);
});
