import type { Change, ChangeKind } from "../../lib/api-types";
import { ofKinds } from "../../lib/changeMatch.ts";

/**
 * Which changes make the collection tabs read again
 * (`docs/requirements/web-app.md`, 웹 명령과 상태 갱신). Each list of kinds is the
 * tables the tab's route reads, grouped as the server announces them
 * (`crates/trss-core/src/changes/tables.rs`, `TABLES`); a kind that is only
 * likely to matter is in the list too, because a read too many costs little
 * and one too few leaves a tab on an old state. The tabs read whole lists, so
 * a change matches by its kind whatever it names. The title candidates and the
 * archive suggestions read again as the 할 일 screen's do
 * (`todo/listens.ts`, `suggestionsChanged`). It has no `@/` imports so it runs
 * under `node --test`.
 */

/**
 * The 구독 tab (`GET /api/subscriptions`): the subscribed rules with their
 * channels and own covers (`rules`) and the Anissia anime they follow
 * (`anissia`). Whether an anime runs, ended or is still coming, and the
 * quarter it counts in, follow the Seoul day, which the server announces at
 * midnight as `collect_status` (`crates/trss/src/deadlines.rs`).
 */
const SUBSCRIPTIONS: readonly ChangeKind[] = ["rules", "anissia", "collect_status"];

export function subscriptionsChanged(change: Change): boolean {
  return ofKinds(change, SUBSCRIPTIONS);
}

/**
 * The 규칙 tab's list and the open rule (`GET /api/rules`): the rules and
 * channels (`rules`), their Anissia anime (`anissia`), what they received and
 * the items their episode is told from (`history`), the collect folder
 * (`settings`), the folder moves and episode undos under way (`command`), and
 * for a rule linked to a season the work, its videos, its season's entries and
 * its cover (`library`, `season_info`, `artwork`).
 */
const RULES: readonly ChangeKind[] = [
  "rules",
  "anissia",
  "history",
  "settings",
  "command",
  "library",
  "season_info",
  "artwork",
];

export function rulesChanged(change: Change): boolean {
  return ofKinds(change, RULES);
}

/**
 * The 기록 tab (`GET /api/history`): the items and their counts (`history`),
 * the channels and rules that name them (`rules`, and `anissia` for a
 * subscription's title), the `다시 받기` commands under way (`command`) and,
 * for an item whose version is unknown, the collect folder and how old the
 * worker's list of Transmission's torrents is (`settings`, `collect_status`).
 */
const HISTORY: readonly ChangeKind[] = ["history", "rules", "anissia", "command", "settings", "collect_status"];

export function historyChanged(change: Change): boolean {
  return ofKinds(change, HISTORY);
}

/** The 채널 tab and the 기록 tab's channel filter (`GET /api/channels`): the channels and their rule counts. */
export function channelListChanged(change: Change): boolean {
  return change.kind === "rules";
}

/** How long to wait before reading the 구독 tab again after a read that failed, in milliseconds. */
export const SUBSCRIPTIONS_RETRY_MS = 30_000;

/** ...and the 규칙 tab. */
export const RULES_RETRY_MS = 30_000;

/** ...and the 기록 tab. */
export const HISTORY_RETRY_MS = 30_000;

/** ...and the channels. */
export const CHANNELS_RETRY_MS = 30_000;
