import type { Change } from "./api-types";

/**
 * The one connection a tab keeps to the server's event stream (`GET
 * /api/events`, `src/web/events_api.rs`), and what the screens that listen to it
 * are told. It has no `@/` imports, React or browser globals so it runs under
 * `node --test`; `sharedEvents.ts` gives it the browser's, and `useChanges.ts`
 * is the hook the screens use.
 *
 * The stream only says *that* something changed (a kind and an ID); a screen
 * reads again through the route it always read from. The ID is `null` when the
 * changed row names no target of that kind: anything of the kind may have
 * changed, so a screen that shows the kind and decides by ID reads again for a
 * `null` one too. A screen that listens
 * registers a {@link Subscriber}:
 *
 * - `match(change)` says whether a change is about what the screen shows, and
 *   `reread()` then reads it again.
 * - `reread()` is also called for every subscriber when the stream says `ready`
 *   (a connection was made, so whatever changed before it is not heard) and
 *   when it says `lagged` (it fell behind and missed changes). That is why a
 *   missed event cannot leave a screen on an old state.
 *
 * The connection is open only while the page is visible and somebody listens:
 * a hidden tab closes it (a browser opens about six connections to a server, and
 * a tab out of sight must not hold one), and opens it again when it is shown,
 * which ends in `ready` and so in one read of everything shown.
 *
 * The server says `alive` while nothing else happens. A page never sees a
 * dead connection that was not closed (a laptop that slept through a server
 * restart, a NAT that forgot it): the browser keeps it open and silent. So
 * every event, `alive` included, sets a watchdog, and a stream that says
 * nothing for {@link SILENCE_MS} is closed and opened again at once, which ends in
 * `ready`.
 */

/** A listener to the changes. */
export interface Subscriber {
  /** Whether the change is about something the listener shows. */
  match: (change: Change) => boolean;
  /** Reads again what the listener shows. May start a request; its outcome is the listener's own. */
  reread: () => void;
}

/** What of the browser's `EventSource` the connection uses. */
export interface EventSourceLike {
  addEventListener(type: string, listener: (event: { data: string }) => void): void;
  close(): void;
  /** 0 connecting, 1 open, 2 closed (the browser gave up: it does not open the stream again by itself). */
  readonly readyState: number;
}

/** Whether the page is in view, and when that changes. */
export interface Visibility {
  hidden(): boolean;
  /** Calls `listener` whenever the page is hidden or shown; returns how to stop. */
  onChange(listener: () => void): () => void;
}

export interface EventsDeps {
  /** Opens a stream. */
  open: () => EventSourceLike;
  visibility: Visibility;
  /** Runs `run` after `ms` milliseconds; returns how to cancel it. */
  setTimer?: (run: () => void, ms: number) => () => void;
}

export interface Events {
  /** Starts telling `subscriber` about the changes; returns how to stop. */
  subscribe(subscriber: Subscriber): () => void;
}

/** How long to wait before opening a stream the browser gave up on, in milliseconds. */
export const REOPEN_MS = 5000;

/**
 * How long a stream may say nothing before it counts as dead, in milliseconds:
 * three of the server's `alive` intervals (15 s).
 */
export const SILENCE_MS = 45_000;

/** `EventSource.CLOSED`. */
const CLOSED = 2;

const defaultTimer = (run: () => void, ms: number) => {
  const timer = setTimeout(run, ms);
  return () => clearTimeout(timer);
};

export function createEvents(deps: EventsDeps): Events {
  const setTimer = deps.setTimer ?? defaultTimer;
  const subscribers = new Set<Subscriber>();
  let source: EventSourceLike | null = null;
  let stopVisibility: (() => void) | null = null;
  let cancelReopen: (() => void) | null = null;
  let cancelWatchdog: (() => void) | null = null;

  const each = (run: (subscriber: Subscriber) => void) => {
    // A copy, and a check: a read may end a listener's interest and so unsubscribe it.
    for (const subscriber of [...subscribers]) {
      if (!subscribers.has(subscriber)) continue;
      try {
        run(subscriber);
      } catch {
        // One listener failing must not leave the others untold.
      }
    }
  };
  const resync = () => each((s) => s.reread());

  const close = () => {
    cancelReopen?.();
    cancelReopen = null;
    cancelWatchdog?.();
    cancelWatchdog = null;
    source?.close();
    source = null;
  };

  const connect = () => {
    const mine = deps.open();
    source = mine;
    // Only the stream that is current is heard: one that was closed says nothing more.
    const heard =
      (run: (data: string) => void, showsLife = true) =>
      (event: { data: string }) => {
        if (source !== mine) return;
        if (showsLife) watch();
        run(event.data);
      };
    // Whatever the stream says, even `alive`, shows it is not dead; one that
    // says nothing for too long is replaced.
    const watch = () => {
      cancelWatchdog?.();
      cancelWatchdog = setTimer(() => {
        cancelWatchdog = null;
        if (source !== mine) return;
        close();
        sync();
      }, SILENCE_MS);
    };
    watch();
    mine.addEventListener("ready", heard(resync));
    mine.addEventListener("lagged", heard(resync));
    mine.addEventListener("alive", heard(() => {}));
    mine.addEventListener(
      "change",
      heard((data) => {
        let change: Change;
        try {
          change = JSON.parse(data) as Change;
        } catch {
          // A change that cannot be read is one that was missed.
          resync();
          return;
        }
        each((s) => {
          if (s.match(change)) s.reread();
        });
      }),
    );
    mine.addEventListener(
      "error",
      // An error is trouble, not a sign of life: it does not set the watchdog.
      heard(() => {
        // A stream that dropped is opened again by the browser, and says `ready`
        // again. One the browser gave up on (the answer was not a stream, say a
        // refusal) stays closed, so it is opened again after a while.
        if (mine.readyState !== CLOSED) return;
        mine.close();
        source = null;
        cancelWatchdog?.();
        cancelWatchdog = null;
        cancelReopen = setTimer(() => {
          cancelReopen = null;
          sync();
        }, REOPEN_MS);
      }, false),
    );
  };

  /** Opens or closes the connection to match whether somebody listens and the page is in view. */
  const sync = () => {
    if (subscribers.size > 0 && !deps.visibility.hidden()) {
      if (source === null && cancelReopen === null) connect();
    } else {
      close();
    }
  };

  return {
    subscribe(subscriber) {
      subscribers.add(subscriber);
      if (stopVisibility === null) stopVisibility = deps.visibility.onChange(sync);
      sync();
      return () => {
        if (!subscribers.delete(subscriber)) return;
        if (subscribers.size === 0) {
          stopVisibility?.();
          stopVisibility = null;
        }
        sync();
      };
    },
  };
}
