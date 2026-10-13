import { api } from "@/lib/api";

import { suggestionPath, type AnimeOfFolder, type Asked } from "./saveFolder";

// --- `/api/save-folder` (`src/web/save_folder_api.rs`) --------------------------------

/** A work of the collect or archive folder, as the work field offers it. */
export interface FolderWork {
  /** The work's folder name: what the work field takes. */
  name: string;
  /** The work title (작품명). */
  title: string;
  /** The work is in the archive folder: a rule saving into it brings it over. */
  archived: boolean;
}

/** A suggested save folder. */
export interface FolderSuggestion {
  /** The library work's folder name when it is one. */
  work: string;
  season: number;
  /** `<work>/Season NN`. */
  directory: string;
  library: FolderWork | null;
  /** Why the season is what it is. */
  reason: string;
}

/** The works of the collect folder, then of the archive folder: the library's records. */
export function fetchFolderWorks(signal?: AbortSignal): Promise<FolderWork[]> {
  return api<{ works: FolderWork[] }>("/save-folder/works", { signal }).then((r) => r.works);
}

/** The folder suggested for a release's work or a picked work; `null` when no folder name is left of it. */
export function fetchSuggestion(asked: Asked, anime: AnimeOfFolder | null, signal?: AbortSignal): Promise<FolderSuggestion | null> {
  return api<{ suggestion: FolderSuggestion | null }>(suggestionPath(asked, anime), { signal }).then((r) => r.suggestion);
}
