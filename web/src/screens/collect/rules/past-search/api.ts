import { api } from "@/lib/api";
import { sendCommand, type Command } from "@/lib/commands";

/**
 * The past episode search API (`src/web/past_search_api.rs`). A search runs on
 * the server for up to about a minute, so the screen starts it and asks how it
 * goes. Nothing here judges a result: the server runs the rule's own judgement.
 */

/** The range the app offers, and why. */
export interface RangeSuggestion {
  from: number | null;
  to: number | null;
  /** A sentence for the screen. */
  basis: string;
}

export interface SearchContext {
  rule_id: string;
  channel_id: string;
  /** The channel's search format as saved, `null` when it has none. */
  format: string | null;
  /** The search words to start with. */
  query: string;
  /** Whether the channel's format made them. */
  from_format: boolean;
  /** Why the box starts empty, when it does. */
  empty_because: string | null;
  suggestion: RangeSuggestion;
  /** The rule's episode conversion, which the folder's episodes follow. */
  offset: number;
  /** The work folder's season number. */
  season: number | null;
  /** A search of this rule that is running now. */
  running: string | null;
  /** Why no search can start now, as a sentence. */
  blocked: string | null;
}

/** What a result is, for the preview. */
export type ResultState =
  | "missing"
  | "replace"
  | "version_unknown"
  | "have"
  | "superseded"
  | "alternate"
  | "batch"
  | "unnumbered";

export interface ResultItem {
  key: string;
  title: string;
  state: ResultState;
  note: string | null;
  /** The release number; `null` for a batch or a title without one. */
  release: number | null;
  half: boolean;
  version: number;
  /** The work folder's name for the episode (`S02E01`). */
  folder: string | null;
  /** Whether the preview starts with it selected. */
  selected: boolean;
  /** Whether it may be selected at all. */
  selectable: boolean;
}

export interface SearchResult {
  from: number;
  to: number;
  query: string;
  items: ResultItem[];
  /** Individual episodes and batches outside the range. */
  out_of_range: number;
  /** Results the rule does not pick. */
  not_picked: number;
  /** The release numbers the work does not have. */
  missing: number[];
  /** `missing` as runs (`4–6`, `9`). */
  missing_ranges: string[];
  /** The missing ones no result is an episode of. */
  not_found: number[];
  /** `not_found` as runs. */
  not_found_ranges: string[];
  notes: string[];
  first_full: boolean;
  extra_sent: number;
  extra_needed: number;
}

export interface SearchPoll {
  state: "running" | "failed" | "done";
  rule_id: string;
  sent: number;
  needed: number;
  error: string | null;
  result: SearchResult | null;
}

export function searchContext(ruleId: string): Promise<SearchContext> {
  return api<SearchContext>(`/rules/${encodeURIComponent(ruleId)}/past-search`);
}

export function startSearch(
  ruleId: string,
  body: { query: string; from: number; to: number },
): Promise<{ search_id: string }> {
  return api<{ search_id: string }>(`/rules/${encodeURIComponent(ruleId)}/past-search`, {
    method: "POST",
    body,
  });
}

export function pollSearch(searchId: string): Promise<SearchPoll> {
  return api<SearchPoll>(`/past-searches/${encodeURIComponent(searchId)}`);
}

export function cancelSearch(searchId: string): Promise<void> {
  return api<void>(`/past-searches/${encodeURIComponent(searchId)}`, { method: "DELETE" });
}

/** Asks the worker to add one result of a finished search (`receive_past`). */
export function receivePast(id: string, ruleId: string, searchId: string, key: string): Promise<Command> {
  return sendCommand(id, "receive_past", { rule_id: ruleId, search_id: searchId, key });
}

/**
 * The work folder's episode a release number lands on, as the worker names the
 * video: a negative conversion adds only while the result stays at 1 or above,
 * a positive `p` makes release `1` the folder's `p`, `0` changes nothing.
 */
export function folderEpisode(release: number, offset: number): number {
  if (offset < 0) return release + offset >= 1 ? release + offset : release;
  if (offset > 0) return release + offset - 1;
  return release;
}

/** `S02E01–12` for the folder episodes of releases `from` to `to`, `1–12화` without a season. */
export function rangeLabel(from: number, to: number, offset: number, season: number | null): string {
  const first = folderEpisode(from, offset);
  const last = folderEpisode(to, offset);
  if (season === null) return `${first}–${last}화`;
  const s = `S${String(season).padStart(2, "0")}`;
  const e = (n: number) => String(n).padStart(2, "0");
  return first === last ? `${s}E${e(first)}` : `${s}E${e(first)}–${e(last)}`;
}
