import { api, ApiError } from "@/lib/api";
import { getCommand, isOpen, newCommandId, type Command } from "@/lib/commands";
import { sharedEvents } from "@/lib/sharedEvents";
import { CommandWatch } from "@/lib/watchCommands";
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

/** How long to wait before reading again a command whose last read failed, in milliseconds. */
const READ_RETRY_MS = 1000;
/** How long a request that got no answer waits before it is sent again, in milliseconds. */
const RESEND_MS = 2000;
/** How many times a request that got no answer is sent again (with the same ID) before giving up. */
const RESENDS = 4;
/** How many reads of the command may fail in a row before the run gives up on it. */
const MISSES = 10;

const wait = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));

/** How one rule's archive ended: its command ended `done` or `failed`, or it could not be sent at all. */
export type ArchiveOutcome = { ok: true; command: Command } | { ok: false; message: string };

/**
 * Archives one rule through the web command contract (`src/store/commands`):
 * one command ID for the action, resent unchanged when the answer was lost
 * (the server stores a command once per ID), then read by that ID, now and
 * whenever the server says it changed (`lib/watchCommands.ts`), until the
 * worker has ended it. The worker runs commands in the order they were
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
      await wait(RESEND_MS);
    }
  }
  if (!isOpen(command)) return { ok: true, command };
  return follow(command);
}

/** Reads `first`, a command the worker has not ended, until it has, or until reads fail too often. */
function follow(first: Command): Promise<ArchiveOutcome> {
  return new Promise((resolve) => {
    const watching = new CommandWatch(sharedEvents());
    let misses = 0;
    const read = async () => {
      try {
        const command = await getCommand(first.id);
        misses = 0;
        if (!isOpen(command)) {
          watching.stop();
          resolve({ ok: true, command });
        }
      } catch {
        misses += 1;
        if (misses > MISSES) {
          watching.stop();
          resolve({ ok: false, message: "결과를 확인하지 못했어요. 규칙의 상태를 확인해 주세요." });
        } else {
          watching.retry(first.id, read, READ_RETRY_MS);
        }
      }
    };
    watching.watch(first.id, read);
  });
}
