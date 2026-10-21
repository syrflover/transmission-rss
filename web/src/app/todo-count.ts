import { useEffect, useRef, useSyncExternalStore } from "react";
import { useLocation } from "react-router-dom";

import { createRereader, type Rereader } from "@/lib/rereads";
import { sharedEvents } from "@/lib/sharedEvents";
import { fetchTodoCount } from "@/screens/todo/api";
import { BADGE_RETRY_MS, todosChanged } from "@/screens/todo/listens";

/**
 * The count shown on the 할 일 menu: only the tasks that need handling
 * (`처리 필요`), never the suggestions (`제안`). It is `undefined` until the
 * first answer, and the badge stays hidden then.
 *
 * One reader serves every nav: it runs while something shows the count (the top
 * and the bottom nav subscribe together). It reads the count when the server
 * says the to-dos changed (`screens/todo/listens.ts`, the same changes as the 할 일
 * list), and again when the route changes. A failed read keeps the last count
 * and is tried again after {@link BADGE_RETRY_MS}. The 할 일 screen hands over
 * the count its own cards came with ({@link setTodoCount}), so the badge and the
 * cards agree.
 */

let count: number | undefined;
const listeners = new Set<() => void>();
let reader: Rereader | undefined;
let stopListening: (() => void) | undefined;

/** Takes a count the screen already has; the badge shows it at once. */
export function setTodoCount(next: number): void {
  if (next === count) return;
  count = next;
  listeners.forEach((listener) => listener());
}

/** Reads the count now, unless a read is on its way (then once more after it). Nothing is read while the page is hidden. */
export function refreshTodoCount(): void {
  reader?.request();
}

function start() {
  const mine = createRereader({
    read: async (signal) => {
      const next = await fetchTodoCount(signal);
      if (!signal.aborted) setTodoCount(next);
    },
    retryMs: () => BADGE_RETRY_MS,
    hidden: () => document.hidden,
  });
  reader = mine;
  // Listening starts before the first read, so a change during it is heard.
  stopListening = sharedEvents().subscribe({ match: todosChanged, reread: () => mine.request() });
  mine.start();
}

function stop() {
  stopListening?.();
  stopListening = undefined;
  reader?.stop();
  reader = undefined;
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  if (listeners.size === 1) start();
  return () => {
    listeners.delete(listener);
    if (listeners.size === 0) stop();
  };
}

export function useTodoCount(): number | undefined {
  const value = useSyncExternalStore(subscribe, () => count);
  const { key } = useLocation();
  // The route changed: whatever was handled on the way is counted again. The first route is read by the reader's start.
  const first = useRef(true);
  useEffect(() => {
    if (first.current) {
      first.current = false;
      return;
    }
    refreshTodoCount();
  }, [key]);
  return value;
}
