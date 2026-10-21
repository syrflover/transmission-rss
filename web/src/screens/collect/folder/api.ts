import { api } from "@/lib/api";
import type {
  FolderSuggestion,
  FolderSuggestionAnswer,
  FolderWork,
  FolderWorksAnswer,
} from "@/lib/api-types";

import { suggestionPath, type AnimeOfFolder, type Asked } from "./saveFolder";

// --- `/api/save-folder` (`src/web/save_folder_api.rs`) --------------------------------

/** The works of the collect folder, then of the archive folder: the library's records. */
export function fetchFolderWorks(signal?: AbortSignal): Promise<FolderWork[]> {
  return api<FolderWorksAnswer>("/save-folder/works", { signal }).then((r) => r.works);
}

/** The folder suggested for a release's work or a picked work; `null` when no folder name is left of it. */
export function fetchSuggestion(asked: Asked, anime: AnimeOfFolder | null, signal?: AbortSignal): Promise<FolderSuggestion | null> {
  return api<FolderSuggestionAnswer>(suggestionPath(asked, anime), { signal }).then((r) => r.suggestion);
}
