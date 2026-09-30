import { useCallback, useEffect, useRef, useState, useSyncExternalStore } from "react";

import { ApiError } from "@/lib/api";

/**
 * Stale-while-revalidate for the screens that read from the server.
 *
 * The last answer for a key is kept in memory for the life of the page. A
 * screen that mounts again shows it in its first render and reads the server
 * again behind it; the fresh answer replaces the old one. So going back to a
 * tab never shows a loading line, and the layout keeps its height.
 *
 * Rules the screens rely on:
 *
 * - A failed refresh behind cached data is ignored by the screen: it keeps
 *   showing the data. Only a first load with nothing cached has an error to show.
 * - A first load shows its loading line only after {@link SLOW_MS}, so an answer
 *   that comes quickly never flashes it.
 * - Whoever changes the server's data keeps the cache true: {@link patch} (or
 *   `update` from the hook) for a change it can apply itself, {@link forget}
 *   for data it cannot. An answer to a read that started before such a write is
 *   dropped, so it cannot bring the old data back.
 */

/** How long a first load may take before the screen shows that it is loading. */
export const SLOW_MS = 300;

const entries = new Map<string, unknown>();
/** Bumped on every write to a key, so a read can tell that its answer is out of date. */
const versions = new Map<string, number>();
const listeners = new Set<() => void>();

const versionOf = (key: string) => versions.get(key) ?? 0;

function changed(key: string) {
  versions.set(key, versionOf(key) + 1);
  listeners.forEach((listener) => listener());
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** The cached value of a key, or `undefined` when there is none. */
export function peek<T>(key: string): T | undefined {
  return entries.get(key) as T | undefined;
}

/** Sets the value of a key. */
export function store<T>(key: string, value: T): void {
  entries.set(key, value);
  changed(key);
}

/** Applies a change to the cached value of a key; nothing happens when there is none. */
export function patch<T>(key: string, change: (value: T) => T): void {
  if (!entries.has(key)) return;
  store(key, change(entries.get(key) as T));
}

/** Drops a key, for data a write changed in a way the writer cannot apply itself. */
export function forget(key: string): void {
  if (entries.delete(key)) changed(key);
}

/** Drops every key that starts with `prefix`. */
export function forgetPrefix(prefix: string): void {
  for (const key of [...entries.keys()]) if (key.startsWith(prefix)) forget(key);
}

interface Status {
  key: string;
  slow: boolean;
  error: string | null;
}

export interface Cached<T> {
  /** The cached or fresh value; `undefined` until the first answer for this key. */
  data: T | undefined;
  /**
   * The message of the last failed read of this key. Show it only when `data`
   * is `undefined`; behind cached data it means the refresh failed.
   */
  error: string | null;
  /** Nothing is cached and the first load has taken longer than {@link SLOW_MS}. */
  slow: boolean;
  /** Reads the server again. The screen keeps what it shows until the answer comes. */
  reload: () => void;
  /** Changes the cached value, like {@link patch} / {@link store} for this key. */
  update: (next: T | ((value: T) => T)) => void;
}

/**
 * Shows the cached value of `key` and reads it from the server when the screen
 * mounts, when `key` changes and on {@link Cached.reload}. The `load` of the
 * render that starts a read is the one that runs.
 */
export function useCached<T>(
  key: string,
  load: (signal: AbortSignal) => Promise<T>,
  failure: string,
): Cached<T> {
  const data = useSyncExternalStore(subscribe, () => peek<T>(key));
  const [status, setStatus] = useState<Status>({ key, slow: false, error: null });
  const [round, setRound] = useState(0);
  const loadRef = useRef(load);
  loadRef.current = load;
  const failureRef = useRef(failure);
  failureRef.current = failure;

  useEffect(() => {
    const controller = new AbortController();
    const started = versionOf(key);
    const hasData = entries.has(key);
    let timer: ReturnType<typeof setTimeout> | undefined;
    if (hasData) {
      // A refresh behind shown data changes nothing on screen until it answers.
      setStatus((prev) => (prev.key === key ? { ...prev, slow: false } : { key, slow: false, error: null }));
    } else {
      setStatus({ key, slow: false, error: null });
      timer = setTimeout(() => setStatus((prev) => (prev.key === key ? { ...prev, slow: true } : prev)), SLOW_MS);
    }
    const current = () => !controller.signal.aborted && versionOf(key) === started;
    loadRef.current(controller.signal).then(
      (value) => {
        if (!current()) return;
        clearTimeout(timer);
        store(key, value);
        setStatus({ key, slow: false, error: null });
      },
      (e: unknown) => {
        if (!current()) return;
        clearTimeout(timer);
        setStatus({ key, slow: false, error: e instanceof ApiError ? e.message : failureRef.current });
      },
    );
    return () => {
      clearTimeout(timer);
      controller.abort();
    };
  }, [key, round]);

  const reload = useCallback(() => setRound((n) => n + 1), []);
  const update = useCallback(
    (next: T | ((value: T) => T)) => {
      if (typeof next === "function") patch(key, next as (value: T) => T);
      else store(key, next);
    },
    [key],
  );

  const mine = status.key === key;
  return { data, error: mine ? status.error : null, slow: mine && status.slow, reload, update };
}

/**
 * True once `active` has stayed true for {@link SLOW_MS}. For a loading line
 * that should not flash when the answer comes quickly.
 */
export function useAfterDelay(active: boolean): boolean {
  const [late, setLate] = useState(false);
  useEffect(() => {
    if (!active) {
      setLate(false);
      return;
    }
    const timer = setTimeout(() => setLate(true), SLOW_MS);
    return () => clearTimeout(timer);
  }, [active]);
  return active && late;
}
