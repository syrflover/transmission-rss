import type { Change, ChangeKind } from "../../../lib/api-types";
import { ofKinds } from "../../../lib/changeMatch.ts";

/**
 * Which changes make the 수집 상태판 read again (`docs/requirements/web-app.md`, 수집
 * 상태판 and 웹 명령과 상태 갱신). The kinds are the tables its route
 * (`GET /api/collect/status`) reads, grouped as the server announces them
 * (`crates/trss-core/src/changes/tables.rs`, `TABLES`). It has no `@/` imports
 * so it runs under `node --test`.
 *
 * - `collect_status`: the collection cycle, the channels' reads and
 *   Transmission's snapshot, and, from the server, the moment the cycle counts
 *   as stalled (`crates/trss/src/deadlines.rs`).
 * - `rules`: the channels' names and addresses.
 * - `history`: the items received in the last seven days and the failures.
 * - `settings`: whether the collect folder is set.
 */
const BOARD: readonly ChangeKind[] = ["collect_status", "rules", "history", "settings"];

export function boardChanged(change: Change): boolean {
  return ofKinds(change, BOARD);
}

/** How long to wait before reading the board again after a read that failed, in milliseconds. */
export const BOARD_RETRY_MS = 30_000;
