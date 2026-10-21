import type { Change, ChangeKind } from "../../lib/api-types";
import { aboutTarget, ofKinds } from "../../lib/changeMatch.ts";

/**
 * Which changes make the 할 일 screen, the menu's 할 일 badge and the job
 * detail read again (`docs/specs/web-app.md`, 웹 명령과 상태 갱신). Each list of
 * kinds is the tables the screen's route reads, grouped as the server announces
 * them (`crates/trss-core/src/changes/tables.rs`, `TABLES`). A kind that is
 * only likely to matter is in the list too: a read too many costs little, a
 * read too few leaves a screen on an old state.
 *
 * - The reads of a burst of changes are one read (`rereads.ts`), so a kind that
 *   changes often does not cost a read per change.
 * - A change with no ID (`null`) of a kind a screen reads always matches.
 */

/**
 * The 자막 작업 list (`GET /api/subtitle-jobs`): the jobs with their steps,
 * items and files (`subtitle_job`), and what each row names of its work: the
 * title (`library`), the cover (`artwork`) and the Anissia title (`anissia`).
 */
const JOB_LIST: readonly ChangeKind[] = ["subtitle_job", "library", "artwork", "anissia"];

export function jobListChanged(change: Change): boolean {
  return ofKinds(change, JOB_LIST);
}

/**
 * What the job detail (`GET /api/subtitle-jobs/{id}`) reads of the job's work:
 * the episodes a file can go on and whether they have a video or a subtitle
 * (`library`, `subtitle_store`), the season's episode count (`season_info`),
 * the mapping that placed a file (`subtitle_mapping`) and the cover.
 */
const OF_ITS_WORK: readonly ChangeKind[] = ["library", "artwork", "subtitle_store", "subtitle_mapping", "season_info"];

/**
 * Whether `change` is about the job `jobId`: its steps, state, items, files,
 * plan and replacements (`subtitle_job`) and its remote screen
 * (`subtitle_screen`), and its work. `work` is the ID of the job's work as the
 * last read gave it, `null` for a job that has none, and `undefined` before
 * the first read, when any work may be the job's.
 */
export function jobChanged(change: Change, jobId: string, work: string | null | undefined): boolean {
  switch (change.kind) {
    case "subtitle_job":
    case "subtitle_screen":
      return aboutTarget(change, jobId);
    case "anissia":
      return true;
    default:
      return ofKinds(change, OF_ITS_WORK) && work !== null && (work === undefined || aboutTarget(change, work));
  }
}

/**
 * The 할 일 list and its count (`GET /api/todo`, `GET /api/todo/count`): the
 * to-dos are gathered from the jobs (`subtitle_job`), the stored subtitles a
 * replacement compares with (`subtitle_store`), the subscribed creators'
 * mappings (`subtitle_mapping`), the library's videos and works (`library`,
 * `artwork`), the collection's failures (`history`, `rules`), the Anissia
 * titles (`anissia`) and the collect folder (`settings`).
 */
const TODOS: readonly ChangeKind[] = [
  "subtitle_job",
  "subtitle_store",
  "subtitle_mapping",
  "library",
  "artwork",
  "history",
  "rules",
  "anissia",
  "settings",
];

export function todosChanged(change: Change): boolean {
  return ofKinds(change, TODOS);
}

/**
 * The 할 일 screen's suggestions (`제목 후보`, `보관 제안`, `자막 구독`): they
 * are derived from the rules and the collection history, the library and the
 * Anissia anime, and the archive suggestions also from how long a feed was
 * quiet and which days it was read (`collect_status`, which a collection cycle
 * writes, so a suggestion that the passing of time brings shows at the next
 * cycle).
 */
const SUGGESTIONS: readonly ChangeKind[] = [
  "rules",
  "history",
  "library",
  "artwork",
  "anissia",
  "subtitle_mapping",
  "collect_status",
  "settings",
];

export function suggestionsChanged(change: Change): boolean {
  return ofKinds(change, SUGGESTIONS);
}

/** How long to wait before reading the jobs again after a read that failed, in milliseconds. */
export const JOBS_RETRY_MS = 2000;

/** ...and the job detail. */
export const JOB_RETRY_MS = 2000;

/** ...and the to-dos, their count and the suggestions. */
export const TODOS_RETRY_MS = 10_000;

/** ...and the menu's badge. */
export const BADGE_RETRY_MS = 15_000;
