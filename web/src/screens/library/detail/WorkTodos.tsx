import type { ReactNode } from "react";
import { Link } from "react-router-dom";

import { Button } from "@/components/ui/button";
import { when } from "@/lib/time";
import { cn } from "@/lib/utils";

import { btnAction } from "../../collect/channels/styles";
import {
  fetchTodos,
  jobPath,
  type AuthTodo,
  type EpisodeCheckTodo,
  type PlacementCheckTodo,
  type ReceiveFailedTodo,
  type ReplacementTodo,
  type Todo,
  type TodoList,
} from "../../todo/api";
import { kindTone, OriginTags, Tag, TodoBadge } from "../../todo/badges";
import { receivedLine, todoTags } from "../../todo/changes";
import { LockIcon } from "../../todo/icons";
import { kindOf, todosOfWork } from "../../todo/kinds";
import { KEYS, TODOS_MS, usePolled } from "../../todo/poll";
import { TargetLine } from "../../todo/TargetLine";
import { episodeCheckPath, episodeCheckReason, receiveFailedAction, receiveFailedPath } from "../../todo/TodoCards";

/**
 * The work's `할 일` (`docs/specs/library.md`, 할 일과 회차 목록): its to-dos that need the person, as cards with a
 * coloured border, above the 자막 후보. They are the 할 일 screen's (`KEYS.todos`, read again while the page is
 * visible), so one handled anywhere is gone at the next read. A work with none, or before the list is read, has no
 * heading, list or space.
 *
 * A card has no cover and no title: the page is the work's. Its first line is what it is about (episodes, creator,
 * season), then what the kind needs said, and its button opens where the person acts: the job detail (`인증`,
 * `비교`, `확인`), the creator's group in 자막 후보, or the episode's row.
 */
export function WorkTodos({ workId, seasonCount }: { workId: string; seasonCount: number }) {
  const todos = usePolled<TodoList>(KEYS.todos, fetchTodos, TODOS_MS, "할 일을 불러오지 못했어요.");
  const mine = todosOfWork(todos.data?.needs ?? [], workId);
  if (mine.length === 0) return null;
  // The season is said only where the work has several.
  const season = (n: number | null) => seasonCount > 1 && n !== null && <Tag>시즌 {n}</Tag>;
  return (
    <section aria-labelledby="work-todos-title">
      <h2 id="work-todos-title" className="mb-3 text-[17px] font-bold">
        할 일
      </h2>
      <ul className="m-0 grid list-none grid-cols-[repeat(auto-fill,minmax(min(100%,280px),1fr))] gap-2.5 p-0">
        {mine.map((todo) => (
          <li key={todo.key} className="min-w-0">
            <WorkTodoCard todo={todo} season={season} />
          </li>
        ))}
      </ul>
    </section>
  );
}

type SeasonTag = (n: number | null) => ReactNode;

function WorkTodoCard({ todo, season }: { todo: Todo; season: SeasonTag }) {
  switch (todo.kind) {
    case "auth":
      return <AuthCard todo={todo} season={season} />;
    case "receive_failed":
      return <ReceiveFailedCard todo={todo} season={season} />;
    case "replacement":
      return <ReplacementCard todo={todo} season={season} />;
    case "episode_check":
      return <EpisodeCheckCard todo={todo} season={season} />;
    case "placement_check":
      return <PlacementCheckCard todo={todo} season={season} />;
  }
}

/** The kind badge and the time on top, the lines under them, and the button at the bottom right. */
function Card({ todo, children, action }: { todo: Todo; children: ReactNode; action: ReactNode }) {
  const kind = kindOf(todo);
  return (
    <article
      className={cn(
        "flex h-full flex-col gap-1.5 rounded-card border bg-surface-1 p-3 shadow-(--card-shadow)",
        kindTone(kind) === "urgent"
          ? "border-[color-mix(in_srgb,var(--accent-urgent)_55%,var(--hairline))]"
          : "border-[color-mix(in_srgb,var(--focus-ring)_55%,var(--hairline))]",
      )}
    >
      <div className="flex items-center justify-between gap-2">
        <TodoBadge kind={kind} />
        <time className="text-xs whitespace-nowrap text-text-muted">{when(todo.at)}</time>
      </div>
      {children}
      <div className="mt-auto flex justify-end pt-1.5">{action}</div>
    </article>
  );
}

function Reason({ children }: { children: ReactNode }) {
  return <p className="min-w-0 text-[12.5px] leading-snug text-text-secondary">{children}</p>;
}

function Action({ to, children }: { to: string; children: ReactNode }) {
  return (
    <Button asChild variant="ghost" className={btnAction}>
      <Link to={to}>{children}</Link>
    </Button>
  );
}

/** `인증 필요`: the site's check (`CAPTCHA`) and since when it asks; `인증` opens the job. */
function AuthCard({ todo, season }: { todo: AuthTodo; season: SeasonTag }) {
  return (
    <Card
      todo={todo}
      action={
        <Action to={jobPath(todo.job_id)}>
          <LockIcon className="size-[15px]" />
          인증
        </Action>
      }
    >
      <TargetLine episodes={todo.episodes} creator={todo.creator}>
        {season(todo.season)}
        {todo.jobs > 1 && <Tag>작업 {todo.jobs}개</Tag>}
      </TargetLine>
      {todo.reason !== "" && <Reason>{todo.reason}</Reason>}
    </Card>
  );
}

function ReceiveFailedCard({ todo, season }: { todo: ReceiveFailedTodo; season: SeasonTag }) {
  return (
    <Card todo={todo} action={<Action to={receiveFailedPath(todo)}>{receiveFailedAction(todo)}</Action>}>
      <TargetLine episodes={todo.episodes} creator={null}>
        {season(todo.season)}
        <Tag>{todo.context === "revision" ? "영상 수정본" : "추가 실패"}</Tag>
        {todo.count > 1 && <Tag>{todo.count}개</Tag>}
      </TargetLine>
      {todo.reason !== null && todo.reason !== "" && <Reason>{todo.reason}</Reason>}
    </Card>
  );
}

/**
 * `교체 승인`: one line of when the current and the new subtitle were received, then the number tags of what the
 * open plans change; `비교` opens the job.
 */
function ReplacementCard({ todo, season }: { todo: ReplacementTodo; season: SeasonTag }) {
  const received = receivedLine(todo);
  return (
    <Card todo={todo} action={<Action to={jobPath(todo.job_id)}>비교</Action>}>
      <TargetLine episodes={todo.episodes.map(String)} creator={todo.creator}>
        {season(todo.season)}
        {todo.jobs > 1 && <Tag>작업 {todo.jobs}개</Tag>}
      </TargetLine>
      {received.length > 0 && (
        <p className="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-0.5 text-[12.5px] leading-snug">
          {received.map((fact) => (
            <span key={fact.label} className="whitespace-nowrap">
              <span className="text-text-muted">{fact.label}</span>{" "}
              <span className="text-text-secondary">{fact.text}</span>
            </span>
          ))}
        </p>
      )}
      <span className="flex flex-wrap items-center gap-1">
        {todoTags(todo.changes).map((tag) => (
          <Tag key={tag}>{tag}</Tag>
        ))}
      </span>
    </Card>
  );
}

/** A mapping's `회차 확인 필요`: the creator and the episodes; it opens the creator's group in 자막 후보. */
function EpisodeCheckCard({ todo, season }: { todo: EpisodeCheckTodo; season: SeasonTag }) {
  return (
    <Card todo={todo} action={<Action to={episodeCheckPath(todo)}>회차 대응 정하기</Action>}>
      <TargetLine episodes={todo.episodes} creator={todo.creator}>
        <Tag>{todo.reason === null ? "어긋난 회차" : "대응 미정"}</Tag>
        {season(todo.season)}
        {todo.sources > 1 && <Tag>확인 {todo.sources}건</Tag>}
      </TargetLine>
      <Reason>{episodeCheckReason(todo)}</Reason>
    </Card>
  );
}

/** A job's 배치 확인 or held files: its source (how it came, the creator, the host) and how many files; `확인`. */
function PlacementCheckCard({ todo, season }: { todo: PlacementCheckTodo; season: SeasonTag }) {
  const job = { origin: todo.origin, creator: todo.creator, revision_of: null, revises_attributed: false };
  return (
    <Card todo={todo} action={<Action to={jobPath(todo.job_id)}>확인</Action>}>
      <TargetLine episodes={[]} creator={todo.creator}>
        {season(todo.season)}
        <OriginTags job={job} />
        {todo.source !== null && (
          <span className="min-w-0 text-xs text-text-muted [overflow-wrap:anywhere]">{todo.source}</span>
        )}
        {todo.files.length > 0 && <Tag>파일 {todo.files.length}개</Tag>}
      </TargetLine>
      <Reason>{todo.reason ?? "받은 파일의 회차를 정하지 못했어요."}</Reason>
    </Card>
  );
}
