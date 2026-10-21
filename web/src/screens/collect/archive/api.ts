import { api, ApiError, timedRead } from "@/lib/api";
import { newCommandId, sendCommand } from "@/lib/commands";
import type {
  ArchiveBatchRule,
  ArchiveKeepBody,
  ArchiveKept,
  ArchiveSuggestion,
  ArchiveSuggestionList,
} from "@/lib/api-types";

import { sendUntilAnswered, type Sent } from "./batchFollow";
import type { ArchiveBatchView } from "./batchRun";

/**
 * The archive suggestions (`src/web/archive_api.rs`): rules the app suggests
 * archiving because their anime ended or nothing new has come for 4 weeks.
 * Archiving them is one `rule_archive_batch` command that the server carries
 * out rule after rule ({@link startBatch}); one rule's own `보관` is its
 * `rule_archive` command (`rules/api.ts`).
 */

export function fetchArchiveSuggestions(signal?: AbortSignal): Promise<ArchiveSuggestion[]> {
  return api<ArchiveSuggestionList>("/archive-suggestions", { signal }).then((r) => r.suggestions);
}

/** `수집 유지`: the rule is not suggested again on the grounds the user saw. */
export async function keepCollecting(suggestion: ArchiveSuggestion): Promise<void> {
  const body: ArchiveKeepBody = { rule_id: suggestion.rule_id, grounds: suggestion.grounds.map((g) => g.key) };
  await api<ArchiveKept>("/archive-suggestions/keep", { method: "POST", body });
}

const wait = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));

/** Whether a failed request may have reached the server, so that sending it again with the same ID is safe. */
function mayBeLost(e: unknown): boolean {
  // A conflict means another batch is open, and the view does not say whose it is, so it is not taken for this one.
  return !(e instanceof ApiError) || e.code === "network" || e.code === "internal" || e.code === "unavailable";
}

/**
 * Sends the rules as one `rule_archive_batch` command, in the order given,
 * through the web command contract (`src/store/commands`): one command ID for
 * the action, resent unchanged when the answer was lost. Resolves with the
 * batch's ID, which its events and its view are named by
 * (`src/web/archive_batch_api.rs`), once the server has stored it, or with the
 * sentence for why it did not.
 */
export function startBatch(rules: ArchiveBatchRule[]): Promise<Sent<string>> {
  const id = newCommandId();
  return sendUntilAnswered(
    async () => {
      await sendCommand(id, "rule_archive_batch", { rules });
      return id;
    },
    mayBeLost,
    (e) => (e instanceof Error ? e.message : String(e)),
    wait,
  );
}

/** Where the batch `id` is. */
export function getBatch(id: string): Promise<ArchiveBatchView> {
  return api<ArchiveBatchView>(`/archive-batches/${id}`, timedRead());
}

/** The batch the server is running now, for a screen that opens during one; `null` when none is. */
export async function getCurrentBatch(): Promise<ArchiveBatchView | null> {
  try {
    return await api<ArchiveBatchView>("/archive-batches/current", timedRead());
  } catch (e) {
    if (e instanceof ApiError && e.code === "not_found") return null;
    throw e;
  }
}
