import { api } from "@/lib/api";

import type { EpisodeSegment } from "./episodeLine.ts";
import type { TodoKind } from "./kinds";
import type { ConfirmView, Placement, PlacementChoice, Relocation } from "./placementTypes";
import type {
  Replacement,
  ReplacementChanges,
  ReplacementDecision,
  ReplacementLines,
  ReplacementResult,
} from "./replacementTypes";
import type { UnpackResult } from "./unpack";

export type { EpisodeSegment };

export type { UnpackResult } from "./unpack";
export type {
  ConfirmEpisode,
  ConfirmView,
  Placement,
  PlacementChoice,
  PlacementOutcome,
  Relocation,
} from "./placementTypes";
export type {
  Comparison,
  DialogueLine,
  Replacement,
  ReplacementChanges,
  ReplacementDecision,
  ReplacementLines,
  ReplacementPath,
  ReplacementResult,
  ReplacementState,
  ReplacementVersion,
  TimingLine,
} from "./replacementTypes";

/**
 * The 할 일 screen and the job detail (`docs/specs/jobs.md`, 할 일 화면 and
 * 작업 상세) read these: the to-dos that need the user (`/api/todo`,
 * `src/web/todo_api.rs`) and the subtitle jobs the worker carries out
 * (`/api/subtitle-jobs`, `src/web/jobs_api.rs`). The suggestions (`제목 후보`,
 * `보관 제안`) come from the collect screen's own sources
 * (`collect/subs/api.ts`, `collect/archive/api.ts`); `자막 구독` from
 * `/api/todo/subtitle-follow`.
 */

// ---------------------------------------------------------------------------
// To-dos that need handling (`처리 필요`)

/** The work a to-do or a job is about, when the library has it. */
export interface WorkRef {
  id: string;
  /** The work's folder name. */
  name: string;
  /** Where its cover image is served, while it has one. */
  cover_url: string | null;
}

/** What every to-do carries: the badge its kind is named by, which the server decides. */
interface Badged {
  /**
   * `placement_check` and `video_check` are `episode_check`, like a mapping's `회차 확인 필요`. The 할 일 cards, the
   * library grid's covers and the work detail's `할 일` show the same name and colour (`badges.tsx`, `TodoBadge`).
   */
  badge: TodoKind;
}

/**
 * `인증 필요`: subtitle jobs of one work wait for a person to solve a site's
 * check. One to-do per work; it opens the oldest such job.
 */
export interface AuthTodo extends Badged {
  kind: "auth";
  /** Stable across reads: what the screen keys the card by. */
  key: string;
  /** Since when the oldest of its jobs waits (Unix ms). */
  at: number;
  work: WorkRef | null;
  /** What to call it: the Anissia title of the job's anime, else the work's name. */
  title: string;
  season: number | null;
  /** The episodes that wait, as Anissia writes them (`"11"`, `"13.5"`). */
  episodes: string[];
  /** `episodes` as runs. */
  episode_segments: EpisodeSegment[];
  creator: string | null;
  /** Why, in a few words (`"CAPTCHA"`). */
  reason: string;
  /** The job the card's `인증` opens. */
  job_id: string;
  /** How many jobs of the work wait for a check. */
  jobs: number;
}

/**
 * `받기 실패`: collection failures the user has to deal with, one to-do per
 * work (a failed video revision replacement, `revision`) or per rule (an item
 * Transmission did not add, `add_failed`).
 */
export interface ReceiveFailedTodo extends Badged {
  kind: "receive_failed";
  key: string;
  /** The newest failure's time (Unix ms). */
  at: number;
  context: "revision" | "add_failed";
  work: WorkRef | null;
  /** The work's name, or for an `add_failed` with no work the rule's folder. */
  title: string;
  /** `revision`: the season of the episodes; `null` when it cannot be read. */
  season: number | null;
  /** `revision`: the episodes whose replacement failed. Empty for `add_failed`. */
  episodes: string[];
  /** `episodes` as runs. */
  episode_segments: EpisodeSegment[];
  /** How many failures the to-do gathers. */
  count: number;
  /** The newest failure's reason. */
  reason: string | null;
  /** `add_failed`: the channel, for the history tab's filter. */
  channel_id: string | null;
}

/**
 * `회차 확인 필요`: a work's subscribed creator has a mapping to the season the app cannot decide (`reason` says
 * why), or some of its episodes fit the decided mapping nowhere, so they are not received. One to-do per work; it
 * names the lowest season's check and opens that creator's group in the work's 자막 후보.
 */
export interface EpisodeCheckTodo extends Badged {
  kind: "episode_check";
  key: string;
  /** Since when the work's oldest check waits (Unix ms). */
  at: number;
  work: WorkRef | null;
  title: string;
  season: number;
  creator: string;
  /** The creator's ID of the anime's lines: what the group's address names. */
  source_id: string;
  /** The episodes that fit nowhere; empty for an undecided mapping. */
  episodes: string[];
  /** `episodes` as runs. */
  episode_segments: EpisodeSegment[];
  /** Why the mapping is undecided; `null` when the mapping is decided and episodes do not fit. */
  reason: string | null;
  /** How many seasons and creators of the work need a check. */
  sources: number;
}

/**
 * `회차 확인 필요` of a subtitle job: it received files whose episode a person has to say (its 배치 확인). One per
 * job; it opens the job's detail.
 */
export interface PlacementCheckTodo extends Badged {
  kind: "placement_check";
  key: string;
  /** Since when the job waits (Unix ms). */
  at: number;
  work: WorkRef | null;
  title: string;
  season: number | null;
  creator: string | null;
  /** How the job came to be (`JobRow.origin`): with `source`, its source as the work detail's card says it. */
  origin: JobRow["origin"];
  /** The host of its posts, when it has posts. */
  source: string | null;
  /** The names of the files it asks about: an upload's or a find job's subtitles its table places, else the held ones. */
  files: string[];
  /** The first file's question, or for an upload's or a find job's table what the job waits for. */
  reason: string | null;
  job_id: string;
}

/**
 * `회차 확인 필요` of a video: a video directly in a season folder other than `Season 00` whose name gives no episode,
 * or another season's. The app neither puts it on an episode nor counts it missing. One per video, until the person
 * says `확인함` (`checkVideo`) or the video is renamed or moved; another video put at the path is asked about again.
 */
export interface VideoCheckTodo extends Badged {
  kind: "video_check";
  key: string;
  /** The video's modification time (Unix ms). */
  at: number;
  work: WorkRef | null;
  title: string;
  /** The season of the folder it is in. */
  season: number;
  /** Relative to the work folder: the season folder and the file name (the card's title in the work detail). */
  path: string;
  /** Why its episode is not known, as a sentence. */
  reason: string;
  /** The video as the scan saw it, which `checkVideo` sends back as it is. */
  seen: string;
}

/**
 * `확인함` on a video a `video_check` to-do asks about: it is no longer asked about while it is the video at the path.
 * A `409` (`conflict`) is another video there since the list was read; a `404` is one renamed or moved.
 */
export function checkVideo(workId: string, path: string, seen: string): Promise<void> {
  return api<void>(`/library/works/${encodeURIComponent(workId)}/videos/check`, {
    method: "POST",
    body: { path, seen },
  });
}

/**
 * `교체 승인`: subtitle jobs of one work wait for a person to approve or refuse replacing the subtitle an episode
 * already has. One to-do per work (per job when it has no work); it opens the oldest such job. Its reason line is the
 * number tags of `changes` (`changes.ts`, `todoTags`).
 */
export interface ReplacementTodo extends Badged {
  kind: "replacement";
  key: string;
  /** Since when the oldest of its jobs waits (Unix ms). */
  at: number;
  work: WorkRef | null;
  title: string;
  season: number | null;
  /** The episodes that wait for a decision, as numbers. */
  episodes: number[];
  /** `episodes` as runs. */
  episode_segments: EpisodeSegment[];
  creator: string | null;
  /** The job the card's `비교` opens. */
  job_id: string;
  /** How many jobs of the work wait for a decision. */
  jobs: number;
  /** What the open plans counted in `episodes` change, summed. */
  changes: ReplacementChanges;
  /**
   * When the open plans' current and new subtitles were received, the newest of each (Unix ms): the work detail's
   * card line. `current_changed_at` is the newest change time of the current files when the app manages none of them.
   */
  current_received_at: number | null;
  current_changed_at: number | null;
  new_received_at: number | null;
}

export type Todo =
  | AuthTodo
  | ReceiveFailedTodo
  | ReplacementTodo
  | EpisodeCheckTodo
  | PlacementCheckTodo
  | VideoCheckTodo;

export interface TodoList {
  /** Red kinds first (`인증 필요`, `받기 실패`), then `교체 승인`, then `회차 확인 필요`, each newest first. */
  needs: Todo[];
  /** What the menu badge shows: `needs.length`. */
  count: number;
  /** Each work's badges by its ID: the kinds of its to-dos, once each, in the list's order. A work without one is not in it. */
  badges: Record<string, TodoKind[]>;
}

export function fetchTodos(signal?: AbortSignal): Promise<TodoList> {
  return api<TodoList>("/todo", { signal });
}

/**
 * `자막 구독`: a subscription that gets subtitles with no creator chosen yet,
 * while its anime has candidates. It names no creator; the user picks one in
 * the work detail's 자막 후보. A suggestion, never counted in the badge.
 */
export interface FollowSuggestion {
  work: WorkRef;
  /** The anime's Anissia title, else the work's name. */
  title: string;
  season: number;
  rule_id: string;
  anime_no: number;
  /** The candidates' episodes as Anissia writes them, once each. */
  episodes: string[];
  /** `episodes` as runs. */
  episode_segments: EpisodeSegment[];
  /** How many creators have candidates. */
  creators: number;
  /** When the first candidate was seen (Unix ms). */
  since: number;
}

export function fetchFollowSuggestions(
  signal?: AbortSignal,
): Promise<FollowSuggestion[]> {
  return api<{ suggestions: FollowSuggestion[] }>("/todo/subtitle-follow", {
    signal,
  }).then((r) => r.suggestions);
}

/** The menu badge's count alone. */
export function fetchTodoCount(signal?: AbortSignal): Promise<number> {
  return api<{ count: number }>("/todo/count", { signal }).then((r) => r.count);
}

// ---------------------------------------------------------------------------
// Subtitle jobs

/**
 * Where a job is.
 *
 * - `pending`: accepted, no worker has started it (`시작 대기`).
 * - `running`: a worker is carrying it out (`stage` says where).
 * - `waiting`: it cannot go on until something happens (`wait` says what).
 * - `held`: a restart found a file whose receipt cannot be confirmed; it stops
 *   there rather than receive again or claim success (`보류`).
 * - `failed`: every item failed; `partial`: some failed and the rest were
 *   received (`일부 실패`).
 * - `done`: every item was received.
 */
export type JobState =
  "pending" | "running" | "waiting" | "held" | "failed" | "partial" | "done";

/** What a `waiting` job or item waits for: a person's check (`인증 필요`) or a source it cannot read yet (`자막 대기`). */
/**
 * What a waiting job or item waits for: a site's check (`auth`), a way to receive or a later build's analysis
 * (`subtitle`), a person to say where its files go (`placement`, `회차 확인 필요`), the approval of a replacement
 * (`approval`), or its video (`video`).
 */
export type Wait = "auth" | "subtitle" | "placement" | "approval" | "video";

/**
 * The class of a failure, the same for every source (`docs/specs/jobs.md`, 공통 수신
 * 결과와 실패 분류): the post or file is gone (`missing`), its signed address was
 * refused even after reading the post again (`expired`), what came was not the file
 * (`not_a_file`), the post holds nothing the source can read (`changed`), or the site
 * could not be reached (`network`). An item can also fail because the images of the
 * post hold no subtitle (`no_subtitle`) or one needs a key only a person can give
 * (`needs_input`).
 */
export type FailureClass =
  "missing" | "expired" | "not_a_file" | "changed" | "network" | "no_subtitle" | "needs_input";

/** What received bytes were checked to be; `other` is kept for the analysis to decide. */
export type FileFormat = "zip" | "ass" | "srt" | "smi" | "other";

/** The format of an uploaded archive. */
export type ArchiveType = "zip" | "rar" | "7z" | "gz" | "bz2" | "xz" | "tar";

/** What an uploaded file was judged to be by its content. */
export type UploadKind = "subtitle" | "font" | "archive";

/**
 * The steps a job goes through, in this order. `auth` only when a source asks for it. An upload job goes
 * `receive` (at once), `placement` (배치 확인: the analysis, then the person's confirmation), `store` and `apply`; a
 * find job `open`, `receive`, `placement`, `store` and `apply`, as it reaches them.
 */
export type StepKind = "found" | "open" | "auth" | "receive" | "placement" | "store" | "approval" | "apply";

export interface JobRow {
  id: string;
  /**
   * `pick`: the user picked its candidates; `auto`: the subscribed creator's, made by the app;
   * `upload`: subtitles and fonts the user uploaded (they are received at once, then the job waits for the worker's
   * analysis and the person's 배치 확인); `find`: the user browses the chosen creator's posts in the server browser
   * and every download is a file of the job (직접 찾기), then it waits for the 배치 확인 too; `relocate`: the app's
   * move of a source's applied copies after its episode mapping changed (재배치), with no episodes, waiting for its
   * 배치 확인 from the start.
   */
  origin: "pick" | "auto" | "upload" | "find" | "relocate";
  /**
   * For a find job: its 받기 has not ended, so its remote screen and `받기 끝내기` still apply. `false` for every
   * other job and for a find job after it finished receiving.
   */
  receiving: boolean;
  /** For a revision of a received subtitle: the observation received before. */
  revision_of: number | null;
  /** The latest job that received `revision_of`, while there is one. */
  revises_job: string | null;
  /** A revision of a subtitle file whose creator the user named; `revision_of` and `revises_job` are `null` then. */
  revises_attributed: boolean;
  state: JobState;
  wait: Wait | null;
  /** The step a `running` job is at. */
  stage: StepKind | null;
  /** One sentence about the state: why it waits, what failed, what is unclear. */
  note: string | null;
  /** Since when the job is in this state (Unix ms); for `done`, `failed` and `partial` when it ended. */
  state_at: number;
  created_at: number;
  work: WorkRef | null;
  /** What to call it: the Anissia title of the job's anime, else the work's name. */
  title: string;
  season: number | null;
  /** The items' episodes in order, as Anissia writes them. */
  episodes: string[];
  /** `episodes` as runs, in numeric order. */
  episode_segments: EpisodeSegment[];
  creator: string | null;
  /** The host of the posts (`kairan03.blogspot.com`). */
  source: string | null;
  /** Items received, failed, and all of them. */
  progress: { done: number; failed: number; total: number };
  /** The class of the first failed item's failure, when it has one. */
  failure: FailureClass | null;
  /** For an upload or a find job: what it kept, by kind, and how many files it dropped. */
  upload: UploadSummary | null;
  /** A find job the user finished, which ends once no download of its server browser is on its way. */
  finishing: boolean;
}

export interface UploadSummary {
  subtitles: number;
  fonts: number;
  archives: number;
  dropped: number;
}

export interface DonePage {
  /** Newest first. */
  items: JobRow[];
  /** What `after` takes for the next page; `null` at the end. */
  next: string | null;
  /** How many jobs are done in all. */
  total: number;
}

export interface JobGroups {
  /** `failed` and `partial`, newest first. */
  failed: JobRow[];
  /** `waiting` (a person's check first), `held`, `pending`. */
  waiting: JobRow[];
  running: JobRow[];
  /** The first five done jobs. */
  done: DonePage;
}

export function fetchJobs(signal?: AbortSignal): Promise<JobGroups> {
  return api<JobGroups>("/subtitle-jobs", { signal });
}

/** The done jobs after `after` (a page's `next`). */
export function fetchDoneJobs(
  after: string,
  signal?: AbortSignal,
): Promise<DonePage> {
  return api<DonePage>(
    `/subtitle-jobs/done?after=${encodeURIComponent(after)}&limit=20`,
    { signal },
  );
}

/**
 * A step as the job went through it. Steps it has not reached are `upcoming`
 * with no time. `waiting` is a step that stopped for something (`인증`
 * waiting for a person); `partial` a `받기` where some items failed.
 */
export interface Step {
  step: StepKind;
  state: "upcoming" | "current" | "waiting" | "done" | "failed" | "partial";
  at: number | null;
  note: string | null;
}

export type ItemState =
  "pending" | "running" | "waiting" | "held" | "failed" | "done";

export interface JobFile {
  /** The receipt's ID, which a placement's `file_id` names. */
  id: string;
  /** The file's name as the site gave it. */
  name: string;
  /** The folders the post shows it in (`회차/2화`), when it does. */
  folder: string | null;
  /** `receiving` while bytes come in; `held`: its receipt could not be confirmed after a restart. */
  state: "receiving" | "done" | "held" | "failed";
  /** Bytes received. */
  size: number | null;
  sha256: string | null;
  /** Where it is in the receive area (absolute, as the server sees it); `null` once it left the area, stored. */
  path: string | null;
  /** The episode of the item that received this same file, when another item of the job did. */
  shared_with: string | null;
  reason: string | null;
  /** What its bytes were checked to be, once received. */
  format: FileFormat | null;
  /** The class of a failed file's failure. */
  failure: FailureClass | null;
  /** The answer's HTTP status and media type, when it came over HTTP. */
  http_status: number | null;
  content_type: string | null;
  /** For a failure: how many bytes the answer that showed it had. */
  response_size: number | null;
  /** For an uploaded file: what its content was judged to be. */
  kind: UploadKind | null;
  /** For an uploaded archive: its format, by its first bytes. */
  archive: ArchiveType | null;
  /** For a received archive: what came of unpacking it; `null` before it was tried, and for a file that is no archive. */
  unpack: UnpackResult | null;
  /** A Google Drive font not received because its size and `Last-Modified` are those of the stored font it uses. */
  unchanged: boolean;
  /**
   * For a received archive (received whole) whose files are all settled: how many of them became new stored files, 0
   * when each was kept already; `null` before, and for a file that is no archive.
   */
  new_assets: number | null;
}

/** A file of an upload that was not kept, with why. */
export interface DroppedFile {
  name: string;
  reason: string;
}

/** One episode of a job: one candidate the user picked. */
export interface JobItem {
  id: number;
  episode: string;
  /** The post the candidate names: a public page, never a signed download address. */
  post_url: string;
  state: ItemState;
  wait: Wait | null;
  reason: string | null;
  /** The class of a failed item's failure. */
  failure: FailureClass | null;
  /**
   * On a revision: the job of the earlier receipt whose files are the same bytes,
   * so there is nothing to replace; `null` otherwise.
   */
  unchanged_from: string | null;
  files: JobFile[];
}

export interface LogEntry {
  at: number;
  /** What happened, as a sentence to show as is. */
  message: string;
  /** Shown dimmed beside it (a size, a host). */
  detail: string | null;
}

export interface JobDetail extends JobRow {
  steps: Step[];
  items: JobItem[];
  /** What became of each received file, in the order they were planned. */
  placements: Placement[];
  /** The latest replacement plan of each row whose episode had a subtitle. */
  replacements: Replacement[];
  /** The job's folder in the receive area (absolute, as the server sees it). */
  receive_dir: string;
  /** Newest first. */
  log: LogEntry[];
  /** For an upload or a find job: the files left out, in the order they were named or downloaded. */
  dropped: DroppedFile[];
  /**
   * The remote screen of a job that waits for a site's check in the server
   * browser (`screen_api.rs`); `null` when it has none.
   */
  screen: JobScreen | null;
  /** The 배치 확인 table's data while the job waits for it (`wait` is `placement`); `null` otherwise. */
  confirm: ConfirmView | null;
  /** A relocation's removals of applied copies (재배치); empty for another job. */
  relocations: Relocation[];
}

/** A job's remote screen, as `screen_api.rs` describes it. */
export interface JobScreen {
  /**
   * `ready`: a browser run shows the check (connect to `run`); `preparing`:
   * the worker is bringing the page to it; `closed`: no run shows it (`note`
   * says why), opening the page prepares it again; `unavailable`: this server
   * has no server browser.
   */
  state: "ready" | "preparing" | "closed" | "unavailable";
  run: string | null;
  /**
   * With `run`: when the run was bound to the job (ms). Another check of the
   * job in the same run is a new binding, with a later `bound`.
   */
  bound: number | null;
  note: string | null;
  /** With `run`: the page shown is not the one the run was bound with (a popup). */
  popup: boolean;
}

export function fetchJob(id: string, signal?: AbortSignal): Promise<JobDetail> {
  return api<JobDetail>(`/subtitle-jobs/${encodeURIComponent(id)}`, { signal });
}

/**
 * A person opened the job's page: asks the worker to bring the page to its
 * check again if its server browser is gone, and counts as use of a live one.
 * Answers the screen as it is now, or `null` when the job has none. Only the
 * opening of the page (and the person's `다시 열기`) sends this; reading the
 * job, polling and reconnecting never do.
 */
export function prepareScreen(id: string): Promise<JobScreen | null> {
  return api<JobScreen | null>(`/subtitle-jobs/${encodeURIComponent(id)}/screen`, { method: "POST" });
}

/**
 * Closes the tab `target` of the job's screen, of the binding (`run`, `bound`) the person sees, shown or not. The worker
 * closes it in the server browser, never the run's first page. A page that is not shown leaves the screen as it is; the
 * one shown goes to the newest page left, as a new binding. A `conflict` when the binding changed or the page cannot be
 * closed.
 */
export async function closeTab(id: string, run: string, bound: number, target: string): Promise<void> {
  await api<unknown>(`/subtitle-jobs/${encodeURIComponent(id)}/screen/close`, {
    method: "POST",
    body: { run, bound, target },
  });
}

/**
 * Shows the tab `target` on the job's screen, of the binding (`run`, `bound`) the person sees: the worker moves the
 * screen to it as a new binding. A `conflict` when the binding changed or the page is not one of the tabs.
 */
export async function switchTab(id: string, run: string, bound: number, target: string): Promise<void> {
  await api<unknown>(`/subtitle-jobs/${encodeURIComponent(id)}/screen/switch`, {
    method: "POST",
    body: { run, bound, target },
  });
}

/**
 * Asks the worker to start the job's server browser run anew, for the binding (`run`, `bound`) the person sees: it ends
 * the run (once no download of it is on its way) and opens the post again in a new run, a new binding. Asked when the
 * page does not answer. A `conflict` when the binding changed.
 */
export async function restartScreen(id: string, run: string, bound: number): Promise<void> {
  await api<unknown>(`/subtitle-jobs/${encodeURIComponent(id)}/screen/restart`, {
    method: "POST",
    body: { run, bound },
  });
}

/**
 * A person's decision on a replacement: `replace` (`새 자막으로 교체`) or `keep` (`현재 유지`), for the plan version
 * the card showed. Answers the plan's new state. A `conflict` when that version is not the one to decide any more
 * (a newer comparison replaced it, or it was decided): the job is read again; `not_found` for a plan the job lacks.
 */
export function decideReplacement(
  jobId: string,
  planId: string,
  version: number,
  decision: "replace" | "keep",
  signal?: AbortSignal,
): Promise<{ state: "approved" | "kept" }> {
  return api<{ state: "approved" | "kept" }>(
    `/subtitle-jobs/${encodeURIComponent(jobId)}/replacements/${encodeURIComponent(planId)}`,
    { method: "POST", body: { version, decision }, signal },
  );
}

/**
 * Several decisions on a job's replacements at once (`모두 교체`·`모두 유지`). Each is bound to the plan and version
 * it names, so a plan made again since the list was read comes back `stale` and the others are written. `not_found`,
 * with nothing written, when one is not this job's plan.
 */
export function decideReplacements(
  jobId: string,
  decisions: readonly ReplacementDecision[],
  signal?: AbortSignal,
): Promise<{ results: ReplacementResult[] }> {
  return api<{ results: ReplacementResult[] }>(`/subtitle-jobs/${encodeURIComponent(jobId)}/replacements`, {
    method: "POST",
    body: { decisions },
    signal,
  });
}

/**
 * The dialogue and timing lines of a plan's comparison. Read when a person first opens 대사 or 타이밍, not with the
 * job: a whole-file change can be hundreds of KB. `not_found` for a plan that is not this job's or was not compared.
 */
export function fetchReplacementLines(jobId: string, planId: string, signal?: AbortSignal): Promise<ReplacementLines> {
  return api<ReplacementLines>(
    `/subtitle-jobs/${encodeURIComponent(jobId)}/replacements/${encodeURIComponent(planId)}/lines`,
    { signal },
  );
}

/**
 * A person's answer to a job's 배치 확인: one entry for each of `confirm.positions`, and for a relocation the `id` of
 * each planned removal it showed. The job becomes `pending` and the worker stores and applies it. `invalid` (a
 * Korean sentence to show as is) for a placing that cannot be kept; a `conflict` when the job no longer waits or its rows changed: the job is read again; `not_found` for no job.
 */
export function confirmPlacement(
  jobId: string,
  rows: readonly PlacementChoice[],
  removals: readonly string[] = [],
  signal?: AbortSignal,
): Promise<{ applied: number; stored: number }> {
  return api<{ applied: number; stored: number }>(`/subtitle-jobs/${encodeURIComponent(jobId)}/placement`, {
    method: "POST",
    body: { rows, removals },
    signal,
  });
}

/** The most candidates one job takes, as the server (`MAX_CANDIDATES` in `jobs_api.rs`) does. */
export const MAX_JOB_CANDIDATES = 200;

/** What creates a job: the candidates are the observation IDs of one creator, in the order the job lists them. */
export interface NewSubtitleJob {
  /** Made by the browser for one action; the same ID with the same content is the same job. */
  id: string;
  work_id: string;
  season: number;
  candidates: number[];
}

/**
 * Creates a subtitle job for the candidates. `202` when it was made now and
 * `200` when the same ID with the same content was made before: both give the
 * job's ID. The same ID with other content is a `conflict`.
 */
export function createSubtitleJob(
  job: NewSubtitleJob,
): Promise<{ id: string }> {
  return api<{ id: string }>("/subtitle-jobs", { method: "POST", body: job });
}

/** Where a job's detail is. */
export function jobPath(id: string): string {
  return `/todo/job/${encodeURIComponent(id)}`;
}

/** What makes a find job (직접 찾기): the browser's ID for the action, the season and a candidate source's ID. */
export interface NewFindJob {
  id: string;
  work_id: string;
  season: number;
  creator: string;
}

/**
 * Creates a find job: the server browser opens the creator's most recently observed post for the user to browse
 * on the job's remote screen. `202` when it was made now and `200` when the same ID with the same content was made
 * before: both give the job's ID. The same ID with other content is a `conflict`.
 */
export function createFindJob(job: NewFindJob): Promise<{ id: string }> {
  return api<{ id: string }>("/subtitle-jobs/find", { method: "POST", body: job });
}

/**
 * Finishes a find job (`받기 끝내기`): the request is written and the worker ends the job once no download of its
 * server browser is on its way (`finishing` until then); `done` for a job that had ended.
 */
export function finishJob(id: string): Promise<{ state: "done" | "finishing" }> {
  return api<{ state: "done" | "finishing" }>(
    `/subtitle-jobs/${encodeURIComponent(id)}/finish`,
    { method: "POST" },
  );
}

/** What an upload answers when it made the job now (`202`): the files kept by kind and the files dropped. */
export interface UploadResult {
  id: string;
  /** Absent when the same upload was stored before (`200`). */
  kept?: { subtitles: number; fonts: number; archives: number };
  dropped?: DroppedFile[];
}

/** What an upload is made of: the browser's ID for it, the season, the creator and the files. */
export interface NewUpload {
  id: string;
  work_id: string;
  season: number;
  /** A candidate source's ID, or `null` for `제작자 알 수 없음`. */
  creator: string | null;
  /** The files to store, each with the name or folder path it is sent under. */
  files: { name: string; file: Blob }[];
  /** The names of the files the browser left out. */
  skipped: string[];
}

/**
 * Uploads subtitles and fonts as one job. The metadata parts come before the
 * files, so the server checks the season and the creator before it takes any
 * bytes. `202` when the job was made now and `200` when the same ID with the
 * same upload was stored before. The same ID with another upload is a `conflict`.
 */
export function uploadSubtitles(
  upload: NewUpload,
  signal?: AbortSignal,
): Promise<UploadResult> {
  const form = new FormData();
  form.append("id", upload.id);
  form.append("work_id", upload.work_id);
  form.append("season", String(upload.season));
  form.append("creator", upload.creator ?? "");
  for (const name of upload.skipped) form.append("skipped", name);
  for (const { name, file } of upload.files) form.append("file", file, name);
  return api<UploadResult>("/subtitle-jobs/upload", {
    method: "POST",
    body: form,
    signal,
  });
}
