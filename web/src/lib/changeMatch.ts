import type { Change, ChangeKind } from "./api-types";

/**
 * Helpers for saying which changes a screen reads again for. A screen matches
 * a change by the kind of what changed and, where it can, by the ID of its
 * target (`docs/requirements/web-app.md`, 웹 명령과 상태 갱신). It has no `@/` imports so it
 * runs under `node --test`.
 */

/** Whether `change` is of one of `kinds`. */
export function ofKinds(change: Change, kinds: readonly ChangeKind[]): boolean {
  return kinds.includes(change.kind);
}

/**
 * Whether `change` is about the target `id` of its kind: it names that ID, or
 * it names none (`id` is `null`), which means anything of the kind may have
 * changed and so this target too.
 */
export function aboutTarget(change: Change, id: string): boolean {
  return change.id === null || change.id === id;
}
