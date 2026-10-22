import { useRead } from "@/lib/useRead";

import { loadMovieLink, movieLinkKey, type AnissiaLink } from "../api";
import { movieLinkChanged, WORK_RETRY_MS } from "./listens";

/**
 * A movie's Anissia link: its version and reference titles are not in the work's page, which names only the anime, so
 * the link is read with the page and shown again from the cache, and read again when the server says it changed
 * (`listens.ts`). `null` for a page that has no movie (`episode` is `null`). A change the page makes goes in with
 * `update`.
 */
export function useMovieLink(workId: string, episode: number | null) {
  return useRead<AnissiaLink | null>(
    movieLinkKey(workId, episode ?? 0),
    (signal) => (episode === null ? Promise.resolve(null) : loadMovieLink(workId, episode, signal)),
    { match: (change) => episode !== null && movieLinkChanged(change, workId), retryMs: WORK_RETRY_MS },
    "극장판의 Anissia 연결을 불러오지 못했어요.",
  );
}
