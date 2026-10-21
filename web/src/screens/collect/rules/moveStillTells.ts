import type { ArchiveDirection } from "../../../lib/api-types/ArchiveDirection";
import type { RuleState } from "../../../lib/api-types/RuleState";

/**
 * Whether an ended folder move still says something about the rule as it is
 * now. It imports only types (by a relative path, erased at run time), so it
 * runs and is type-checked under `node --test`.
 *
 * - An archive tells while the rule is archived, a restore while it collects.
 * - `start` and `resume` (a rule that waited for its work folder) tell that they
 *   moved while the rule collects. A failed one tells only while the rule is
 *   still paused: once the rule was turned on another way (the archive copy
 *   cleared by hand, then `영상 받기` on) the old failure is not about it.
 * - A failed archive or restore tells whatever the rule is now.
 */
export function moveStillTells(
  move: { direction: ArchiveDirection; command: { state: string } },
  ruleState: RuleState,
): boolean {
  const failed = move.command.state === "failed";
  switch (move.direction) {
    case "archive":
      return failed || ruleState === "archived";
    case "restore":
      return failed || ruleState === "active";
    case "start":
    case "resume":
      return failed ? ruleState === "paused" : ruleState === "active";
  }
}
