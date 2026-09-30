import { api } from "@/lib/api";

/**
 * The history and commands APIs (`src/web/history_api.rs`,
 * `src/web/commands_api.rs`). A history item never carries its link: the copy
 * kept there is masked and nothing on the screen needs it.
 */

export type HistoryResult = "received" | "no_match" | "excluded" | "duplicate" | "add_failed";

export const HISTORY_RESULTS: readonly HistoryResult[] = [
  "received",
  "no_match",
  "excluded",
  "duplicate",
  "add_failed",
];

export type CommandState = "pending" | "running" | "done" | "failed";

export interface CommandOutcome {
  /** For `receive_once`, a history result code. */
  result: string;
  reason: string | null;
}

export interface Command {
  id: string;
  kind: string;
  state: CommandState;
  created_at: number;
  updated_at: number;
  finished_at: number | null;
  outcome: CommandOutcome | null;
}

export interface HistoryItem {
  id: number;
  channel_id: string;
  channel_name: string;
  channel_deleted: boolean;
  /** What a save folder is relative to; `null` once the channel is deleted. */
  base_dir: string | null;
  title: string;
  /** Unix milliseconds. */
  first_seen_at: number;
  result: HistoryResult;
  result_label: string;
  result_at: number;
  rule_label: string | null;
  by_hand: boolean;
  reason: string | null;
  can_receive_once: boolean;
  /** The `receive_once` command that has not ended yet. */
  command: Command | null;
}

export interface HistoryCounts {
  total: number;
  received: number;
  no_match: number;
  excluded: number;
  duplicate: number;
  add_failed: number;
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

/** What `한 번 받기` sends. `folder` is relative to the channel's base folder; empty means the base folder. */
export interface ReceiveOncePayload {
  item_id: number;
  folder: string;
}

/**
 * A command ID for one user action. The browser makes it, sends it together
 * with the request's content, and asks for the command by it after a lost
 * answer. `randomUUID` needs a secure context, so the ID is built from
 * `getRandomValues`, which the app also has over plain HTTP.
 */
export function newCommandId(): string {
  const bytes = new Uint8Array(16);
  crypto.getRandomValues(bytes);
  return Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
}

/** Sends the command; `202` when stored now, `200` when the same command was stored before: both give its current state. */
export function sendReceiveOnce(id: string, payload: ReceiveOncePayload): Promise<Command> {
  return api<Command>("/commands", {
    method: "POST",
    body: { id, kind: "receive_once", payload },
  });
}

export function getCommand(id: string): Promise<Command> {
  return api<Command>(`/commands/${id}`);
}
