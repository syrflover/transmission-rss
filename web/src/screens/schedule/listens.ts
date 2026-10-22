import type { Change, ChangeKind } from "../../lib/api-types";
import { ofKinds } from "../../lib/changeMatch.ts";

/**
 * Which changes make the 이번 주 편성 read again (`docs/requirements/web-app.md`, 이번
 * 주 편성 and 웹 명령과 상태 갱신). The kinds are the tables its route
 * (`GET /api/schedule/week`) reads, grouped as the server announces them
 * (`crates/trss-core/src/changes/tables.rs`, `TABLES`); a kind that is only
 * likely to matter is in the list too, because a read too many costs little and
 * one too few leaves the schedule on an old state. It has no `@/` imports so it
 * runs under `node --test`.
 *
 * - `collect_status`: the collection cycle and Transmission's snapshot, which
 *   decide `영상 받는 중`, and, from the server, the moments a state that only
 *   the clock changes comes (the stall of the cycle, the air time of an
 *   episode, a new Seoul day: `crates/trss/src/deadlines.rs`).
 * - `library`: the episodes' videos and subtitles beside them (`영상 받음`,
 *   `자막 받음`).
 * - `rules`, `anissia`, `season_info`: the subscriptions, the anime they follow
 *   and the AniList entries that tell when an episode airs. A subscription's
 *   own cover, which a card shows until the subscription has a season, is
 *   announced as `rules` too.
 * - `artwork`: the works' covers on the cards.
 * - `history`: the torrents a rule received (`영상 받는 중` names the episodes).
 * - `settings`: the first run's checklist, which takes the schedule's place.
 */
const SCHEDULE: readonly ChangeKind[] = [
  "collect_status",
  "library",
  "rules",
  "anissia",
  "season_info",
  "artwork",
  "history",
  "settings",
];

export function scheduleChanged(change: Change): boolean {
  return ofKinds(change, SCHEDULE);
}

/** How long to wait before reading the schedule again after a read that failed, in milliseconds. */
export const SCHEDULE_RETRY_MS = 30_000;
