import { api } from "@/lib/api";

/**
 * The watch folders (`src/web/watch_folders_api.rs`): the folders the app reads
 * for works. Each carries the counts its row shows; `checked_at` is the last
 * attempt to read it and `error` a sentence while that attempt could not read
 * everything (what was found before is kept).
 */
export interface WatchFolder {
  id: string;
  path: string;
  /** The collect or archive folder: the app keeps it registered while the settings use it, so it cannot be unregistered. */
  automatic: boolean;
  /** Every work of the folder, including the ones whose folder is gone. */
  works: number;
  /** Of those, the works whose folder is gone (`폴더 없음`). */
  missing_works: number;
  /** Works linked to Anissia; none can be before linking exists. */
  linked_works: number;
  /** Works found after the folder's first check, within the last 7 days. */
  new_works: number;
  checked_at: number | null;
  error: string | null;
}

export interface WatchFolderList {
  folders: WatchFolder[];
}

export interface AddedFolder {
  folder: WatchFolder;
  works_found: number;
}

export interface RemovedFolder {
  removed_works: number;
}

export function loadWatchFolders(signal?: AbortSignal): Promise<WatchFolderList> {
  return api<WatchFolderList>("/library/watch-folders", { signal });
}

/** Registers the folder and reads it once; a refusal is an `invalid` error with the reason as its message. */
export function addWatchFolder(path: string): Promise<AddedFolder> {
  return api<AddedFolder>("/library/watch-folders", { method: "POST", body: { path } });
}

/** Removes the folder and its works from the library; no file on the disk is touched. */
export function removeWatchFolder(id: string): Promise<RemovedFolder> {
  return api<RemovedFolder>(`/library/watch-folders/${id}`, { method: "DELETE" });
}

/** The command kind of `다시 확인` (`src/web/commands_api.rs`). */
export const RESCAN_KIND = "watch_rescan";

/** What the settings list shows for the item: `폴더 2개`, or `null` while none is registered. */
export function foldersSummary(list: WatchFolderList): string | null {
  return list.folders.length === 0 ? null : `폴더 ${list.folders.length}개`;
}
