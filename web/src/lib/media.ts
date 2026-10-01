import { useSyncExternalStore } from "react";

/**
 * Whether a CSS media query matches, kept up to date. For a screen that shows
 * a different component (not just different styles) on a phone, so the one
 * that is not shown is not in the page at all (not in the tab order, and not
 * read by assistive technology).
 */
export function useMediaQuery(query: string): boolean {
  return useSyncExternalStore(
    (notify) => {
      const list = window.matchMedia(query);
      list.addEventListener("change", notify);
      return () => list.removeEventListener("change", notify);
    },
    () => window.matchMedia(query).matches,
    () => false,
  );
}

/** A phone: the same width the layout's `max-[720px]` styles switch at. */
export const PHONE_QUERY = "(max-width: 720px)";
