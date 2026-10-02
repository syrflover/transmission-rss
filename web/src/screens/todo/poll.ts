import { useEffect, useRef, useState } from "react";

import { ApiError, type ApiErrorCode } from "@/lib/api";
import { peek, store, useAfterDelay, useStored } from "@/lib/cached";

/**
 * Cache keys of the 할 일 screen and the job detail (`lib/cached.ts`). The
 * suggestions use the collect screen's own keys (`collect/cache.ts`), so both
 * screens show and refresh the same copy.
 */
export const KEYS = {
  todos: "todo:list",
  follow: "todo:subtitle-follow",
  jobs: "todo:jobs",
  job: (id: string) => `todo:job:${id}`,
} as const;

export interface Polled<T> {
  /** The cached or fresh value; `undefined` until the first answer for this key. */
  data: T | undefined;
  /** The message of the last failed read. Show it only when `data` is `undefined`. */
  error: string | null;
  /** The code of that failure (`not_found` for a thing that is not there). */
  code: ApiErrorCode | null;
  /** Nothing is cached and the first load has taken longer than `SLOW_MS`. */
  slow: boolean;
}

/**
 * Keeps the cached value of `key` fresh while the page is visible
 * (`docs/specs/web-app.md`, 웹 명령과 상태 갱신):
 *
 * - the next read is scheduled when the previous one has answered, so reads
 *   never stack and a slow server is never cut off by the next tick;
 * - nothing is polled while the page is hidden (only the first read does not
 *   wait), and the page is read at once when it becomes visible again;
 * - a failed read keeps the data on screen (only a first load has an error to
 *   show) and the polling goes on;
 * - `every` may be a function of the cached value: `null` stops the polling
 *   after the current read (a job that ended), a number goes on or starts it
 *   again. A thing that is not there (`not_found`) is not read again either.
 */
export function usePolled<T>(
  key: string,
  load: (signal: AbortSignal) => Promise<T>,
  every: number | null | ((data: T | undefined) => number | null),
  failure: string,
): Polled<T> {
  const data = useStored<T>(key);
  const [status, setStatus] = useState<{ key: string; error: string | null; code: ApiErrorCode | null }>({
    key,
    error: null,
    code: null,
  });
  const loadRef = useRef(load);
  loadRef.current = load;
  const failureRef = useRef(failure);
  failureRef.current = failure;
  const everyRef = useRef(every);
  everyRef.current = every;
  const gone = useRef(false);
  /** Starts the schedule again when `every` turns from `null` to a number. */
  const resume = useRef<() => void>(() => {});

  useEffect(() => {
    let stopped = false;
    let pending = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let controller: AbortController | undefined;

    const schedule = () => {
      clearTimeout(timer);
      timer = undefined;
      const rule = everyRef.current;
      const ms = typeof rule === "function" ? rule(peek<T>(key)) : rule;
      if (stopped || gone.current || ms === null || document.hidden) return;
      timer = setTimeout(() => run(), ms);
    };

    const run = (first = false) => {
      // The first read does not wait for the page to be shown; the polling does.
      if (stopped || pending || (document.hidden && !first)) return;
      clearTimeout(timer);
      timer = undefined;
      pending = true;
      const mine = new AbortController();
      controller = mine;
      loadRef.current(mine.signal).then(
        (value) => {
          if (stopped) return;
          store(key, value);
          setStatus({ key, error: null, code: null });
        },
        (e: unknown) => {
          if (stopped || mine.signal.aborted) return;
          if (e instanceof ApiError && e.code === "not_found") gone.current = true;
          setStatus({
            key,
            error: e instanceof ApiError ? e.message : failureRef.current,
            code: e instanceof ApiError ? e.code : null,
          });
        },
      ).finally(() => {
        pending = false;
        schedule();
      });
    };

    const onVisibility = () => {
      if (document.hidden) {
        clearTimeout(timer);
        timer = undefined;
      } else {
        run();
      }
    };

    gone.current = false;
    // A new value can change the interval (a rule that turns fast once a job starts), so the wait starts over.
    resume.current = () => {
      if (!pending) schedule();
    };
    document.addEventListener("visibilitychange", onVisibility);
    setStatus((prev) => (prev.key === key ? prev : { key, error: null, code: null }));
    run(true);

    return () => {
      stopped = true;
      clearTimeout(timer);
      controller?.abort();
      document.removeEventListener("visibilitychange", onVisibility);
    };
  }, [key]);

  // A job that was ended and is going again (or any change of the rule) takes up the schedule.
  useEffect(() => {
    resume.current();
  }, [data]);

  const mine = status.key === key;
  const error = mine ? status.error : null;
  return { data, error, code: mine ? status.code : null, slow: useAfterDelay(data === undefined && error === null) };
}
