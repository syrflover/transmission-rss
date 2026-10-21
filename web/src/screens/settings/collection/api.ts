import { api } from "@/lib/api";
import type { CollectionBody, CollectionSettings } from "@/lib/api-types";

/*
 * The collect folder and the archive folder (`src/web/settings_api.rs`): `CollectionSettings` is generated
 * (`lib/api-types`). `folder` is `null` and `version` 0 until the collect folder is chosen; the version seen goes back
 * with every save, and a stale one answers `conflict`.
 */
export function loadCollection(signal?: AbortSignal): Promise<CollectionSettings> {
  return api<CollectionSettings>("/settings/collection", { signal });
}

/** `archive` may be blank: the archive folder is optional. */
export function saveCollection(version: number, folder: string, archive: string): Promise<CollectionSettings> {
  const body: CollectionBody = { version, folder, archive_folder: archive.trim() === "" ? null : archive };
  return api<CollectionSettings>("/settings/collection", { method: "PUT", body });
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
export function collectionSummary(collection: CollectionSettings): string | null {
  if (collection.folder === null) return null;
  const name = folderName(collection.folder);
  return collection.archive_folder === null ? name : `${name} · 보관 ${folderName(collection.archive_folder)}`;
}
