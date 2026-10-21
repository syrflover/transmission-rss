import { api } from "@/lib/api";
import type { AddWatchFolderBody, WatchFolder, WatchFolderAdded, WatchFolderList, WatchFolderRemoved } from "@/lib/api-types";

/*
 * The watch folders (`src/web/watch_folders_api.rs`): the folders the app reads for works. The shapes are generated
 * (`lib/api-types`).
 */
export type { WatchFolder, WatchFolderList };

export function loadWatchFolders(signal?: AbortSignal): Promise<WatchFolderList> {
  return api<WatchFolderList>("/library/watch-folders", { signal });
}

/** Registers the folder and reads it once; a refusal is an `invalid` error with the reason as its message. */
export function addWatchFolder(path: string): Promise<WatchFolderAdded> {
  const body: AddWatchFolderBody = { path };
  return api<WatchFolderAdded>("/library/watch-folders", { method: "POST", body });
}

/** Removes the folder and its works from the library; no file on the disk is touched. */
export function removeWatchFolder(id: string): Promise<WatchFolderRemoved> {
  return api<WatchFolderRemoved>(`/library/watch-folders/${id}`, { method: "DELETE" });
}

/** The command kind of `다시 확인` (`src/web/commands_api.rs`). */
export const RESCAN_KIND = "watch_rescan";

/** What the settings list shows for the item: `폴더 2개`, or `null` while none is registered. */
export function foldersSummary(list: WatchFolderList): string | null {
  return list.folders.length === 0 ? null : `폴더 ${list.folders.length}개`;
}
