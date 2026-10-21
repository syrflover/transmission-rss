/**
 * Cache keys of the 할 일 screen and the job detail (`lib/cached.ts`). The
 * suggestions use the collect screen's own keys (`collect/cache.ts`), so both
 * screens show and refresh the same copy.
 */
export const KEYS = {
  todos: "todo:list",
  follow: "todo:subtitle-follow",
  jobs: "todo:jobs",
  job: (id: string) => `todo:job:${id}`,
} as const;
