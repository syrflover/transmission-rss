import { api } from "@/lib/api";
import { sendCommand, type Command } from "@/lib/commands";
import type {
  RangeLabel,
  SearchContext,
  SearchPoll,
  SearchRangeBody,
  SearchStarted,
  StartSearchBody,
} from "@/lib/api-types";

/**
 * The past episode search API (`src/web/past_search_api.rs`). A search runs on
 * the server for up to about a minute, so the screen starts it and asks how it
 * goes. Nothing here judges a result: the server runs the rule's own judgement.
 */

export function searchContext(ruleId: string): Promise<SearchContext> {
  return api<SearchContext>(`/rules/${encodeURIComponent(ruleId)}/past-search`);
}

/** The folder episodes of the releases a person typed (`POST …/past-search/range`), written by the server. */
export function rangeLabelOf(ruleId: string, from: number, to: number, signal?: AbortSignal): Promise<RangeLabel> {
  const body: SearchRangeBody = { from, to };
  return api<RangeLabel>(`/rules/${encodeURIComponent(ruleId)}/past-search/range`, { method: "POST", body, signal });
}

export function startSearch(ruleId: string, body: StartSearchBody): Promise<SearchStarted> {
  return api<SearchStarted>(`/rules/${encodeURIComponent(ruleId)}/past-search`, {
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
