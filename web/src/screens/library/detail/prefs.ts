import { useCallback, useState } from "react";

import type { EpisodeOrder } from "./model";

/**
 * What the work page remembers for the life of the page: the order of the
 * episode list, which carries over to the next work opened. A reload or a new
 * page starts from the newest episode (user decision, 2026-10-04).
 */
let kept: EpisodeOrder = "latest";

export function useEpisodeOrder(): [EpisodeOrder, (order: EpisodeOrder) => void] {
  const [order, setOrder] = useState<EpisodeOrder>(kept);
  const choose = useCallback((next: EpisodeOrder) => {
    kept = next;
    setOrder(next);
  }, []);
  return [order, choose];
}
