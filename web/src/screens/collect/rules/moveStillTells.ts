import type { ArchiveDirection } from "../../../lib/api-types/ArchiveDirection";
import type { RuleState } from "../../../lib/api-types/RuleState";

/**
 * Whether an ended move of a rule's work folder out of the archive folder
 * (`start` or `resume`: a rule that waited for its work folder) still says
 * something about the rule as it is now. It imports only types (by a relative
 * path, erased at run time), so it runs and is type-checked under
 * `node --test`.
 *
 * A move that went through tells while the rule collects. A failed one tells
 * only while the rule is still paused: once the rule was turned on another way
 * (the archive copy cleared by hand, then `영상 받기` on) the old failure is not
 * about it.
 */
export function moveStillTells(
  move: { direction: ArchiveDirection; command: { state: string } },
  ruleState: RuleState,
): boolean {
  return move.command.state === "failed" ? ruleState === "paused" : ruleState === "active";
}
