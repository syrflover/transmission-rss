import { api, timedRead } from "@/lib/api";
import { sendCommand, type Command } from "@/lib/commands";
import type { HistoryItem, HistoryPage, HistoryResult, ReceiveOnce } from "@/lib/api-types";

// The command API is shared with other screens; these were first defined here.
export { getCommand, newCommandId, type Command, type CommandOutcome, type CommandState } from "@/lib/commands";

/**
 * The history API (`src/web/history_api.rs`) and the `다시 받기` command
 * (`src/web/commands_api.rs`). A history item never carries its link: the copy
 * kept there is masked and nothing on the screen needs it.
 */

export const HISTORY_RESULTS = [
  "received",
  "no_match",
  "excluded",
  "duplicate",
  "add_failed",
  "version_unknown",
] as const satisfies readonly HistoryResult[];

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
  return api<HistoryItem>(`/history/${id}`, timedRead());
}

/** Sends the command; `202` when stored now, `200` when the same command was stored before: both give its current state. */
export function sendRetry(id: string, payload: ReceiveOnce): Promise<Command> {
  return sendCommand(id, "receive_once", payload);
}
