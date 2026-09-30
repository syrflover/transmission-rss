/**
 * The count shown on the 할 일 menu: only the tasks that need handling
 * (`처리 필요`), never the suggestions (`제안`). It is `undefined` while
 * nothing supplies a count, and the badge stays hidden then.
 *
 * The 할 일 feature will feed a real count in through this hook; until the
 * task list exists there is nothing to count, so the badge slot stays empty.
 */
export function useTodoCount(): number | undefined {
  return undefined;
}
