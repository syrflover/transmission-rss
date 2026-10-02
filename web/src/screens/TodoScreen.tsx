import { useEffect, useId } from "react";

import { setTodoCount } from "@/app/todo-count";

import { EmptyState, ScreenFrame } from "./ScreenFrame";
import { KEYS as COLLECT_KEYS } from "./collect/cache";
import { fetchArchiveSuggestions, type ArchiveSuggestion } from "./collect/archive/api";
import { fetchCandidates, type TitleCandidate } from "./collect/subs/api";
import {
  fetchFollowSuggestions,
  fetchJobs,
  fetchTodos,
  type FollowSuggestion,
  type JobGroups,
  type TodoList,
} from "./todo/api";
import { CountChip } from "./todo/badges";
import { JobGroupsView } from "./todo/JobList";
import { KEYS, usePolled } from "./todo/poll";
import { SuggestionRows, suggestionRows } from "./todo/Suggestions";
import { TodoCards } from "./todo/TodoCards";

/** How often the jobs are read while the screen is visible. */
const JOBS_MS = 2000;
/** How often the to-dos and the suggestions are read. */
const TODOS_MS = 10_000;

/**
 * The 할 일 screen: what needs the user (`처리 필요`, red kinds first), the
 * suggestions that may wait (`제안`), and the subtitle jobs the worker carries
 * out (`자막 작업`: `실패`, `대기 중`, `진행 중`, `최근 완료`). A section with
 * nothing in it is not drawn.
 */
export function TodoScreen() {
  const todos = usePolled<TodoList>(KEYS.todos, fetchTodos, TODOS_MS, "할 일을 불러오지 못했어요.");
  const jobs = usePolled<JobGroups>(KEYS.jobs, fetchJobs, JOBS_MS, "자막 작업을 불러오지 못했어요.");
  const candidates = usePolled<TitleCandidate[]>(
    COLLECT_KEYS.candidates,
    fetchCandidates,
    TODOS_MS,
    "제목 후보를 불러오지 못했어요.",
  );
  const archives = usePolled<ArchiveSuggestion[]>(
    COLLECT_KEYS.archiveSuggestions,
    fetchArchiveSuggestions,
    TODOS_MS,
    "보관 제안을 불러오지 못했어요.",
  );
  const follows = usePolled<FollowSuggestion[]>(
    KEYS.follow,
    fetchFollowSuggestions,
    TODOS_MS,
    "자막 구독 제안을 불러오지 못했어요.",
  );

  // The menu badge and the cards agree: the badge takes the count the cards came with.
  const count = todos.data?.count;
  useEffect(() => {
    if (count !== undefined) setTodoCount(count);
  }, [count]);

  const needs = todos.data?.needs ?? [];
  const rows = suggestionRows(candidates.data ?? [], archives.data ?? [], follows.data ?? []);
  const groups = jobs.data;
  const jobTotal = groups
    ? groups.failed.length + groups.waiting.length + groups.running.length + groups.done.total
    : 0;

  // Every part has answered once: only then can the screen say that there is nothing.
  const parts = [todos, jobs, candidates, archives, follows];
  const answered = parts.every((part) => part.data !== undefined);
  const empty = answered && needs.length === 0 && rows.length === 0 && jobTotal === 0;
  // A part that has no answer and failed is said once, not hidden.
  const failures = [
    ...new Set(parts.filter((part) => part.data === undefined && part.error !== null).map((part) => part.error as string)),
  ];
  const loading = !answered && failures.length === 0 && parts.some((part) => part.slow);

  return (
    <ScreenFrame
      title="할 일"
      meta={
        <p className="m-0 text-[13px] text-text-secondary">
          손봐야 할 일과 자막 작업이 어디까지 왔는지를 한곳에서 봐요.
        </p>
      }
    >
      {failures.map((message) => (
        <p key={message} role="alert" className="pb-3 text-[13px] font-semibold text-urgent">
          {message}
        </p>
      ))}
      {loading && <p className="text-[13px] text-text-muted">불러오는 중이에요.</p>}

      {needs.length > 0 && (
        <Part title="처리 필요" count={needs.length}>
          <TodoCards todos={needs} />
        </Part>
      )}

      {rows.length > 0 && (
        <Part title="제안" count={rows.length}>
          <SuggestionRows rows={rows} />
        </Part>
      )}

      {groups !== undefined && jobTotal > 0 && (
        <Part title="자막 작업">
          <div>
            <JobGroupsView groups={groups} />
          </div>
        </Part>
      )}

      {empty && (
        <EmptyState>
          지금 처리할 일이 없어요. 사람의 확인이 필요한 일이나 받기에 실패한 일, 자막을 받는 작업이 생기면 여기에 모여요.
        </EmptyState>
      )}
    </ScreenFrame>
  );
}

/** A top-level part of the screen: a heading with an optional count chip, then its content. */
function Part({ title, count, children }: { title: string; count?: number; children: React.ReactNode }) {
  const headingId = useId();
  return (
    <section aria-labelledby={headingId} className="mt-8 first:mt-0">
      <div className="mb-3 flex flex-wrap items-center gap-x-2.5 gap-y-1">
        <h2 id={headingId} className="text-[17px] font-bold">
          {title}
        </h2>
        {count !== undefined && <CountChip>{count}</CountChip>}
      </div>
      {children}
    </section>
  );
}
