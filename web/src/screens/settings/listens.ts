import type { Change, ChangeKind } from "../../lib/api-types";
import { ofKinds } from "../../lib/changeMatch.ts";

/**
 * Which changes make the settings list and its panels read again
 * (`docs/requirements/web-app.md`, 웹 명령과 상태 갱신), by the tables their routes
 * read, grouped as the server announces them
 * (`crates/trss-core/src/changes/tables.rs`, `TABLES`). The list and a panel
 * read the same route under the same cache key, so they listen alike; each
 * export is what `useRead` takes as its `listen`. A change matches by its kind
 * whatever it names, because the routes read whole lists. It has no `@/`
 * imports so it runs under `node --test`.
 */

/** How long to wait before reading a setting again after a read that failed, in milliseconds. */
export const SETTINGS_RETRY_MS = 30_000;

const listening = (kinds: readonly ChangeKind[]) => ({
  match: (change: Change) => ofKinds(change, kinds),
  retryMs: SETTINGS_RETRY_MS,
});

/** `수집 폴더` (`GET /api/settings/collection`): the collect and archive folders. */
export const COLLECTION = listening(["settings"]);

/**
 * `감시 폴더` (`GET /api/library/watch-folders`): the folders and the counts of
 * their works (`library`). How many works are new counts the last 7 days, and
 * nothing announces a work leaving that window.
 */
export const WATCH_FOLDERS = listening(["library"]);

/**
 * `자막 형식·브라우저` (`GET /api/settings/policy`): the common policy
 * (`settings`) and the works with an order of their own, named by their work
 * title (`library`, `season_info`, `anissia`).
 */
export const POLICY = listening(["settings", "library", "season_info", "anissia"]);

/**
 * `파일 용량·정리` (`GET /api/library/storage`): the stored subtitle files and
 * what can be cleaned (`subtitle_store`, and `subtitle_job` for the files a
 * job still holds), the covers (`artwork`) and the works by their title
 * (`library`, `season_info`, `anissia`).
 */
export const STORAGE = listening(["subtitle_store", "subtitle_job", "artwork", "library", "season_info", "anissia"]);
