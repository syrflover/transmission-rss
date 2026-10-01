import { api, ApiError } from "@/lib/api";

/**
 * The library list (`src/web/library_api.rs`): every work with the summary the
 * list shows. Times are Unix milliseconds, `null` when unknown.
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

export interface LibraryWorkList {
  works: LibraryWork[];
}

/** The cache key of the list; whoever changes the watch folders drops it. */
export const WORKS_KEY = "library:works";

export function loadWorks(signal?: AbortSignal): Promise<LibraryWorkList> {
  return api<LibraryWorkList>("/library/works", { signal });
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
export const workKey = (id: string) => `library:work:${id}`;

/** One work, or `null` when the library has no work with this ID. */
export async function loadWork(id: string, signal?: AbortSignal): Promise<WorkDetail | null> {
  try {
    return await api<WorkDetail>(`/library/works/${encodeURIComponent(id)}`, { signal });
  } catch (e) {
    if (e instanceof ApiError && e.code === "not_found") return null;
    throw e;
  }
}
