import { useCached } from "@/lib/cached";

import { loadMovieLink, movieLinkKey, type AnissiaLink } from "../api";

/**
 * A movie's Anissia link: its version and reference titles are not in the work's page, which names only the anime, so
 * the link is read with the page and shown again from the cache. `null` for a page that has no movie (`episode` is
 * `null`). A change the page makes goes in with `update`.
 */
export function useMovieLink(workId: string, episode: number | null) {
  return useCached<AnissiaLink | null>(
    movieLinkKey(workId, episode ?? 0),
    (signal) => (episode === null ? Promise.resolve(null) : loadMovieLink(workId, episode, signal)),
    "극장판의 Anissia 연결을 불러오지 못했어요.",
  );
}
