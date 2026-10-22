import type { ArchiveBatchFailure } from "../../../lib/api-types/ArchiveBatchFailure";
import type { ArchiveBatchView } from "../../../lib/api-types/ArchiveBatchView";

import { archivedLine } from "./archivedLine.ts";

// The types are generated (`lib/api-types`). They are imported by a relative path and only as types; the one module
// imported at run time is relative too, so this module runs and is type-checked under `node --test`.
export type { ArchiveBatchFailure, ArchiveBatchView };

/** What became of one rule of a run, as `RunStatus` lists it. */
export interface RunResult {
  ruleId: string;
  /** The rule is deleted, and what became of its work folder ({@link archivedLine}). */
  text: string;
  /** The work, for a link to its detail; `null` when there is none to link. */
  workId: string | null;
}

/** Where a `보관` of one or several suggestions is, as the 구독 tab's `RunStatus` shows it. */
export interface Run {
  total: number;
  /** How many were archived so far. */
  finished: number;
  /** The work being archived now; `null` once the run ended. */
  current: string | null;
  /** Why the run stopped before the end. */
  error: string | null;
  /** What became of each archived rule, once the run ended; empty before. */
  results: RunResult[];
  /** The work whose move stopped the run, for a link to its detail, where it moves again. */
  errorWorkId: string | null;
}

/** Said when the batch's progress could not be read often enough to know how it ended. */
export const LOST = "결과를 확인하지 못했어요. 규칙의 상태를 확인해 주세요.";

/** Said when a batch failed and the server named no rule (it names one whenever it can). */
const UNKNOWN_FAILURE = "일괄 보관이 끝까지 가지 못했어요. 규칙의 상태를 확인해 주세요.";

/** Whether the server has not ended the batch yet. */
export function isOpen(view: Pick<ArchiveBatchView, "state">): boolean {
  return view.state === "pending" || view.state === "running";
}

/**
 * Why a batch stopped: a rule that was not archived (its work folder is moving
 * already, or it kept changing) says the server's sentence after its name, and
 * a rule whose work folder could not be moved says it is deleted all the same,
 * why the folder stayed and where to move it again.
 */
export function failureText(failure: ArchiveBatchFailure | null): string {
  if (failure === null) return UNKNOWN_FAILURE;
  if (failure.kind === "refused") return `‘${failure.name}’: ${failure.reason}`;
  return archivedLine(failure.name, { state: "failed", outcome: { result: "failed", reason: failure.reason } }, null)
    .text;
}

/** The run a batch's view says: its progress while open, how it ended and what became of each rule after. */
export function runOfView(view: ArchiveBatchView): Run {
  if (isOpen(view)) {
    return {
      total: view.total,
      finished: view.finished,
      current: view.current?.name ?? "",
      error: null,
      results: [],
      errorWorkId: null,
    };
  }
  return {
    total: view.total,
    finished: view.finished,
    current: null,
    error: view.state === "failed" ? failureText(view.failure) : null,
    results: view.results.map((result) => ({
      ruleId: result.rule_id,
      text: archivedLine(result.name, result.command, null).text,
      workId: result.work_id,
    })),
    errorWorkId: view.state === "failed" ? (view.failure?.work_id ?? null) : null,
  };
}

/** What was seen of a batch the last time it was read. */
export interface Seen {
  finished: number;
  /** The rule that was being archived. */
  current: string | null;
}

export function seenOf(view: ArchiveBatchView): Seen {
  return { finished: view.finished, current: view.current?.rule_id ?? null };
}

/**
 * The rules that finished between `seen` and `view`, in order, for the cached
 * suggestions to leave out as each is archived. A screen that sent the batch
 * knows its rules (`known`, in the order sent). One that found the batch
 * running when it opened knows only the rule that was in turn: it names that
 * one when the batch moved a single rule on, and none when it moved further,
 * which the list read again at the end puts right.
 */
export function newlyFinished(known: readonly string[] | null, seen: Seen, view: ArchiveBatchView): string[] {
  if (view.finished <= seen.finished) return [];
  if (known !== null) return known.slice(seen.finished, view.finished);
  return seen.current !== null && view.finished === seen.finished + 1 ? [seen.current] : [];
}
