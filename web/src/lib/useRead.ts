import { useEffect, useRef, useState } from "react";

import { ApiError, type ApiErrorCode } from "@/lib/api";
import type { Change } from "@/lib/api-types";
import { peek, store, useAfterDelay, useStored, versionOf } from "@/lib/cached";

import { createRereader } from "./rereads";
import { sharedEvents } from "./sharedEvents";

export interface Read<T> {
  /** The cached or fresh value; `undefined` until the first answer for this key. */
  data: T | undefined;
  /** The message of the last failed read. Show it only when `data` is `undefined`. */
  error: string | null;
  /** The code of that failure (`not_found` for a thing that is not there). */
  code: ApiErrorCode | null;
  /** Nothing is cached and the first load has taken longer than `SLOW_MS`. */
  slow: boolean;
}

/** When to read again. */
export interface Listen<T> {
  /**
   * Whether `change` is about what is read. `data` is the cached value, so a
   * screen can tell which target the value is about. A change that names no
   * target of its kind (`id` is `null`) means anything of the kind may have
   * changed, so a screen that reads the kind matches it too.
   */
  match: (change: Change, data: T | undefined) => boolean;
  /**
   * After a read that failed (the server could not be reached), how long to
   * wait before reading again, in milliseconds; `null` for not at all. It is
   * the interval the screen read at when it asked the server on a timer.
   */
  retryMs: number | ((data: T | undefined) => number | null);
}

/**
 * Keeps the cached value of `key` fresh (`docs/specs/web-app.md`, 웹 명령과
 * 상태 갱신): it reads when the screen starts, and again when the event stream
 * says something `listen.match` accepts changed or that changes may have been
 * missed (the connection was made again, or fell behind: `events.ts`). The reads
 * are the screen's own routes, as they were when the screen read on a timer.
 *
 * - Events come in bursts, so a change that arrives while a read is under way
 *   only makes one more read after it ends (`rereads.ts`).
 * - Nothing is read while the page is hidden: the connection is closed then,
 *   and showing the page reads again.
 * - A failed read keeps the data on screen (only a first load has an error to
 *   show) and is tried again after `listen.retryMs`. A thing that is not there
 *   (`not_found`) is not read again.
 *
 * It listens from the first effect on and before the first read, so a change
 * that happens during that read is heard.
 */
export function useRead<T>(
  key: string,
  load: (signal: AbortSignal) => Promise<T>,
  listen: Listen<T>,
  failure: string,
): Read<T> {
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
  const listenRef = useRef(listen);
  listenRef.current = listen;

  useEffect(() => {
    const rereader = createRereader({
      read: async (signal) => {
        // A screen that wrote the key while this read was on its way (what it just sent) has a newer value than the
        // answer, as with `useCached`; a key that was forgotten meanwhile takes the answer.
        const started = versionOf(key);
        try {
          const value = await loadRef.current(signal);
          if (signal.aborted) return;
          if (versionOf(key) === started || peek<T>(key) === undefined) store(key, value);
          setStatus({ key, error: null, code: null });
        } catch (e) {
          if (signal.aborted) return;
          setStatus({
            key,
            error: e instanceof ApiError ? e.message : failureRef.current,
            code: e instanceof ApiError ? e.code : null,
          });
          throw e;
        }
      },
      retryMs: () => {
        const { retryMs } = listenRef.current;
        return typeof retryMs === "function" ? retryMs(peek<T>(key)) : retryMs;
      },
      gone: (e) => e instanceof ApiError && e.code === "not_found",
      hidden: () => document.hidden,
    });
    const stopListening = sharedEvents().subscribe({
      match: (change) => listenRef.current.match(change, peek<T>(key)),
      reread: () => rereader.request(),
    });
    setStatus((prev) => (prev.key === key ? prev : { key, error: null, code: null }));
    rereader.start();
    return () => {
      stopListening();
      rereader.stop();
    };
  }, [key]);

  const mine = status.key === key;
  const error = mine ? status.error : null;
  return { data, error, code: mine ? status.code : null, slow: useAfterDelay(data === undefined && error === null) };
}
