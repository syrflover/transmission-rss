import { api } from "@/lib/api";

import type { StorageOverview } from "../../library/storage.ts";

/** The works with a stored file or a cover, each with its size per kind, the biggest first. */
export function loadStorage(signal?: AbortSignal): Promise<StorageOverview> {
  return api<StorageOverview>("/library/storage", { signal });
}
