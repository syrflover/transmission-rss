/**
 * How a screen reads again what it shows when the event stream says it changed
 * (`events.ts`), without a read for every event. It has no `@/` imports, React
 * or browser globals so it runs under `node --test`; `useRead.ts` is the hook
 * the screens use.
 *
 * Events come in bursts: a collection cycle announces 250 `history` changes
 * within 5 ms, and one subtitle job announces 12 changes of the same job
 * within 74 ms (lighter-structure/0164), and the server does not
 * gather them. So a {@link Rereader} keeps one read in flight: a request that
 * comes while it reads only sets a flag, and exactly one more read follows when
 * the read ends, however many requests came. That read starts after the last
 * request, so it sees everything the burst changed.
 */

export interface RereaderOptions {
  /**
   * Reads what the screen shows and puts it on the screen. It rejects when the
   * read failed; `signal` aborts when the rereader is stopped.
   */
  read: (signal: AbortSignal) => Promise<void>;
  /**
   * How long to wait before reading again after `error`, in milliseconds, for a
   * read that failed (the server could not be reached): `null` for not at all.
   * The server making its stream again (`ready`) asks for a read at once
   * anyway. Not set: no retry.
   */
  retryMs?: (error: unknown) => number | null;
  /**
   * Whether `error` says that what is read is not there (`not_found`): nothing
   * is read again after it.
   */
  gone?: (error: unknown) => boolean;
  /** Whether the page is out of sight; a request does not read then (showing the page reads everything). */
  hidden?: () => boolean;
  /** Runs `run` after `ms` milliseconds; returns how to cancel it. */
  setTimer?: (run: () => void, ms: number) => () => void;
}

export interface Rereader {
  /** Makes the first read. It does not wait for the page to be in sight. */
  start(): void;
  /**
   * Asks for a read: it happens now, or, when one is on its way, once more when
   * that one ends. Does nothing while the page is hidden, after {@link Rereader.stop} and
   * after what is read turned out to be gone.
   */
  request(): void;
  /** Aborts the read on its way and forgets a retry; nothing is read after it. */
  stop(): void;
}

const defaultTimer = (run: () => void, ms: number) => {
  const timer = setTimeout(run, ms);
  return () => clearTimeout(timer);
};

export function createRereader(options: RereaderOptions): Rereader {
  const setTimer = options.setTimer ?? defaultTimer;
  let reading = false;
  /** A request came while a read was on its way. */
  let again = false;
  let stopped = false;
  let gone = false;
  let controller: AbortController | undefined;
  let cancelRetry: (() => void) | null = null;

  const run = async (): Promise<void> => {
    cancelRetry?.();
    cancelRetry = null;
    reading = true;
    again = false;
    const mine = new AbortController();
    controller = mine;
    try {
      await options.read(mine.signal);
    } catch (error) {
      if (!stopped && !mine.signal.aborted) {
        if (options.gone?.(error)) {
          gone = true;
        } else {
          const ms = options.retryMs?.(error) ?? null;
          if (ms !== null) {
            cancelRetry = setTimer(() => {
              cancelRetry = null;
              request();
            }, ms);
          }
        }
      }
    } finally {
      reading = false;
    }
    if (again && !stopped && !gone) await run();
  };

  const request = () => {
    if (stopped || gone || options.hidden?.()) return;
    if (reading) {
      again = true;
      return;
    }
    void run();
  };

  return {
    start() {
      if (stopped) return;
      if (reading) {
        again = true;
        return;
      }
      void run();
    },
    request,
    stop() {
      stopped = true;
      controller?.abort();
      cancelRetry?.();
      cancelRetry = null;
    },
  };
}
