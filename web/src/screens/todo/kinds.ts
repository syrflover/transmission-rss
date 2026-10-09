/**
 * The kinds of `처리 필요` as a badge names them (`docs/specs/jobs.md`, 할 일). The 할 일 screen's cards, the library
 * grid's covers and the work detail's `할 일` show the same name and colour (`badges.tsx`, `TodoBadge`). The server
 * decides which badge a to-do has: a job's 배치 확인 and a video's episode are a `회차 확인 필요` like a mapping's.
 */
export type TodoKind = "auth" | "receive_failed" | "replacement" | "episode_check";

/**
 * What these read of a to-do (`api.ts`, `Todo`): its work. Said here rather than imported, so the tests run without
 * the app's module paths. A to-do's badge and a work's badges are the server's (`api.ts`, `Todo.badge` and
 * `TodoList.badges`).
 */
export interface TodoOf {
  work: { id: string } | null;
}

/** One work's to-dos, in the list's order. */
export function todosOfWork<T extends TodoOf>(todos: readonly T[], workId: string): T[] {
  return todos.filter((todo) => todo.work?.id === workId);
}

/**
 * The works with their badges as `kinds` has them, a work with none having none. A work whose badges are the same
 * stays the same object, and so do the works when none changed, so the list draws again only what changed.
 */
export function withKinds<W extends { id: string; todos: TodoKind[] }>(
  works: W[],
  kinds: ReadonlyMap<string, TodoKind[]>,
): W[] {
  let changed = false;
  const next = works.map((work) => {
    const todos = kinds.get(work.id) ?? [];
    if (todos.length === work.todos.length && todos.every((kind, i) => kind === work.todos[i])) return work;
    changed = true;
    return { ...work, todos };
  });
  return changed ? next : works;
}
