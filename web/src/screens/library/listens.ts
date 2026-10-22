import type { Change, ChangeKind } from "../../lib/api-types";
import { ofKinds } from "../../lib/changeMatch.ts";

/**
 * Which changes make the library list read again (`docs/requirements/web-app.md`,
 * 웹 명령과 상태 갱신; `docs/requirements/library.md`, 라이브러리 화면). The kinds are
 * the tables its route (`GET /api/library/works`) reads, grouped as the server
 * announces them (`crates/trss-core/src/changes/tables.rs`, `TABLES`); a kind
 * that is only likely to matter is in the list too, because a read too many
 * costs little and one too few leaves the list on an old state. It has no `@/`
 * imports so it runs under `node --test`.
 *
 * - `library`: the works, their folders, videos and subtitles beside them.
 * - `season_info`, `anissia`: the work title (the Anissia title of the first
 *   linked season) and the AniList entries the airing filter reads.
 * - `artwork`: the covers.
 * - `settings`: the collect and archive folders (`보관` badge, folder choice).
 * - `subtitle_job`, `subtitle_store`, `subtitle_mapping`, `rules`, `history`:
 *   what the to-do badges are gathered from, as the 할 일 screen's
 *   (`todo/listens.ts`).
 */
const LIST: readonly ChangeKind[] = [
  "library",
  "season_info",
  "anissia",
  "artwork",
  "settings",
  "subtitle_job",
  "subtitle_store",
  "subtitle_mapping",
  "rules",
  "history",
];

export function libraryListChanged(change: Change): boolean {
  return ofKinds(change, LIST);
}

/** How long to wait before reading the list again after a read that failed, in milliseconds. */
export const LIST_RETRY_MS = 30_000;
