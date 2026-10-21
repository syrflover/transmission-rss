import type { Change, ChangeKind } from "../../../lib/api-types";
import { aboutTarget, ofKinds } from "../../../lib/changeMatch.ts";

/**
 * Which changes make the work detail's parts read again
 * (`docs/specs/web-app.md`, 웹 명령과 상태 갱신), by the tables their routes
 * read, grouped as the server announces them
 * (`crates/trss-core/src/changes/tables.rs`, `TABLES`). A change with no ID
 * (`null`) of a kind a part reads always matches; a kind that is only likely to
 * matter is listed too, because a read too many costs little and one too few
 * leaves the part on an old state. It has no `@/` imports so it runs under
 * `node --test`.
 */

/**
 * What a season's candidates (`GET /api/library/works/{id}/seasons/{n}/anissia/candidates`)
 * read of the work: the season's Anissia link (`season_info`), the creators'
 * mappings (`subtitle_mapping`), the subtitle files with a creator and the
 * season's episode facts (`library`, `subtitle_store`).
 */
const OF_THE_WORK: readonly ChangeKind[] = ["season_info", "subtitle_mapping", "library", "subtitle_store"];

/**
 * Whether `change` is about the candidates of the Anissia anime `animeNo` of
 * the work `workId`:
 *
 * - the anime's lines, sources and when Anissia was last read (`anissia`, which
 *   a read of Anissia by `trss` itself writes, with no one asking for it);
 * - the jobs of the candidates and the jobs made for them (`subtitle_job`: the
 *   candidate names the job, and a job made elsewhere takes candidates this page
 *   does not know yet, so any job matches);
 * - the `새로고침` command (`command`): the page names it by an ID the browser
 *   made, and the list tells it only after a read, so a change of it between
 *   the request and that read would be missed by an ID match. Any command
 *   matches; commands are what a person asked for, so they are few.
 */
export function candidatesChanged(change: Change, workId: string, animeNo: number | null): boolean {
  if (animeNo === null) return false;
  switch (change.kind) {
    case "anissia":
      return aboutTarget(change, String(animeNo));
    case "subtitle_job":
    case "command":
      return true;
    default:
      return ofKinds(change, OF_THE_WORK) && aboutTarget(change, workId);
  }
}

/**
 * The work page's reading of the work's stored files, while a clean waits for
 * `trss` (`보관 파일 정리`): the clean and the files it removes
 * (`subtitle_store`, the work's), and the jobs whose files it must leave
 * (`subtitle_job`).
 */
export function cleanupChanged(change: Change, workId: string): boolean {
  switch (change.kind) {
    case "subtitle_store":
      return aboutTarget(change, workId);
    case "subtitle_job":
      return true;
    default:
      return false;
  }
}

/**
 * Whether `change` makes the line of a stored copy's apply read again: the job
 * that applies it (`subtitle_job`), and, once the job is done, the work's
 * episodes listing the file the job put beside a video (`library`, the watch
 * folder's reading of the work) and the stored copy itself (`subtitle_store`).
 */
export function applyChanged(change: Change, jobId: string, workId: string): boolean {
  switch (change.kind) {
    case "subtitle_job":
      return aboutTarget(change, jobId);
    case "library":
    case "subtitle_store":
      return aboutTarget(change, workId);
    default:
      return false;
  }
}

/**
 * Whether `change` is about the cover of the work `workId` (`work_artwork`): the
 * work page's `cover_url` and `cover_pending` (`GET /api/library/works/{id}`) and
 * the cover view (`GET /api/library/works/{id}/artwork`) are read from the row
 * the worker writes when it has received the image.
 */
export function workCoverChanged(change: Change, workId: string): boolean {
  return change.kind === "artwork" && aboutTarget(change, workId);
}

/**
 * Whether `change` is about the season info of the work `workId` (`GET
 * /api/library/works/{id}/seasons/{n}/info`): the season's link, the AniList
 * entries it names and the search the app still has to do for it. An AniList
 * entry belongs to no work, so its change has no ID and is every work's.
 */
export function seasonInfoChanged(change: Change, workId: string): boolean {
  return change.kind === "season_info" && aboutTarget(change, workId);
}

/** How long to wait before reading the work's cover again after a read that failed, in milliseconds. */
export const COVER_RETRY_MS = 3000;

/** ...and the cover view. */
export const COVER_VIEW_RETRY_MS = 3000;

/** ...and the season info. */
export const SEASON_INFO_RETRY_MS = 4000;
