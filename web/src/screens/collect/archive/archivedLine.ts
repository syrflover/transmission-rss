import type { Command } from "../../../lib/api-types/Command";

// The types are generated (`lib/api-types`). They are imported by a relative path and only as types, so this module
// imports nothing at run time and runs and is type-checked under `node --test`.

/** What `보관` of one rule came to, in a sentence. */
export interface ArchivedLine {
  text: string;
  /** The work folder could not be moved: the work detail moves it again. */
  failed: boolean;
}

/**
 * What `보관` of the rule `name` came to: the rule is deleted, and its work folder moved, stays (and why), is moving,
 * or could not move (and why). `command` is the move of the work folder the server stored with the deletion; `null`
 * when the rule had no work folder, `reason` then saying why (or nothing, when it is not known).
 */
export function archivedLine(
  name: string,
  command: Pick<Command, "state" | "outcome"> | null,
  reason: string | null,
): ArchivedLine {
  const deleted = `‘${name}’ 규칙을 지웠어요.`;
  if (command === null) return { text: reason === null ? deleted : `${deleted} ${reason}`, failed: false };
  if (command.state === "pending" || command.state === "running") {
    return { text: `${deleted} 작품 폴더를 보관 폴더로 옮기는 중이에요.`, failed: false };
  }
  const why = command.outcome?.reason ?? null;
  if (command.state === "failed") {
    return {
      text: `‘${name}’ 규칙은 지웠지만 작품 폴더를 옮기지 못했어요. ${why ?? "까닭은 알 수 없어요."} 작품 상세에서 다시 옮길 수 있어요.`,
      failed: true,
    };
  }
  if (command.outcome?.result === "kept") {
    return { text: `${deleted} ${why ?? "작품 폴더는 옮기지 않았어요."}`, failed: false };
  }
  return { text: `‘${name}’ 규칙을 지우고 작품 폴더를 보관 폴더로 옮겼어요.`, failed: false };
}
