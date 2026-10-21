import assert from "node:assert/strict";
import { test } from "node:test";

import { createEvents, REOPEN_MS, SILENCE_MS, type EventSourceLike, type Subscriber } from "./events.ts";

/** A stream the test drives: it says what the server would. */
class FakeSource implements EventSourceLike {
  readyState = 1;
  closed = false;
  private readonly listeners = new Map<string, ((event: { data: string }) => void)[]>();

  addEventListener(type: string, listener: (event: { data: string }) => void) {
    this.listeners.set(type, [...(this.listeners.get(type) ?? []), listener]);
  }

  close() {
    this.closed = true;
    this.readyState = 2;
  }

  say(type: string, data = "{}") {
    for (const listener of this.listeners.get(type) ?? []) listener({ data });
  }

  change(kind: string, id: string | null) {
    this.say("change", JSON.stringify({ kind, id }));
  }
}

/** The page's visibility, which the test flips. */
class FakeVisibility {
  isHidden = false;
  listeners = new Set<() => void>();
  hidden = () => this.isHidden;
  onChange = (listener: () => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  set(hidden: boolean) {
    this.isHidden = hidden;
    for (const listener of [...this.listeners]) listener();
  }
}

/** The timers the test runs by hand. */
class FakeTimers {
  pending: { run: () => void; ms: number; live: boolean }[] = [];
  setTimer = (run: () => void, ms: number) => {
    const timer = { run, ms, live: true };
    this.pending.push(timer);
    return () => {
      timer.live = false;
    };
  };

  /** The delays of the timers that wait. */
  waiting() {
    return this.pending.filter((t) => t.live).map((t) => t.ms);
  }

  fire() {
    const due = this.pending.filter((t) => t.live);
    this.pending = [];
    for (const timer of due) timer.run();
    return due.map((t) => t.ms);
  }
}

function setup(options: { hidden?: boolean } = {}) {
  const visibility = new FakeVisibility();
  visibility.isHidden = options.hidden ?? false;
  const timers = new FakeTimers();
  const sources: FakeSource[] = [];
  const events = createEvents({
    open: () => {
      const source = new FakeSource();
      sources.push(source);
      return source;
    },
    visibility,
    setTimer: timers.setTimer,
  });
  return { events, sources, visibility, timers };
}

/** A subscriber that notes what it was told to read again. */
function listener(accepts: (kind: string, id: string | null) => boolean = () => true) {
  const rereads: number[] = [];
  const subscriber: Subscriber = {
    match: (change) => accepts(change.kind, change.id),
    reread: () => void rereads.push(rereads.length + 1),
  };
  return { subscriber, rereads };
}

test("nothing is opened until somebody listens, and one connection serves every listener", () => {
  const { events, sources } = setup();
  assert.equal(sources.length, 0);

  const a = listener();
  const b = listener();
  events.subscribe(a.subscriber);
  events.subscribe(b.subscriber);

  assert.equal(sources.length, 1);
});

test("a change with no ID is passed on with a null ID, and its listener decides what it is about", () => {
  const { events, sources } = setup();
  const seen: (string | null)[] = [];
  const ofOneJob = listener((kind, id) => {
    seen.push(id);
    return kind === "subtitle_job" && (id === "j1" || id === null);
  });
  const ofAnotherJob = listener((kind, id) => kind === "subtitle_job" && id === "j2");
  events.subscribe(ofOneJob.subscriber);
  events.subscribe(ofAnotherJob.subscriber);

  sources[0].change("subtitle_job", null);
  sources[0].change("subtitle_job", "j1");

  assert.deepEqual(seen, [null, "j1"]);
  assert.equal(ofOneJob.rereads.length, 2);
  assert.equal(ofAnotherJob.rereads.length, 0);
});

test("the last listener to leave closes the connection, and the next one opens a new one", () => {
  const { events, sources } = setup();
  const a = listener();
  const b = listener();
  const stopA = events.subscribe(a.subscriber);
  const stopB = events.subscribe(b.subscriber);

  stopA();
  assert.equal(sources[0].closed, false);
  stopB();
  assert.equal(sources[0].closed, true);

  events.subscribe(listener().subscriber);
  assert.equal(sources.length, 2);
  assert.equal(sources[1].closed, false);
});

test("a page that is hidden opens nothing, and one that hides closes the connection", () => {
  const { events, sources, visibility } = setup({ hidden: true });
  events.subscribe(listener().subscriber);
  assert.equal(sources.length, 0);

  visibility.set(false);
  assert.equal(sources.length, 1);

  visibility.set(true);
  assert.equal(sources[0].closed, true);
});

test("a page shown again opens one connection and its ready reads everything again", () => {
  const { events, sources, visibility } = setup();
  const a = listener();
  const b = listener((kind) => kind === "never");
  events.subscribe(a.subscriber);
  events.subscribe(b.subscriber);
  sources[0].say("ready");
  assert.deepEqual([a.rereads.length, b.rereads.length], [1, 1]);

  visibility.set(true);
  // Hidden twice in a row, or shown twice, opens nothing more.
  visibility.set(true);
  assert.equal(sources.length, 1);
  visibility.set(false);
  visibility.set(false);
  assert.equal(sources.length, 2);
  assert.equal(sources[1].closed, false);
  assert.deepEqual([a.rereads.length, b.rereads.length], [1, 1]);

  sources[1].say("ready");
  assert.deepEqual([a.rereads.length, b.rereads.length], [2, 2]);
});

test("a change reaches only the listeners it matches", () => {
  const { events, sources } = setup();
  const command = listener((kind, id) => kind === "command" && id === "c1");
  const search = listener((kind) => kind === "past_search");
  events.subscribe(command.subscriber);
  events.subscribe(search.subscriber);

  sources[0].change("command", "c1");
  sources[0].change("command", "c2");
  sources[0].change("past_search", "c1");

  assert.equal(command.rereads.length, 1);
  assert.equal(search.rereads.length, 1);
});

test("lagged reads everything again, and a change that cannot be read counts as one that was missed", () => {
  const { events, sources } = setup();
  const never = listener(() => false);
  events.subscribe(never.subscriber);

  sources[0].say("lagged");
  assert.equal(never.rereads.length, 1);

  sources[0].say("change", "not json");
  assert.equal(never.rereads.length, 2);
});

test("a listener that left is not told, and one that leaves while being told does not stop the others", () => {
  const { events, sources } = setup();
  const stayer = listener();
  const leaver = listener();
  let stopLeaver = () => {};
  const first: Subscriber = {
    match: () => true,
    reread: () => stopLeaver(),
  };
  events.subscribe(first);
  stopLeaver = events.subscribe(leaver.subscriber);
  events.subscribe(stayer.subscriber);

  sources[0].say("ready");

  assert.equal(leaver.rereads.length, 0);
  assert.equal(stayer.rereads.length, 1);
});

test("a stream the browser gave up on is opened again after a while, not at once", () => {
  const { events, sources, timers } = setup();
  const a = listener();
  events.subscribe(a.subscriber);

  // A drop the browser mends itself (it is connecting again): nothing to do.
  sources[0].readyState = 0;
  sources[0].say("error");
  assert.deepEqual(timers.waiting(), [SILENCE_MS]);
  assert.equal(sources.length, 1);

  // A refusal: the browser closes the stream and does not try again.
  sources[0].readyState = 2;
  sources[0].say("error");
  assert.equal(sources.length, 1);
  assert.deepEqual(timers.fire(), [REOPEN_MS]);
  assert.equal(sources.length, 2);

  sources[1].say("ready");
  assert.equal(a.rereads.length, 1);
});

test("a reopen that waits ends with the last listener, and a stream that was closed says nothing more", () => {
  const { events, sources, timers } = setup();
  const a = listener();
  const stop = events.subscribe(a.subscriber);
  sources[0].readyState = 2;
  sources[0].say("error");

  stop();
  assert.deepEqual(timers.fire(), []);
  assert.equal(sources.length, 1);

  events.subscribe(a.subscriber);
  assert.equal(sources.length, 2);
  // The old stream's late events are not heard.
  sources[0].say("ready");
  assert.equal(a.rereads.length, 0);
  sources[1].say("ready");
  assert.equal(a.rereads.length, 1);
});

test("a stream that says nothing for the silence time is closed and replaced at once, and its successor's ready reads again", () => {
  const { events, sources, timers } = setup();
  const a = listener();
  events.subscribe(a.subscriber);
  sources[0].say("ready");
  assert.equal(a.rereads.length, 1);
  assert.deepEqual(timers.waiting(), [SILENCE_MS]);

  // Nothing came: the connection is dead although the browser calls it open.
  assert.deepEqual(timers.fire(), [SILENCE_MS]);

  assert.equal(sources[0].closed, true);
  assert.equal(sources.length, 2);
  assert.deepEqual(timers.waiting(), [SILENCE_MS]);
  // What the dead stream might still say is not heard.
  sources[0].say("ready");
  assert.equal(a.rereads.length, 1);
  sources[1].say("ready");
  assert.equal(a.rereads.length, 2);
});

test("a stream that keeps saying alive is kept, and any event sets the watchdog again", () => {
  const { events, sources, timers } = setup();
  const a = listener((kind) => kind === "command");
  events.subscribe(a.subscriber);

  for (const say of [
    () => sources[0].say("ready"),
    () => sources[0].say("alive"),
    () => sources[0].change("past_search", "x"),
    () => sources[0].say("alive"),
    () => sources[0].say("lagged"),
  ]) {
    say();
    // One watchdog waits, the one the last event set; the ones before are cancelled.
    assert.deepEqual(timers.waiting(), [SILENCE_MS]);
  }
  assert.equal(sources.length, 1);
  assert.equal(sources[0].closed, false);
  // `alive` itself reads nothing: only ready and lagged did.
  assert.equal(a.rereads.length, 2);
});

test("the watchdog ends with the connection: a closed stream or a given-up one leaves none waiting", () => {
  const { events, sources, timers, visibility } = setup();
  const stop = events.subscribe(listener().subscriber);
  stop();
  assert.deepEqual(timers.waiting(), []);

  events.subscribe(listener().subscriber);
  visibility.set(true);
  assert.deepEqual(timers.waiting(), []);
  visibility.set(false);
  assert.deepEqual(timers.waiting(), [SILENCE_MS]);

  // Given up on by the browser: the reopen waits, the watchdog does not.
  const last = sources[sources.length - 1];
  last.readyState = 2;
  last.say("error");
  assert.deepEqual(timers.waiting(), [REOPEN_MS]);
});

test("a listener that throws does not keep the others from being told", () => {
  const { events, sources } = setup();
  const before = listener();
  const throwsInMatch: Subscriber = {
    match: () => {
      throw new Error("match failed");
    },
    reread: () => {},
  };
  const throwsInReread: Subscriber = {
    match: () => true,
    reread: () => {
      throw new Error("reread failed");
    },
  };
  const after = listener();
  events.subscribe(before.subscriber);
  events.subscribe(throwsInMatch);
  events.subscribe(throwsInReread);
  events.subscribe(after.subscriber);

  sources[0].change("command", "c1");
  sources[0].say("ready");

  assert.equal(before.rereads.length, 2);
  assert.equal(after.rereads.length, 2);
});
