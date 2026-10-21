import type { Change } from "./api-types";
import type { Events } from "./events";

/**
 * How a screen follows a web command until it ends, without asking the server
 * on a timer: it reads the command once when it starts to follow it, and again
 * whenever the event stream says the command changed or that changes may have
 * been missed (`events.ts`). It has no `@/` imports so it runs under `node --test`;
 * `useCommandWatch.ts` is the hook the screens use.
 *
 * What a read does with the command, and when the command is final, is the
 * screen's: it calls {@link CommandWatch.unwatch} (or {@link CommandWatch.stop})
 * once it has what it needs, as a timer used to be cleared.
 */

/** A read of a command; the next read of the same command waits until it is over. */
export type Reread = () => void | Promise<void>;

/** How long to wait before reading again a command whose last read failed, in milliseconds. */
export const RETRY_MS = 3000;

/** Whether `change` is about the command `id`. */
export function isCommand(change: Change, id: string): boolean {
  return change.kind === "command" && change.id === id;
}

interface Followed {
  stop: () => void;
  /** A read is under way. */
  reading: boolean;
  /** A change came while it was: read again when it is over. */
  again: boolean;
  reread: Reread;
}

const defaultTimer = (run: () => void, ms: number) => {
  const timer = setTimeout(run, ms);
  return () => clearTimeout(timer);
};

export class CommandWatch {
  private readonly followed = new Map<string, Followed>();
  /** The reads that wait to try again after a failed one, by command ID. */
  private readonly retries = new Map<string, () => void>();
  private readonly events: Events;
  private readonly setTimer: (run: () => void, ms: number) => () => void;

  constructor(events: Events, setTimer?: (run: () => void, ms: number) => () => void) {
    this.events = events;
    this.setTimer = setTimer ?? defaultTimer;
  }

  /**
   * Follows the command `id`: listens for its changes, then reads it with
   * `reread`. Listening starts first, so a change between the screen's last
   * read (the answer that told it the command) and this one is not lost.
   * Does nothing when `id` is followed already. Changes that come while
   * `reread` runs make it run once more after, not at the same time.
   */
  watch(id: string, reread: Reread): void {
    if (this.followed.has(id)) return;
    const mine: Followed = { stop: () => {}, reading: false, again: false, reread };
    this.followed.set(id, mine);
    mine.stop = this.events.subscribe({
      match: (change) => isCommand(change, id),
      reread: () => void this.read(id, mine),
    });
    void this.read(id, mine);
  }

  /**
   * Reads `id` again with `reread` in `delayMs` ({@link RETRY_MS} unless the
   * screen has an interval of its own), for a read that failed (the server
   * could not be reached). Replaces a retry of `id` that waits
   * already. It needs no {@link CommandWatch.watch}: a command whose request
   * got no answer is read this way before it is known to exist. The server
   * making its stream again (`ready`) reads the followed commands at once
   * anyway.
   */
  retry(id: string, reread: Reread, delayMs: number = RETRY_MS): void {
    this.retries.get(id)?.();
    this.retries.set(
      id,
      this.setTimer(() => {
        this.retries.delete(id);
        void Promise.resolve()
          .then(reread)
          .catch(() => {
            // The screen's read reports its own failure.
          });
      }, delayMs),
    );
  }

  /** Stops following `id`, and forgets a retry of it. */
  unwatch(id: string): void {
    this.retries.get(id)?.();
    this.retries.delete(id);
    const mine = this.followed.get(id);
    if (!mine) return;
    this.followed.delete(id);
    mine.stop();
  }

  /** Follows exactly the commands in `ids`, each read with `reread(id)`. */
  sync(ids: readonly string[], reread: (id: string) => void | Promise<void>): void {
    const wanted = new Set(ids);
    for (const id of [...this.followed.keys()]) {
      if (!wanted.has(id)) this.unwatch(id);
    }
    for (const id of ids) this.watch(id, () => reread(id));
  }

  /** Stops following every command, and forgets the retries. */
  stop(): void {
    for (const id of [...this.followed.keys(), ...this.retries.keys()]) this.unwatch(id);
  }

  private async read(id: string, mine: Followed): Promise<void> {
    if (this.followed.get(id) !== mine) return;
    if (mine.reading) {
      mine.again = true;
      return;
    }
    mine.reading = true;
    try {
      await mine.reread();
    } catch {
      // The screen's read reports its own failure.
    } finally {
      mine.reading = false;
    }
    if (mine.again) {
      mine.again = false;
      void this.read(id, mine);
    }
  }
}
