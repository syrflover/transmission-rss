import { api } from "@/lib/api";

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

/**
 * `인증 필요`: subtitle jobs of one work wait for a person to solve a site's
 * check. One to-do per work; it opens the oldest such job.
 */
export interface AuthTodo {
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
export interface ReceiveFailedTodo {
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
export interface EpisodeCheckTodo {
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
  /** Why the mapping is undecided; `null` when the mapping is decided and episodes do not fit. */
  reason: string | null;
  /** How many seasons and creators of the work need a check. */
  sources: number;
}

export type Todo = AuthTodo | ReceiveFailedTodo | EpisodeCheckTodo;

export interface TodoList {
  /** Red kinds first (`인증 필요`, `받기 실패`), then `회차 확인 필요`, each newest first. */
  needs: Todo[];
  /** What the menu badge shows: `needs.length`. */
  count: number;
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
export type Wait = "auth" | "subtitle";

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

/** The steps a job goes through, in this order. `auth` only when a source asks for it. An upload job has only `receive`. */
export type StepKind = "found" | "open" | "auth" | "receive";

export interface JobRow {
  id: string;
  /**
   * `pick`: the user picked its candidates; `auto`: the subscribed creator's, made by the app;
   * `upload`: subtitles and fonts the user uploaded (it is `done` from the start).
   */
  origin: "pick" | "auto" | "upload";
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
  creator: string | null;
  /** The host of the posts (`kairan03.blogspot.com`). */
  source: string | null;
  /** Items received, failed, and all of them. */
  progress: { done: number; failed: number; total: number };
  /** The class of the first failed item's failure, when it has one. */
  failure: FailureClass | null;
  /** For an upload job: what it kept, by kind, and how many files it dropped. */
  upload: UploadSummary | null;
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
  /** The file's name as the site gave it. */
  name: string;
  /** The folders the post shows it in (`회차/2화`), when it does. */
  folder: string | null;
  /** `receiving` while bytes come in; `held`: its receipt could not be confirmed after a restart. */
  state: "receiving" | "done" | "held" | "failed";
  /** Bytes received. */
  size: number | null;
  sha256: string | null;
  /** Where it is in the receive area (absolute, as the server sees it). */
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
  /** The job's folder in the receive area (absolute, as the server sees it). */
  receive_dir: string;
  /** Newest first. */
  log: LogEntry[];
  /** For an upload job: the files left out, in the order they were named. */
  dropped: DroppedFile[];
}

export function fetchJob(id: string, signal?: AbortSignal): Promise<JobDetail> {
  return api<JobDetail>(`/subtitle-jobs/${encodeURIComponent(id)}`, { signal });
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
