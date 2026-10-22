import { useEffect, useRef } from "react";

import type { Change } from "@/lib/api-types";

import { sharedEvents } from "./sharedEvents";

/**
 * Reads again when the server says something changed
 * (`docs/requirements/web-app.md`, 웹 명령과 상태 갱신). `reread` is called for a
 * change `match` accepts, and for every screen when the connection was made or
 * fell behind, so it has to read everything the screen shows.
 *
 * It listens from the screen's first effect on, so call it before the effect
 * that makes the screen's own first read: a change that happens during that
 * read is heard. It listens while `enabled` and the screen is mounted; the
 * connection itself is the tab's one, and is closed while the tab is hidden
 * (`events.ts`).
 */
export function useChanges(match: (change: Change) => boolean, reread: () => void, enabled = true) {
  const latest = useRef({ match, reread });
  latest.current = { match, reread };
  useEffect(() => {
    if (!enabled) return;
    return sharedEvents().subscribe({
      match: (change) => latest.current.match(change),
      reread: () => latest.current.reread(),
    });
  }, [enabled]);
}
