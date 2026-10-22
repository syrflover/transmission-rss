import type { CommandWatch } from "../../../lib/watchCommands.ts";

import {
  isOpen,
  LOST,
  newlyFinished,
  runOfView,
  seenOf,
  type ArchiveBatchView,
  type Run,
  type Seen,
} from "./batchRun.ts";

/**
 * How the 구독 tab follows a batch of `보관` the server runs, and sends it,
 * without asking the server on a timer: it reads the batch's view when it
 * starts to follow it and again whenever the event stream says the batch
 * changed (`lib/watchCommands.ts`). It has no `@/` imports so it runs under
 * `node --test`; `api.ts` gives it the requests and `ArchiveSuggestions.tsx`
 * the screen.
 */

/** How long to wait before reading again a batch whose last read failed, in milliseconds. */
export const READ_RETRY_MS = 1000;
/** How many reads of the batch may fail in a row before the screen gives up on it. */
export const MISSES = 10;
/** How long a request that got no answer waits before it is sent again, in milliseconds. */
export const RESEND_MS = 2000;
/** How many times a request that got no answer is sent again (with the same ID) before giving up. */
export const RESENDS = 4;

/** Said when a request got no answer however often it was sent. */
export const UNCONFIRMED = "접수됐는지 확인하지 못했어요. 규칙의 상태를 확인해 주세요.";

export interface Follow {
  watching: CommandWatch;
  /** Reads the view of the batch `id`. */
  read: (id: string) => Promise<ArchiveBatchView>;
  /** Called with each view read, the last one included. */
  onView: (view: ArchiveBatchView) => void;
  /** Called once the reads failed so often that the screen gives up on the batch. */
  onLost: () => void;
}

/**
 * Follows the batch `id` until it has ended or the reads keep failing: reads
 * it now and whenever the server says it changed (one read at a time), and a
 * read that failed is made again after {@link READ_RETRY_MS}. The screen
 * calls `watching.unwatch(id)` (or stops `watching`) to stop earlier.
 */
export function followBatch(deps: Follow, id: string): void {
  const { watching } = deps;
  let misses = 0;
  const reread = async () => {
    let view: ArchiveBatchView;
    try {
      view = await deps.read(id);
    } catch {
      misses += 1;
      if (misses > MISSES) {
        watching.unwatch(id);
        deps.onLost();
      } else {
        watching.retry(id, reread, READ_RETRY_MS);
      }
      return;
    }
    misses = 0;
    if (!isOpen(view)) watching.unwatch(id);
    deps.onView(view);
  };
  watching.watch(id, reread);
}

/** How a request came out: the answer, or the sentence for why there is none. */
export type Sent<T> = { ok: true; value: T } | { ok: false; message: string };

/**
 * Sends a request through the web command contract: one command ID for the
 * action, so `send` (which sends it with that ID) can be repeated unchanged
 * when the answer was lost (the server stores a command once per ID). A
 * failure `resendable` says is one of those is sent again after
 * {@link RESEND_MS}, up to {@link RESENDS} times; any other failure is the
 * server's answer and is not repeated.
 */
export async function sendUntilAnswered<T>(
  send: () => Promise<T>,
  resendable: (error: unknown) => boolean,
  messageOf: (error: unknown) => string,
  wait: (ms: number) => Promise<void>,
): Promise<Sent<T>> {
  for (let attempt = 0; ; attempt += 1) {
    try {
      return { ok: true, value: await send() };
    } catch (e) {
      if (!resendable(e)) return { ok: false, message: messageOf(e) };
      if (attempt >= RESENDS) return { ok: false, message: UNCONFIRMED };
      await wait(RESEND_MS);
    }
  }
}

/** What the tracker tells a screen: the run to show now, and whether the batch just ended. */
export interface Update {
  /** `null` once the screen dismissed the run. */
  run: Run | null;
  /** The batch ended (or was given up on) with this update; the screen puts its own state right. */
  ended: boolean;
}

export interface TrackerDeps {
  watching: CommandWatch;
  /** Reads the view of the batch `id`. */
  read: (id: string) => Promise<ArchiveBatchView>;
  /** A rule the batch archived leaves the cached suggestions. */
  gone: (ruleId: string) => void;
  /** Something was archived: the cached lists that show rules read the server again. */
  archived: () => void;
}

/** What the screen that asked for a batch does at its end; kept up to date by the screen. */
export interface TrackerHooks {
  /** The screen's parent lists read again. */
  archived: () => void;
  /** The screen's own suggestions read again. */
  reload: () => void;
}

interface Tracked {
  /** The rules it was sent with, in order; `null` for a batch found running when a screen opened. */
  known: string[] | null;
  seen: Seen;
}

/**
 * The one batch of `보관` this tab follows, kept outside the screen so that
 * closing the screen does not stop the follow: the server goes on with the
 * batch whether or not a screen shows it, and what it archived must still
 * leave the cached lists when it ends. A screen shows {@link BatchTracker.run}
 * and listens to {@link BatchTracker.subscribe}; only its own state is
 * skipped once it is gone, because it no longer listens.
 */
export class BatchTracker {
  private readonly deps: TrackerDeps;
  private hooks: TrackerHooks = { archived: () => {}, reload: () => {} };
  private readonly listeners = new Set<(update: Update) => void>();
  private shown: Run | null = null;
  /** A batch is being sent, or followed. */
  private busy = false;
  private tracked: Tracked | null = null;

  constructor(deps: TrackerDeps) {
    this.deps = deps;
  }

  /** What the screen that is shown now does at the end of a batch. */
  setHooks(hooks: TrackerHooks): void {
    this.hooks = hooks;
  }

  /** The run to show: a batch in progress, how the last one ended, or none. */
  get run(): Run | null {
    return this.shown;
  }

  /** Whether a batch is being sent or followed, so another one must not be started. */
  get active(): boolean {
    return this.busy;
  }

  /** Calls `listener` with each update until the returned function is called. */
  subscribe(listener: (update: Update) => void): () => void {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  }

  /** The screen closed the run it showed. */
  dismiss(): void {
    if (this.busy) return;
    this.shown = null;
    this.notify(false);
  }

  /**
   * Sends the rules (`rules` holds their IDs and names, in order) with `send`
   * and follows the batch it starts until it ends, even when no screen listens
   * by then. Does nothing while another batch is active.
   */
  async start(rules: { rule_id: string; name: string }[], send: () => Promise<Sent<string>>): Promise<void> {
    if (this.busy || rules.length === 0) return;
    this.busy = true;
    const total = rules.length;
    this.shown = { total, finished: 0, current: rules[0].name, error: null, results: [], errorWorkId: null };
    this.notify(false);
    let sent: Sent<string>;
    try {
      sent = await send();
    } catch (e) {
      sent = { ok: false, message: e instanceof Error ? e.message : String(e) };
    }
    if (!sent.ok) {
      this.shown = { total, finished: 0, current: null, error: sent.message, results: [], errorWorkId: null };
      this.busy = false;
      this.finish(0);
      return;
    }
    this.tracked = { known: rules.map((rule) => rule.rule_id), seen: { finished: 0, current: null } };
    this.follow(sent.value);
  }

  /** Follows a batch the server was running when a screen opened; does nothing while another is active. */
  adopt(view: ArchiveBatchView): void {
    if (this.busy || !isOpen(view)) return;
    this.busy = true;
    this.tracked = { known: null, seen: seenOf(view) };
    this.shown = runOfView(view);
    this.notify(false);
    this.follow(view.id);
  }

  private follow(id: string): void {
    followBatch(
      {
        watching: this.deps.watching,
        read: this.deps.read,
        onView: (view) => this.see(view),
        onLost: () => {
          const finished = this.tracked?.seen.finished ?? 0;
          this.tracked = null;
          this.busy = false;
          this.shown = this.shown && { ...this.shown, current: null, error: LOST };
          this.finish(finished);
        },
      },
      id,
    );
  }

  private see(view: ArchiveBatchView): void {
    const following = this.tracked;
    if (following === null) return;
    for (const ruleId of newlyFinished(following.known, following.seen, view)) this.deps.gone(ruleId);
    following.seen = seenOf(view);
    this.shown = runOfView(view);
    if (isOpen(view)) {
      this.notify(false);
      return;
    }
    this.tracked = null;
    this.busy = false;
    this.finish(view.finished);
  }

  /** The batch is over: what it archived is out of the lists that show it, and the screen is told. */
  private finish(finished: number): void {
    if (finished > 0) {
      this.deps.archived();
      this.hooks.archived();
    }
    this.hooks.reload();
    this.notify(true);
  }

  private notify(ended: boolean): void {
    for (const listener of [...this.listeners]) listener({ run: this.shown, ended });
  }
}
