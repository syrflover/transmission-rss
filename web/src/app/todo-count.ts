import { useEffect, useSyncExternalStore } from "react";
import { useLocation } from "react-router-dom";

import { fetchTodoCount } from "@/screens/todo/api";

/**
 * The count shown on the 할 일 menu: only the tasks that need handling
 * (`처리 필요`), never the suggestions (`제안`). It is `undefined` until the
 * first answer, and the badge stays hidden then.
 *
 * One poll serves every nav: it runs while something shows the count (the top
 * and the bottom nav subscribe together), every {@link EVERY_MS} while the page
 * is visible, again when the page becomes visible and when the route changes.
 * A failed read keeps the last count. The 할 일 screen hands over the count its
 * own cards came with ({@link setTodoCount}), so the badge and the cards agree.
 */

const EVERY_MS = 15_000;

let count: number | undefined;
const listeners = new Set<() => void>();
let timer: ReturnType<typeof setTimeout> | undefined;
let controller: AbortController | undefined;
let pending = false;

/** Takes a count the screen already has; the badge shows it at once. */
export function setTodoCount(next: number): void {
  if (next === count) return;
  count = next;
  listeners.forEach((listener) => listener());
}

function schedule() {
  clearTimeout(timer);
  timer = undefined;
  if (listeners.size === 0 || document.hidden) return;
  timer = setTimeout(() => refreshTodoCount(), EVERY_MS);
}

/** Reads the count now, unless a read is on its way. Only the first read goes on while the page is hidden. */
export function refreshTodoCount(first = false): void {
  if (pending || listeners.size === 0 || (document.hidden && !first)) return;
  clearTimeout(timer);
  timer = undefined;
  pending = true;
  const mine = new AbortController();
  controller = mine;
  fetchTodoCount(mine.signal)
    .then(
      (next) => {
        if (!mine.signal.aborted) setTodoCount(next);
      },
      () => {
        // A failed read keeps the last count.
      },
    )
    .finally(() => {
      pending = false;
      schedule();
    });
}

function onVisibility() {
  if (document.hidden) {
    clearTimeout(timer);
    timer = undefined;
  } else {
    refreshTodoCount();
  }
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  if (listeners.size === 1) {
    document.addEventListener("visibilitychange", onVisibility);
    refreshTodoCount(true);
  }
  return () => {
    listeners.delete(listener);
    if (listeners.size === 0) {
      document.removeEventListener("visibilitychange", onVisibility);
      clearTimeout(timer);
      timer = undefined;
      controller?.abort();
      pending = false;
    }
  };
}

export function useTodoCount(): number | undefined {
  const value = useSyncExternalStore(subscribe, () => count);
  const { key } = useLocation();
  // The route changed: whatever was handled on the way is counted again.
  useEffect(() => {
    refreshTodoCount();
  }, [key]);
  return value;
}
