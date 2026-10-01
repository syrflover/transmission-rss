import { api, ApiError } from "@/lib/api";
import { forgetPrefix } from "@/lib/cached";

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

/**
 * Whoever changes which folders the library reads (adds, rescans or removes a
 * watch folder, sets the collect or archive folder) calls this: the loaded
 * pages of the list and every work's page are read again the next time.
 */
export function forgetLibrary(): void {
  forgetPrefix(LIST_PREFIX);
  forgetPrefix(WORK_PREFIX);
}

/** Where a work opens (the work detail screen). */
export const workPath = (id: string) => `/library/${encodeURIComponent(id)}`;

// --- one work (`src/web/library_work_api.rs`) ---------------------------------------

/** A recorded file, by its path relative to the work folder. `added_at` is `null` when unknown. */
export interface WorkFile {
  path: string;
  added_at: number | null;
}

export interface WorkEpisode {
  /** As written in the file names; `01` and `13`/`013` are one episode. */
  episode: string;
  /** The episode as a number, `null` when it is no number. */
  sort: number | null;
  video: WorkFile[];
  subtitle: WorkFile[];
}

export interface WorkSeason {
  number: number;
  /** Ascending. */
  episodes: WorkEpisode[];
}

/** A file the scan could not attach to an episode, with why. */
export interface UnrecognizedFile {
  path: string;
  reason: string;
  /** The reason as a sentence. */
  message: string;
}

/** A rule that saves into the work's folder. */
export interface WorkRule {
  id: string;
  channel: { id: string; name: string | null; host: string };
  match: string | null;
  /** Relative to the collect folder. */
  directory: string;
  save_path: string;
  state: "active" | "archived";
}

export interface WorkDetail {
  id: string;
  name: string;
  missing: boolean;
  watch_folder: { id: string; path: string };
  folder_path: string;
  added_at: number | null;
  /** Ascending by season number. */
  seasons: WorkSeason[];
  unrecognized: UnrecognizedFile[];
  rules: WorkRule[];
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
