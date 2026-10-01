import { useCallback, useState } from "react";

import { ORDERS, type EpisodeOrder } from "./model";

/**
 * What the work page remembers in this browser (`localStorage`): the order of
 * the episode list. Storage can be blocked or full; every access is guarded
 * and the page works without it.
 */
const STORAGE_KEY = "trss-work-v1";

function read(): EpisodeOrder {
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    if (raw) {
      const parsed = JSON.parse(raw) as { order?: unknown };
      if (ORDERS.some((o) => o.key === parsed.order)) return parsed.order as EpisodeOrder;
    }
  } catch {
    // Blocked storage or an unreadable value: the default.
  }
  return "latest";
}

export function useEpisodeOrder(): [EpisodeOrder, (order: EpisodeOrder) => void] {
  const [order, setOrder] = useState<EpisodeOrder>(read);
  const choose = useCallback((next: EpisodeOrder) => {
    setOrder(next);
    try {
      window.localStorage.setItem(STORAGE_KEY, JSON.stringify({ order: next }));
    } catch {
      // Not remembered; the choice still applies to this visit.
    }
  }, []);
  return [order, choose];
}
