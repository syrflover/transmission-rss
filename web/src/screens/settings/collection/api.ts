import { api } from "@/lib/api";

/**
 * The collect folder and the archive folder (`src/web/settings_api.rs`).
 * `folder` is `null` and `version` 0 until the collect folder is chosen; the
 * version seen goes back with every save, and a stale one answers `conflict`.
 */
export interface Collection {
  folder: string | null;
  archive_folder: string | null;
  version: number;
}

export function loadCollection(signal?: AbortSignal): Promise<Collection> {
  return api<Collection>("/settings/collection", { signal });
}

/** `archive` may be blank: the archive folder is optional. */
export function saveCollection(version: number, folder: string, archive: string): Promise<Collection> {
  return api<Collection>("/settings/collection", {
    method: "PUT",
    body: { version, folder, archive_folder: archive.trim() === "" ? null : archive },
  });
}

/** The last part of a path, which is what a folder is called in a list. */
export function folderName(path: string): string {
  const parts = path.split("/").filter((part) => part !== "");
  return parts.length > 0 ? parts[parts.length - 1] : path;
}

/**
 * What the settings list shows for the item: `Shows (current) · 보관 Shows`,
 * only the collect folder's name when there is no archive folder, and `null`
 * while no collect folder is chosen.
 */
export function collectionSummary(collection: Collection): string | null {
  if (collection.folder === null) return null;
  const name = folderName(collection.folder);
  return collection.archive_folder === null ? name : `${name} · 보관 ${folderName(collection.archive_folder)}`;
}
