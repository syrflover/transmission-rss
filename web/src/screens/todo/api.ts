import { api } from "@/lib/api";
import type {
  CloseTabBody,
  ConfirmPlacementBody,
  DecideAnswer,
  DecideBody,
  DecideManyAnswer,
  DecideManyBody,
  FinishAnswer,
  FollowSuggestions,
  FollowSuggestionView,
  JobCreated,
  JobDetail,
  JobGroups,
  JobScreen,
  DonePage,
  NewFindJob,
  NewSubtitleJob,
  PlacementChoice,
  PlacementConfirmed,
  ReplacementDecision,
  ReplacementLines,
  RestartAnswer,
  RestartScreenBody,
  SwitchTabBody,
  TodoCount,
  TodoList,
  TodoWithBadge,
  UploadAnswer,
  VideoCheckBody,
} from "@/lib/api-types";

/*
 * The shapes of these APIs are generated from the Rust types (`lib/api-types`, `cargo types`); the types below are the
 * ones the screens import from here. A type of this file that is not re-exported is a shape of the screen's own.
 */
export type {
  ArchiveType,
  ComparedItem,
  ComparedSide,
  Comparison,
  ConfirmEpisode,
  ConfirmView,
  DialogueLine,
  DonePage,
  DroppedFile,
  EpisodeSegment,
  FailureKind,
  FileFormat,
  FollowSuggestionView,
  ItemState,
  JobCreated,
  JobDetail,
  JobFile,
  JobGroups,
  JobItem,
  JobRow,
  JobScreen,
  JobState,
  JobStep,
  LogEntry,
  NewFindJob,
  NewSubtitleJob,
  Outcome,
  PlacedText,
  Placement,
  PlacementChoice,
  PlanState,
  Relocation,
  Replacement,
  ReplacementChanges,
  ReplacementDecision,
  ReplacementLines,
  ReplacementPath,
  ReplacementVersion,
  StepKind,
  StyleChange,
  TimingLine,
  TodoWithBadge,
  UnpackResult,
  UploadAnswer,
  UploadKind,
  UploadSummary,
  ItemWait,
  WorkRef,
} from "@/lib/api-types";
export type { TodoList };

/**
 * The 할 일 screen and the job detail (`docs/requirements/jobs.md`, 할 일 화면 and
 * 작업 상세) read these: the to-dos that need the user (`/api/todo`,
 * `src/web/todo_api.rs`) and the subtitle jobs the worker carries out
 * (`/api/subtitle-jobs`, `src/web/jobs_api.rs`). The suggestions (`제목 후보`,
 * `보관 제안`) come from the collect screen's own sources
 * (`collect/subs/api.ts`, `collect/archive/api.ts`); `자막 구독` from
 * `/api/todo/subtitle-follow`.
 */

// ---------------------------------------------------------------------------
// To-dos that need handling (`처리 필요`)

/**
 * One kind of to-do. The server's `Todo` is one enum tagged by `kind`, so the generated `TodoWithBadge` is a union
 * and each kind is that union narrowed.
 */
export type AuthTodo = Extract<TodoWithBadge, { kind: "auth" }>;
export type ReceiveFailedTodo = Extract<TodoWithBadge, { kind: "receive_failed" }>;
export type EpisodeCheckTodo = Extract<TodoWithBadge, { kind: "episode_check" }>;
export type PlacementCheckTodo = Extract<TodoWithBadge, { kind: "placement_check" }>;
export type VideoCheckTodo = Extract<TodoWithBadge, { kind: "video_check" }>;
export type ReplacementTodo = Extract<TodoWithBadge, { kind: "replacement" }>;

/**
 * `확인함` on a video a `video_check` to-do asks about: it is no longer asked about while it is the video at the path.
 * A `409` (`conflict`) is another video there since the list was read; a `404` is one renamed or moved.
 */
export function checkVideo(workId: string, path: string, seen: string): Promise<void> {
  const body: VideoCheckBody = { path, seen };
  return api<void>(`/library/works/${encodeURIComponent(workId)}/videos/check`, { method: "POST", body });
}

export function fetchTodos(signal?: AbortSignal): Promise<TodoList> {
  return api<TodoList>("/todo", { signal });
}

export function fetchFollowSuggestions(
  signal?: AbortSignal,
): Promise<FollowSuggestionView[]> {
  return api<FollowSuggestions>("/todo/subtitle-follow", {
    signal,
  }).then((r) => r.suggestions);
}

/** The menu badge's count alone. */
export function fetchTodoCount(signal?: AbortSignal): Promise<number> {
  return api<TodoCount>("/todo/count", { signal }).then((r) => r.count);
}

// ---------------------------------------------------------------------------
// Subtitle jobs

export function fetchJobs(signal?: AbortSignal): Promise<JobGroups> {
  return api<JobGroups>("/subtitle-jobs", { signal });
}

/** The done jobs after `after` (a page's `next`). */
export function fetchDoneJobs(
  after: string,
  signal?: AbortSignal,
): Promise<DonePage> {
  return api<DonePage>(
    `/subtitle-jobs/done?after=${encodeURIComponent(after)}`,
    { signal },
  );
}

export function fetchJob(id: string, signal?: AbortSignal): Promise<JobDetail> {
  return api<JobDetail>(`/subtitle-jobs/${encodeURIComponent(id)}`, { signal });
}

/**
 * A person opened the job's page: asks the worker to bring the page to its
 * check again if its server browser is gone, and counts as use of a live one.
 * Answers the screen as it is now, or `null` when the job has none. Only the
 * opening of the page (and the person's `다시 열기`) sends this; reading the
 * job, again on a change or after reconnecting, never does.
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
  const body: CloseTabBody = { run, bound, target };
  await api<unknown>(`/subtitle-jobs/${encodeURIComponent(id)}/screen/close`, { method: "POST", body });
}

/**
 * Shows the tab `target` on the job's screen, of the binding (`run`, `bound`) the person sees: the worker moves the
 * screen to it as a new binding. A `conflict` when the binding changed or the page is not one of the tabs.
 */
export async function switchTab(id: string, run: string, bound: number, target: string): Promise<void> {
  const body: SwitchTabBody = { run, bound, target };
  await api<unknown>(`/subtitle-jobs/${encodeURIComponent(id)}/screen/switch`, { method: "POST", body });
}

/**
 * Asks the worker to start the job's server browser run anew, for the binding (`run`, `bound`) the person sees: it ends
 * the run (once no download of it is on its way) and opens the post again in a new run, a new binding. Asked when the
 * page does not answer. A `conflict` when the binding changed.
 */
export async function restartScreen(id: string, run: string, bound: number): Promise<void> {
  const body: RestartScreenBody = { run, bound };
  await api<unknown>(`/subtitle-jobs/${encodeURIComponent(id)}/screen/restart`, { method: "POST", body });
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
  decision: DecideBody["decision"],
  signal?: AbortSignal,
): Promise<DecideAnswer> {
  const body: DecideBody = { version, decision };
  return api<DecideAnswer>(
    `/subtitle-jobs/${encodeURIComponent(jobId)}/replacements/${encodeURIComponent(planId)}`,
    { method: "POST", body, signal },
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
): Promise<DecideManyAnswer> {
  const body: DecideManyBody = { decisions: [...decisions] };
  return api<DecideManyAnswer>(`/subtitle-jobs/${encodeURIComponent(jobId)}/replacements`, {
    method: "POST",
    body,
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
): Promise<PlacementConfirmed> {
  const body: ConfirmPlacementBody = { rows: [...rows], removals: [...removals] };
  return api<PlacementConfirmed>(`/subtitle-jobs/${encodeURIComponent(jobId)}/placement`, {
    method: "POST",
    body,
    signal,
  });
}

/**
 * Creates a subtitle job for the candidates. `202` when it was made now and
 * `200` when the same ID with the same content was made before: both give the
 * job's ID. The same ID with other content is a `conflict`.
 */
export function createSubtitleJob(job: NewSubtitleJob): Promise<JobCreated> {
  return api<JobCreated>("/subtitle-jobs", { method: "POST", body: job });
}

/** Where a job's detail is. */
export function jobPath(id: string): string {
  return `/todo/job/${encodeURIComponent(id)}`;
}

/**
 * Creates a find job: the server browser opens the creator's most recently observed post for the user to browse
 * on the job's remote screen. `202` when it was made now and `200` when the same ID with the same content was made
 * before: both give the job's ID. The same ID with other content is a `conflict`.
 */
export function createFindJob(job: NewFindJob): Promise<JobCreated> {
  return api<JobCreated>("/subtitle-jobs/find", { method: "POST", body: job });
}

/**
 * Finishes a find job (`받기 끝내기`): the request is written and the worker ends the job once no download of its
 * server browser is on its way (`finishing` until then); `done` for a job that had ended.
 */
export function finishJob(id: string): Promise<FinishAnswer> {
  return api<FinishAnswer>(
    `/subtitle-jobs/${encodeURIComponent(id)}/finish`,
    { method: "POST" },
  );
}

/**
 * Starts an `interrupted` job again (`다시 시작`): it is `running` from the step it was cut in, and the worker is rung.
 * A job that is not interrupted any more is a `conflict`.
 */
export function restartJob(id: string): Promise<RestartAnswer> {
  return api<RestartAnswer>(`/subtitle-jobs/${encodeURIComponent(id)}/restart`, { method: "POST" });
}

/** What an upload is made of: the browser's ID for it, the season (and movie), the creator and the files. */
export interface NewUpload {
  id: string;
  work_id: string;
  season: number;
  /** For an upload to a movie: its episode number, an episode of season 0 (`season` is then `0`). */
  movie?: number;
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
): Promise<UploadAnswer> {
  const form = new FormData();
  form.append("id", upload.id);
  form.append("work_id", upload.work_id);
  form.append("season", String(upload.season));
  if (upload.movie !== undefined) form.append("movie", String(upload.movie));
  form.append("creator", upload.creator ?? "");
  for (const name of upload.skipped) form.append("skipped", name);
  for (const { name, file } of upload.files) form.append("file", file, name);
  return api<UploadAnswer>("/subtitle-jobs/upload", {
    method: "POST",
    body: form,
    signal,
  });
}
