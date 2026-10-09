import { api, ApiError } from "@/lib/api";
import { forget, forgetPrefix } from "@/lib/cached";
import type { Command } from "@/lib/commands";

import { forgetWeek } from "../schedule/api";
import type { EpisodeSegment } from "../todo/episodeLine.ts";
import type { TodoKind } from "../todo/kinds";
import type { FormatOrder, SubtitleFormat, WorkSubtitles } from "./detail/subtitles.ts";
import type { WorkStorage } from "./storage.ts";

export type { FormatOrder, SubtitleCopy, SubtitleCreatorCopies, SubtitleFormat, WorkSubtitles } from "./detail/subtitles.ts";

export type {
  AssetKind,
  CleanAsset,
  CleanKind,
  CleanableEntry,
  CleaningEntry,
  KeptAsset,
  StorageKind,
  StorageOverview,
  StorageWork,
  WorkStorage,
} from "./storage.ts";

/**
 * The library list (`src/web/library_api.rs`): the works with the summary the
 * list shows, one page at a time. Times are Unix milliseconds, `null` when unknown.
 */

/** A run of consecutive episodes, written as in the file names (`first === last` for one). */
export interface EpisodeRange {
  first: string;
  last: string;
}

/** How many of the episodes with a video also have a subtitle; `null` for a work whose folder is gone. */
export type SubtitleCoverage = "all" | "some" | "none";

export interface LibraryWork {
  id: string;
  /** The work's folder name. */
  name: string;
  /** The work's folder is not there any more (`폴더 없음`). */
  missing: boolean;
  watch_folder: { id: string; path: string };
  /** The highest season number recorded for the work. */
  latest_season: number | null;
  /** Episodes of the latest season that have a video / a subtitle. */
  video: EpisodeRange[];
  subtitle: EpisodeRange[];
  subtitle_coverage: SubtitleCoverage | null;
  /** A subtitle file the scan could not attach to an episode. */
  subtitle_check_needed: boolean;
  /** When the work first appeared. */
  added_at: number | null;
  /** The latest known time a video / subtitle was added, over every season. */
  video_added_at: number | null;
  subtitle_added_at: number | null;
  /** Where the cover image is served while the work has one; it may still fail (the placeholder stays). */
  cover_url: string | null;
  /** The kinds of its to-dos that need the person, the grid's badges on the cover, in the to-do list's order. */
  todos: TodoKind[];
}

/** How a page is ordered (`sort=` of the request). */
export type SortKey = "title" | "year" | "added" | "video" | "subtitle";
/** Which works a page lists (`filter=` of the request). */
export type FilterKey = "all" | "airing" | "complete" | "partial" | "none" | "check";

/** One page of the list. */
export interface LibraryWorkPage {
  items: LibraryWork[];
  /** Where the page after this one starts (`after=`); `null` after the last page. */
  next: string | null;
  /** How many works the filter and the search match, over every page. */
  total: number;
  /** How many works the library has, whatever the filter and the search. */
  library_count: number;
}

export interface WorkPageQuery {
  sort: SortKey;
  filter: FilterKey;
  /** The title text to search for; empty for none. */
  q: string;
  /** The `next` of the page before, none for the first page. */
  after?: string | null;
  /** How many works a page has; the server's default (60) when left out. */
  limit?: number;
}

export function loadWorkPage(query: WorkPageQuery, signal?: AbortSignal): Promise<LibraryWorkPage> {
  const params = new URLSearchParams({ sort: query.sort, filter: query.filter });
  if (query.q !== "") params.set("q", query.q);
  if (query.after) params.set("after", query.after);
  if (query.limit !== undefined) params.set("limit", String(query.limit));
  return api<LibraryWorkPage>(`/library/works?${params}`, { signal });
}

/** The cache keys that start with this hold the loaded pages of the list. */
export const LIST_PREFIX = "library:list:";
/** ...and these one work's page. */
export const WORK_PREFIX = "library:work:";

/** The cache key of the settings' stored size per work (`settings/storage`). */
export const STORAGE_KEY = "library:storage";

/**
 * Whoever changes which folders the library reads (adds, rescans or removes a
 * watch folder, sets the collect or archive folder) calls this: the loaded
 * pages of the list and every work's page are read again the next time.
 */
export function forgetLibrary(): void {
  forgetPrefix(LIST_PREFIX);
  forgetPrefix(WORK_PREFIX);
  forget(STORAGE_KEY);
  // The home screen's checklist ends with the first watch folder, and its cards link to works.
  forgetWeek();
}

/** Where a work opens (the work detail screen). */
export const workPath = (id: string) => `/library/${encodeURIComponent(id)}`;

// --- one work (`src/web/library_work_api.rs`) ---------------------------------------

/** A recorded file, by its path relative to the work folder. `added_at` is `null` when unknown. */
export interface WorkFile {
  path: string;
  added_at: number | null;
}

/** The creator the user named for a subtitle file: one creator of the season's Anissia anime. */
export interface FileCreator {
  /** The app's ID of the creator's lines of the anime. */
  source_id: string;
  /** The creator's name as Anissia gives it. */
  name: string;
  anime_no: number;
}

/**
 * A subtitle file. `creator` is `null` for `제작자 알 수 없음`, which every file found in a watch folder is until the
 * user names one; `creator_version` is what a change of it names, so a change made from an older version is refused.
 * `applied` is set for a copy trss applied beside a video: its creator is the stored copy's (`null` is
 * `제작자 알 수 없음`), it comes before a creator the user named for the path, so `creator` is `null` for it, and its
 * creator is not changed from here.
 */
export interface WorkSubtitle extends WorkFile {
  creator: FileCreator | null;
  creator_version: number;
  applied: { creator: string | null } | null;
}

/** The episode's video was replaced by a higher revision of the same release: the quiet version line. */
export interface EpisodeRevision {
  /** `v1`; `null` when the old video's revision was not known. */
  from: string | null;
  /** `v2`. */
  to: string;
  /** When the new video took the episode name (Unix milliseconds). */
  replaced_at: number;
}

/** One of the two videos of a replacement that failed, and what became of it. */
export interface FailureFile {
  role: "old" | "new";
  /** Relative to the work folder; `null` for a new video that was not received. */
  path: string | null;
  state: "kept" | "removed" | "received_name" | "missing" | "not_received";
}

/** A replacement of the episode's video that failed (`받기 실패`), with why and both files. */
export interface EpisodeFailure {
  at: number;
  /** The revision's history item, which `다시 받기` receives. */
  history_item_id: number;
  reason: string;
  files: FailureFile[];
  /** Whether `다시 받기` is offered: the revision's download stopped before it was received. */
  can_retry: boolean;
  /**
   * Why `다시 받기` is missing on such a revision, as a sentence; also `이미 같거나 더 높은 수정본(v3)이 있어서 다시
   * 받지 않아요.` when the episode's place is known to hold the same or a higher revision already.
   */
  retry_blocked: string | null;
  /** Its `다시 받기` command that has not ended yet. */
  command: Command | null;
}

export interface WorkEpisode {
  /** As written in the file names; `01` and `13`/`013` are one episode. */
  episode: string;
  /** The episode as a number, `null` when it is no number. */
  sort: number | null;
  /** When AniList schedules it (Unix milliseconds); only for a releasing entry with a schedule, else `null`. */
  air_at: number | null;
  video: WorkFile[];
  subtitle: WorkSubtitle[];
  /** Set when the video was replaced by a higher revision; `null` otherwise. */
  revision: EpisodeRevision | null;
  /** Set while a replacement of the video has failed; `null` otherwise. */
  failure: EpisodeFailure | null;
  /**
   * Stored subtitles on the episode with no applied copy beside its video (`보관본 있음`), oldest first. One whose
   * last comparison a person ended with `현재 유지` is left out while the episode has a subtitle.
   */
  stored: StoredSubtitle[];
}

/** A stored subtitle of an episode that is not applied beside its video. */
export interface StoredSubtitle {
  id: string;
  /** The stored file's name. */
  name: string;
  /** The creator it was received from; `null` is `제작자 알 수 없음`. */
  creator: string | null;
  format: "ass" | "srt" | "smi" | "other";
  /** When it was stored (Unix milliseconds). */
  stored_at: number;
  /** Whether `적용` can ask for it: a format the app applies, from a job whose record is there. */
  can_apply: boolean;
  /** A job applies it once the episode's video comes (`영상 대기`); it asks for no `적용`. */
  awaiting_video: boolean;
  /** The job waiting for the user to approve replacing the episode's subtitle with it (`교체 승인`); it asks for no `적용`. */
  approval_job: string | null;
  /** The episode has a subtitle: `적용` goes to the comparison (`교체 승인`), not to applying at once; missing is `false`. */
  compare?: boolean;
}

/** `apply` takes the stored copy as the episode's subtitle; `add` puts the same creator's other format beside it. */
export type ApplyMode = "apply" | "add";

/**
 * Asks to apply the stored subtitle `storedId` beside the episode's video; answers the job. `compare` is `true` when
 * the episode has a subtitle: the job compares and waits for the person's `교체 승인`, so the job's page is the next
 * step. A `409` is a refusal whose message is the sentence to show, a `404` says the copy is gone.
 */
export function applyStored(
  id: string,
  storedId: string,
  mode: ApplyMode = "apply",
): Promise<{ job_id: string; compare: boolean }> {
  return api<{ job_id: string; compare: boolean }>(
    `/library/works/${encodeURIComponent(id)}/stored/${encodeURIComponent(storedId)}/apply`,
    { method: "POST", body: { mode } },
  );
}

const orderPath = (id: string) => `/library/works/${encodeURIComponent(id)}/subtitle-order`;

/** Sets the work's own format order; a `400` has the sentence for a bad one. */
export function putSubtitleOrder(id: string, order: SubtitleFormat[]): Promise<FormatOrder> {
  return api<FormatOrder>(orderPath(id), { method: "PUT", body: { format_order: order } });
}

/** Takes the work's own order away; the answer is the global order. */
export function deleteSubtitleOrder(id: string): Promise<FormatOrder> {
  return api<FormatOrder>(orderPath(id), { method: "DELETE" });
}

/**
 * Asks the worker to delete the stored subtitle `storedId` and `assets`, the ids of the linked files the confirmation
 * listed; answers the cleanup's id. A `409` is a `conflict`: the stored file cannot be cleaned (its message is why),
 * or its files changed since (`current` is the new entry).
 */
export function cleanStored(id: string, storedId: string, assets: string[]): Promise<{ cleanup_id: string }> {
  return api<{ cleanup_id: string }>(
    `/library/works/${encodeURIComponent(id)}/stored/${encodeURIComponent(storedId)}/clean`,
    { method: "POST", body: { assets } },
  );
}

export interface WorkSeason {
  number: number;
  /** The AniList entries the season links, taken together. */
  info: SeasonInfo;
  /** The Anissia anime the season is linked to. */
  anissia: AnissiaLink;
  /** The episodes that have a video, as runs (`1–3`, `5`). */
  video_ranges: string[];
  /** The episodes that have a subtitle, as runs. */
  subtitle_ranges: string[];
  /** Ascending. */
  episodes: WorkEpisode[];
}

/** A file the scan could not attach to an episode, with why. */
export interface UnrecognizedFile {
  path: string;
  reason: string;
  /** The reason as a sentence. */
  message: string;
  /** A video the app asks about (`회차 확인 필요`) whose `확인함` holds for it. */
  checked: boolean;
}

/** A subscription rule connected to a season of the work. */
export interface WorkSubscription {
  season: number;
  rule_id: string;
  rule_version: number;
  rule_state: "active" | "paused" | "archived";
  anime_no: number;
  /** Anissia's title; `null` when the app has no snapshot of the anime. */
  subject: string | null;
  subtitles: "follow" | "undecided" | "none";
  creator: string | null;
}

/** A rule that saves into the work's folder. */
export interface WorkRule {
  id: string;
  channel: { id: string; name: string | null; host: string };
  match: string | null;
  /** Relative to the collect folder. */
  directory: string;
  save_path: string;
  state: "active" | "paused" | "archived";
}

export interface WorkDetail {
  id: string;
  name: string;
  missing: boolean;
  watch_folder: { id: string; path: string };
  folder_path: string;
  added_at: number | null;
  /** The first season's first linked AniList entry's native title. */
  native_title: string | null;
  /** Anissia's title of the anime the work's first connected season follows; `null` without one. */
  korean_title: string | null;
  /** The subscription rules connected to a season of this work, by season. */
  subscriptions: WorkSubscription[];
  /** Ascending by season number. */
  seasons: WorkSeason[];
  unrecognized: UnrecognizedFile[];
  rules: WorkRule[];
  /** Where the cover image is served while the work has one. */
  cover_url: string | null;
  /** The worker still has to receive the cover's image; the old image shows until then. */
  cover_pending: boolean;
  /** What the app stored for the work, and the stored subtitles that can be cleaned (`파일` card). */
  storage: WorkStorage;
  /** The received subtitles by creator and the format order (`자막` card); a server without them leaves it out. */
  subtitles?: WorkSubtitles;
}

/** The cache key of one work's page. */
export const workKey = (id: string) => `${WORK_PREFIX}${id}`;

/** One work, or `null` when the library has no work with this ID. */
export async function loadWork(id: string, signal?: AbortSignal): Promise<WorkDetail | null> {
  try {
    return await api<WorkDetail>(`/library/works/${encodeURIComponent(id)}`, { signal });
  } catch (e) {
    if (e instanceof ApiError && e.code === "not_found") return null;
    throw e;
  }
}

// --- a work's cover (`src/web/artwork_api.rs`) ----------------------------------------

/** `auto`: the app picks a clear AniList match; `manual`: the user chose; `disabled`: no cover, no search. */
export type ArtworkMode = "auto" | "manual" | "disabled";
/** The image file, checked when the state was read. Only `available` is shown. */
export type ImageStatus = "available" | "missing" | "mismatch" | "unverified";

export interface ArtworkImage {
  id: string;
  origin: "anilist" | "upload";
  format: "jpeg" | "png" | "webp";
  byte_size: number;
  status: ImageStatus;
  url: string;
}

export interface ArtworkState {
  mode: ArtworkMode;
  source: "anilist" | "upload" | null;
  anilist_media_id: number | null;
  /** Sent back with a change; a change from an older version is a `conflict`. */
  version: number;
  image: ArtworkImage | null;
  /** Automatic work still to do: looking the title up, or receiving the image. */
  pending: "search" | "fetch" | null;
  /** Why the last automatic attempt left the cover as it is. */
  note: { code: string; message: string } | null;
}

/** One AniList entry of a search. `thumb_url` is AniList's own image address. */
export interface AnilistCandidate {
  id: number;
  title: string;
  titles: string[];
  format: string | null;
  season_year: number | null;
  thumb_url: string | null;
}

export interface AnilistPage {
  items: AnilistCandidate[];
  has_next: boolean;
  page: number;
}

const artworkPath = (id: string) => `/library/works/${encodeURIComponent(id)}/artwork`;

export function loadArtwork(id: string, signal?: AbortSignal): Promise<ArtworkState> {
  return api<ArtworkState>(artworkPath(id), { signal });
}

export function searchAnilist(id: string, q: string, page: number, signal?: AbortSignal): Promise<AnilistPage> {
  return api<AnilistPage>(`${artworkPath(id)}/search`, { method: "POST", body: { q, page }, signal });
}

export function pickArtwork(id: string, version: number, anilistMediaId: number): Promise<ArtworkState> {
  return api<ArtworkState>(`${artworkPath(id)}/pick`, {
    method: "POST",
    body: { version, anilist_media_id: anilistMediaId },
  });
}

export function uploadArtwork(id: string, version: number, file: File): Promise<ArtworkState> {
  return api<ArtworkState>(`${artworkPath(id)}/upload?version=${version}`, { method: "POST", body: file });
}

/** `clear`: no cover and no automatic search; `auto`: back to automatic with a new search; `repair`: the chosen AniList entry's image again. */
export function changeArtwork(id: string, version: number, action: "clear" | "auto" | "repair"): Promise<ArtworkState> {
  return api<ArtworkState>(`${artworkPath(id)}/${action}`, { method: "POST", body: { version } });
}

/** The cover URL the list and the detail show for a state. */
export function coverUrlOf(state: ArtworkState): string | null {
  return state.image?.url ?? null;
}

// --- a season's info (`src/web/seasons_api.rs`) ---------------------------------------

/** A date AniList may know only in part. */
export interface FuzzyDate {
  year: number | null;
  month: number | null;
  day: number | null;
}

/** An AniList entry a season links. */
export interface SeasonEntry {
  id: number;
  title: string;
  romaji: string | null;
  english: string | null;
  native: string | null;
  format: string | null;
  status: string | null;
  episodes: number | null;
  start: FuzzyDate;
  end: FuzzyDate;
  url: string;
}

/** A sequel of the previous season's last entry, offered until the user confirms one. */
export interface SeasonSuggestion {
  id: number;
  title: string;
  romaji: string | null;
  english: string | null;
  native: string | null;
  format: string | null;
  status: string | null;
  start: FuzzyDate;
  url: string;
}

export type AiringState = "releasing" | "finished" | "not_yet_released" | "cancelled" | "hiatus";

export interface SeasonInfo {
  season: number;
  /** Sent back with a change; a change from an older version is a `conflict`. A season never touched has 0. */
  version: number;
  /** `auto`: the app linked the entry itself (or nothing is linked yet). */
  origin: "auto" | "user";
  /** The app still has the season's automatic search to do. */
  pending: "search" | null;
  /** Why the last automatic search linked nothing. */
  note: { code: string; message: string } | null;
  /** The work's first season: the only one the app searches for. */
  can_auto: boolean;
  entries: SeasonEntry[];
  airing: { start: FuzzyDate; end: FuzzyDate; state: AiringState | string | null } | null;
  /** `null` when unknown (no entry, or an entry's count is unknown). */
  episodes: number | null;
  studios: string[];
  genres: string[];
  anilist_url: string | null;
  /** The first entry's description as plain text paragraphs; `null` without one. */
  synopsis: string[] | null;
  suggestions: SeasonSuggestion[];
}

const seasonPath = (id: string, season: number) => `/library/works/${encodeURIComponent(id)}/seasons/${season}`;

export function loadSeasonInfo(id: string, season: number, signal?: AbortSignal): Promise<SeasonInfo> {
  return api<SeasonInfo>(`${seasonPath(id, season)}/info`, { signal });
}

export function searchSeason(id: string, season: number, q: string, page: number, signal?: AbortSignal): Promise<AnilistPage> {
  return api<AnilistPage>(`${seasonPath(id, season)}/search`, { method: "POST", body: { q, page }, signal });
}

/** Makes `anilistIds` the season's entries, in this order (empty unlinks). */
export function setSeasonLinks(id: string, season: number, version: number, anilistIds: number[]): Promise<SeasonInfo> {
  return api<SeasonInfo>(`${seasonPath(id, season)}/links`, { method: "POST", body: { version, anilist_ids: anilistIds } });
}

/** Unlinks the first season and asks for a new automatic search. */
export function restartSeasonAuto(id: string, season: number, version: number): Promise<SeasonInfo> {
  return api<SeasonInfo>(`${seasonPath(id, season)}/auto`, { method: "POST", body: { version } });
}

/** Asks AniList again for the season's entries, finished ones too. */
export function refreshSeason(id: string, season: number): Promise<SeasonInfo> {
  return api<SeasonInfo>(`${seasonPath(id, season)}/refresh`, { method: "POST", body: {} });
}

// --- a season's Anissia link (`src/web/seasons_anissia_api.rs`) -----------------------

/** The Anissia anime a season is linked to, as Anissia last listed it. */
export interface AnissiaAnime {
  anime_no: number;
  subject: string;
  original_subject: string | null;
  /** `ON`, `OFF`, or `END` for an anime that has finished. */
  status: string;
  /** The anime's page on Anissia. */
  url: string;
}

/** A subscription connected to the season: the season's anime is the subscription's. */
export interface AnissiaHolder {
  rule_id: string;
  anime_no: number;
  subject: string | null;
}

export interface AnissiaLink {
  season: number;
  /** Sent back with a change; a change from an older version is a `conflict`. A season never linked has 0. */
  version: number;
  anime: AnissiaAnime | null;
  /** Set while a subscription is connected to the season: the season's anime is the subscription's. */
  subscription: AnissiaHolder | null;
  /** Titles to read when writing a search: nothing is searched with them. No duplicates. */
  reference_titles: ReferenceTitle[];
}

/** A title of the season's linked AniList entries (`korean`: a synonym with Hangul) or the work's folder name. */
export interface ReferenceTitle {
  kind: "native" | "english" | "romaji" | "korean" | "folder";
  title: string;
}

/** An anime of Anissia's full list. */
export interface AnissiaCandidate {
  anime_no: number;
  subject: string;
  original_subject: string | null;
  status: string;
  /** 0 (Sunday) to 6 (Saturday), 7 (`기타`) or 8 (`신작`). */
  week: number;
  start_date: string | null;
  end_date: string | null;
  genres: string[];
  url: string;
}

export interface AnissiaPage {
  /** The text searched for. */
  q: string;
  items: AnissiaCandidate[];
  has_next: boolean;
  page: number;
}

/** Where a picked anime came from: the server checks it against that very list. */
export type AnissiaSource = { week: number } | { q: string; page: number };

const anissiaPath = (id: string, season: number) => `${seasonPath(id, season)}/anissia`;

/** Searches Anissia's full list (finished anime included) for the user's `q`, which is required. */
export function searchAnissia(id: string, season: number, q: string, page: number, signal?: AbortSignal): Promise<AnissiaPage> {
  return api<AnissiaPage>(`${anissiaPath(id, season)}/search`, { method: "POST", body: { q, page }, signal });
}

/** Links the season to `animeNo` (`null` cuts the link). */
export function setAnissiaLink(id: string, season: number, version: number, animeNo: number | null, source?: AnissiaSource): Promise<AnissiaLink> {
  return api<AnissiaLink>(`${anissiaPath(id, season)}/link`, { method: "POST", body: { version, anime_no: animeNo, ...source } });
}

// --- a season's subtitle candidates (`src/web/seasons_anissia_api.rs`) -----------------

/** How a subtitle job stands for one candidate: the job's item for it. */
export interface CandidateJob {
  /** The job whose page `/todo/job/<id>` shows it. */
  id: string;
  state: "pending" | "running" | "waiting" | "held" | "failed" | "done";
  /** What a `waiting` item waits for: a person's check or a source it cannot read yet. */
  wait: "auth" | "subtitle" | "placement" | "approval" | "video" | null;
}

/**
 * A revision candidate: the same creator's subtitle of the episode was received from an earlier observation, or the
 * user named the creator for the season's subtitle file of the episode (`of` and `same_post` are then `null`: the
 * file's post is not known).
 */
export interface CandidateRevision {
  /** The earlier observation. */
  of: number | null;
  /** The earlier observation had the same post address: the post was fixed, not posted again. */
  same_post: boolean | null;
}

/**
 * One observation of a creator's Anissia line. A creator can have several
 * observations of one episode (the post was fixed or posted again); the IDs
 * grow with the time they were observed.
 */
export interface Candidate {
  id: number;
  /** The app's ID of the creator's lines of the anime; `creator` is only the display name. */
  source_id: string;
  creator: string;
  post_url: string;
  /** Anissia's text as written (`12`, `13.5`, `0`): not a number to compute with. */
  episode: string;
  /** Anissia's `updDt` as received. */
  updated: string;
  updated_at: number | null;
  updated_parse_failed: boolean;
  first_seen_at: number;
  sort_at: number;
  revision: CandidateRevision | null;
  job: CandidateJob | null;
}

export interface CandidateList {
  season: number;
  /** `null` for a season with no Anissia link (no candidates). */
  anime_no: number | null;
  /** When the 30-minute reading last read Anissia's whole recent list. */
  read_at: number | null;
  /** The latest `새로고침` of the anime. */
  refresh: Command | null;
  /** Newest first. */
  candidates: Candidate[];
  /** The episodes each creator's candidates are about, as runs. */
  creator_episodes: { source_id: string; episode_segments: EpisodeSegment[] }[];
  /** The sources' episode mappings to the season (the app's decision, or the user's). */
  mappings: CandidateMapping[];
  /** The episodes of the earlier seasons together when each is known (`0` for the first season); `null` otherwise. */
  previous_episodes?: number | null;
  /** The season's own episode count when it is known. */
  season_episodes?: number | null;
}

/**
 * How a source's episodes map to the season's: `auto` (the app decided it from grounds that agree;
 * `offset` is added to Anissia's whole episode), `undecided` (the grounds are missing or disagree;
 * nothing is received automatically) or `user`.
 */
export interface CandidateMapping {
  source_id: string;
  kind: "auto" | "undecided" | "user";
  offset: number | null;
  /** The grounds that agree, or why none do, as a sentence. */
  evidence: string;
  decided_at: number;
  /** What a save or a revert carries; a source with no mapping is version 0. */
  version: number;
  /** The user's exceptions: one Anissia episode text to one season episode, or `null` for 받지 않음. */
  exceptions: MappingException[];
}

export interface MappingException {
  episode: string;
  target: number | null;
}

/** A source's mapping as the revert and a `409` carry it (`null` when the source has none). */
export interface SourceMapping {
  source_id: string;
  mapping: CandidateMapping | null;
}

const mappingPath = (id: string, season: number, sourceId: string) =>
  `${anissiaPath(id, season)}/sources/${encodeURIComponent(sourceId)}/mapping`;

/** Saves the user's mapping of a source: the default offset and the exceptions as one unit, from `version`. */
export function saveMapping(
  id: string,
  season: number,
  sourceId: string,
  body: { version: number; offset: number; exceptions: MappingException[] },
): Promise<CandidateMapping> {
  return api<CandidateMapping>(mappingPath(id, season, sourceId), { method: "PUT", body });
}

/** `자동으로 되돌리기`: drops the user's mapping so the app decides again. */
export function revertMapping(id: string, season: number, sourceId: string, version: number): Promise<SourceMapping> {
  return api<SourceMapping>(`${mappingPath(id, season, sourceId)}/revert`, { method: "POST", body: { version } });
}

/** The cache key of one season's candidates (the anime is in it, so a new link never shows the old one's list). */
export const candidatesKey = (id: string, season: number, animeNo: number | null) =>
  `library:candidates:${id}:${season}:${animeNo ?? "none"}`;

export function loadCandidates(id: string, season: number, signal?: AbortSignal): Promise<CandidateList> {
  return api<CandidateList>(`${anissiaPath(id, season)}/candidates`, { signal });
}

/** The command `새로고침` sends: reads the anime's subtitle lines now. */
export const ANISSIA_CAPTIONS_KIND = "anissia_captions";

// --- the creator of a season's subtitle files (`src/web/subtitle_creator_api.rs`) -----

/** The answer of naming a season's unknown subtitles a creator. */
export interface NamedCreators {
  season: number;
  creator: FileCreator;
  /** How many files it named: the ones that were still unknown. */
  attached: number;
}

/** Names the creator `creator` (one of the season's Anissia anime) for every subtitle file of the season that has none. */
export function nameUnknownCreators(id: string, season: number, creator: string): Promise<NamedCreators> {
  return api<NamedCreators>(`${seasonPath(id, season)}/subtitle-creators`, { method: "POST", body: { creator } });
}

/** One subtitle file's creator now. */
export interface FileCreatorState {
  path: string;
  version: number;
  creator: FileCreator | null;
}

/** Changes one subtitle file's creator from `version` (`null` creator: back to `제작자 알 수 없음`). */
export function setFileCreator(
  id: string,
  season: number,
  file: { path: string; version: number },
  creator: string | null,
): Promise<FileCreatorState> {
  return api<FileCreatorState>(`${seasonPath(id, season)}/subtitle-creators/file`, {
    method: "PUT",
    body: { path: file.path, version: file.version, creator },
  });
}
