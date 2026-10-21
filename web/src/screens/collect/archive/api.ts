import { api, ApiError } from "@/lib/api";
import { getCommand, isOpen, newCommandId, type Command } from "@/lib/commands";
import type { ArchiveKeepBody, ArchiveKept, ArchiveSuggestion, ArchiveSuggestionList } from "@/lib/api-types";

import { sendArchive } from "../rules/api";

/**
 * The archive suggestions (`src/web/archive_api.rs`): rules the app suggests
 * archiving because their anime ended or nothing new has come for 4 weeks.
 * Archiving itself is the rule's `rule_archive` command (`rules/api.ts`).
 */

export function fetchArchiveSuggestions(signal?: AbortSignal): Promise<ArchiveSuggestion[]> {
  return api<ArchiveSuggestionList>("/archive-suggestions", { signal }).then((r) => r.suggestions);
}

/** `수집 유지`: the rule is not suggested again on the grounds the user saw. */
export async function keepCollecting(suggestion: ArchiveSuggestion): Promise<void> {
  const body: ArchiveKeepBody = { rule_id: suggestion.rule_id, grounds: suggestion.grounds.map((g) => g.key) };
  await api<ArchiveKept>("/archive-suggestions/keep", { method: "POST", body });
}

/** How often an open archive command is asked for, in milliseconds. */
export const POLL_MS = 1000;
/** How many times a request that got no answer is sent again (with the same ID) before giving up. */
const RESENDS = 4;

const wait = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));

/** How one rule's archive ended: its command ended `done` or `failed`, or it could not be sent at all. */
export type ArchiveOutcome = { ok: true; command: Command } | { ok: false; message: string };

/**
 * Archives one rule through the web command contract (`src/store/commands`):
 * one command ID for the action, resent unchanged when the answer was lost
 * (the server stores a command once per ID), then asked for by that ID until
 * the worker has ended it. The worker runs commands in the order they were
 * accepted, so waiting for the end of each before sending the next keeps the
 * order the caller gave.
 */
export async function archiveRule(ruleId: string): Promise<ArchiveOutcome> {
  const id = newCommandId();
  let command: Command | null = null;
  for (let attempt = 0; command === null; attempt += 1) {
    try {
      command = await sendArchive(id, ruleId, "archive");
    } catch (e) {
      // A conflict means another command of this rule is open: a restore from
      // another tab as likely as an archive, and the view does not say which,
      // so it is not taken for this one and the run stops at it.
      if (e instanceof ApiError && e.code !== "network" && e.code !== "internal" && e.code !== "unavailable") {
        return { ok: false, message: e.message };
      }
      if (attempt >= RESENDS) {
        return { ok: false, message: "접수됐는지 확인하지 못했어요. 규칙의 상태를 확인해 주세요." };
      }
      await wait(2 * POLL_MS);
    }
  }
  let misses = 0;
  while (isOpen(command)) {
    await wait(POLL_MS);
    try {
      command = await getCommand(command.id);
      misses = 0;
    } catch {
      misses += 1;
      if (misses > 10) return { ok: false, message: "결과를 확인하지 못했어요. 규칙의 상태를 확인해 주세요." };
    }
  }
  return { ok: true, command };
}
