/**
 * The kinds of `처리 필요` as a badge names them (`docs/specs/jobs.md`, 할 일). The 할 일 screen's cards, the library
 * grid's covers and the work detail's `할 일` show the same name and colour (`badges.tsx`, `TodoBadge`).
 */
export type TodoKind = "auth" | "receive_failed" | "replacement" | "episode_check";

/**
 * What these read of a to-do (`api.ts`, `Todo`): its kind and its work. Said here rather than imported, so the tests
 * run without the app's module paths.
 */
export interface TodoOf {
  kind: TodoKind | "placement_check";
  work: { id: string } | null;
}

/** A to-do's badge: a job's 배치 확인 is a `회차 확인 필요` like a mapping's (the server's `todo_api::Todo::badge`). */
export function kindOf(todo: Pick<TodoOf, "kind">): TodoKind {
  return todo.kind === "placement_check" ? "episode_check" : todo.kind;
}

/**
 * Each work's badges in the to-do list: its kinds once each, in the list's order (red kinds first). The library
 * list's `todos` are made the same way on the server (`todo_api::badges_by_work`). A to-do of no work has none.
 */
export function kindsByWork(todos: readonly TodoOf[]): Map<string, TodoKind[]> {
  const kinds = new Map<string, TodoKind[]>();
  for (const todo of todos) {
    if (todo.work === null) continue;
    const mine = kinds.get(todo.work.id) ?? [];
    const kind = kindOf(todo);
    if (!mine.includes(kind)) mine.push(kind);
    kinds.set(todo.work.id, mine);
  }
  return kinds;
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
