import { api } from "@/lib/api";
import { sendCommand, type Command } from "@/lib/commands";

import type { WaitingSub } from "../subs/api";

// The command API is shared with other screens; these were first defined here.
export { getCommand, newCommandId, type Command, type CommandOutcome, type CommandState } from "@/lib/commands";

/**
 * The history API (`src/web/history_api.rs`) and the `다시 받기` command
 * (`src/web/commands_api.rs`). A history item never carries its link: the copy
 * kept there is masked and nothing on the screen needs it.
 */

export type HistoryResult = "received" | "no_match" | "excluded" | "duplicate" | "add_failed" | "version_unknown";

export const HISTORY_RESULTS: readonly HistoryResult[] = [
  "received",
  "no_match",
  "excluded",
  "duplicate",
  "add_failed",
  "version_unknown",
];


export interface HistoryItem {
  id: number;
  channel_id: string;
  channel_name: string;
  channel_deleted: boolean;
  title: string;
  /** Unix milliseconds. */
  first_seen_at: number;
  result: HistoryResult;
  result_label: string;
  result_at: number;
  rule_label: string | null;
  by_hand: boolean;
  reason: string | null;
  /**
   * Whether `다시 받기` is offered: the item failed to be added, or is a video revision held as `version_unknown`,
   * and the rule that picked it still exists and is active.
   */
  can_retry: boolean;
  /** Why `다시 받기` is missing on an item a rule picked and failed to add, as a sentence. */
  retry_blocked: string | null;
  /** The retry command (`receive_once`) that has not ended yet. */
  command: Command | null;
  /** Set on a `no_match` item whose channel has a subscription waiting for a title. */
  name_title: NameTitle | null;
}

/** What a `no_match` item offers when the channel has subscriptions waiting for a title. */
export interface NameTitle {
  /** The work part of the item's title: the match phrase it would give. */
  work: string;
  /** The save folder made from the work. */
  folder: string | null;
  waiting: WaitingSub[];
}

export interface HistoryCounts {
  total: number;
  received: number;
  no_match: number;
  excluded: number;
  duplicate: number;
  add_failed: number;
  version_unknown: number;
}

export interface HistoryPage {
  items: HistoryItem[];
  /** Pass as `after` for the next page; `null` on the last one. */
  next: string | null;
  counts: HistoryCounts;
}

export interface HistoryFilter {
  results: HistoryResult[];
  /** A channel ID, or `null` for every channel. */
  channel: string | null;
}

export function listHistory(
  filter: HistoryFilter,
  options: { after?: string | null; limit?: number } = {},
): Promise<HistoryPage> {
  const params = new URLSearchParams();
  if (filter.results.length > 0) params.set("result", filter.results.join(","));
  if (filter.channel) params.set("channel", filter.channel);
  if (options.after) params.set("after", options.after);
  if (options.limit) params.set("limit", String(options.limit));
  const query = params.toString();
  return api<HistoryPage>(`/history${query === "" ? "" : `?${query}`}`);
}

export function getHistoryItem(id: number): Promise<HistoryItem> {
  return api<HistoryItem>(`/history/${id}`);
}

/**
 * What `다시 받기` sends: the item alone. The save folder and the episode
 * conversion are the rule's, so there is no folder to send.
 */
export interface RetryPayload {
  item_id: number;
}

/** Sends the command; `202` when stored now, `200` when the same command was stored before: both give its current state. */
export function sendRetry(id: string, payload: RetryPayload): Promise<Command> {
  return sendCommand(id, "receive_once", payload);
}
