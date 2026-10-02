import { Link } from "react-router-dom";

import { Button } from "@/components/ui/button";
import { when } from "@/lib/time";

import { btnAction } from "../collect/channels/styles";
import { coverOf } from "../library/model";
import { Cover } from "../library/WorkItem";
import { jobPath, type AuthTodo, type ReceiveFailedTodo, type Todo } from "./api";
import { Badge, Tag } from "./badges";
import { LockIcon, WarningIcon } from "./icons";
import { TargetLine } from "./TargetLine";

/** Where a `받기 실패` card goes: the episode in the work, else the failure in the history. */
export function receiveFailedPath(todo: ReceiveFailedTodo): string {
  if (todo.context === "revision" && todo.work !== null) {
    const params = new URLSearchParams();
    const first = todo.episodes[0];
    if (todo.season !== null && first !== undefined) {
      params.set("season", String(todo.season));
      params.set("episode", first);
    }
    const query = params.toString();
    return `/library/${encodeURIComponent(todo.work.id)}${query === "" ? "" : `?${query}`}`;
  }
  if (todo.context === "revision") return "/collect/history";
  const channel = todo.channel_id === null ? "" : `&channel=${encodeURIComponent(todo.channel_id)}`;
  return `/collect/history?result=add_failed${channel}`;
}

function receiveFailedAction(todo: ReceiveFailedTodo): string {
  return todo.context === "revision" && todo.work !== null ? "회차 보기" : "기록 보기";
}

/** The cards of `처리 필요`, in the order the server gives (red kinds first). */
export function TodoCards({ todos }: { todos: readonly Todo[] }) {
  return (
    <ul
      aria-label="처리 필요"
      className="m-0 grid list-none grid-cols-[repeat(auto-fill,minmax(min(100%,340px),1fr))] gap-3 p-0 max-[720px]:gap-2.5"
    >
      {todos.map((todo) => (
        <li key={todo.key} className="min-w-0">
          {todo.kind === "auth" ? <AuthCard todo={todo} /> : <ReceiveFailedCard todo={todo} />}
        </li>
      ))}
    </ul>
  );
}

/**
 * The shared card: the cover as a full-height column on the left; the kind
 * badge and time, the title, the target line and the reason line on the right,
 * and the button at the bottom right. Both kinds are red, so the border is.
 */
function Card({
  kind,
  at,
  title,
  work,
  target,
  reason,
  action,
}: {
  kind: React.ReactNode;
  at: number;
  title: string;
  work: { cover_url: string | null } | null;
  target: React.ReactNode;
  reason: string | null;
  action: React.ReactNode;
}) {
  return (
    <article className="grid h-full min-h-[116px] grid-cols-[64px_minmax(0,1fr)] gap-x-3.5 rounded-card border border-[color-mix(in_srgb,var(--accent-urgent)_55%,var(--hairline))] bg-surface-1 p-3 shadow-(--card-shadow) max-[720px]:grid-cols-[52px_minmax(0,1fr)] max-[720px]:gap-x-3">
      <Cover
        work={coverOf(title)}
        imageUrl={work?.cover_url}
        className="w-full self-stretch rounded-md shadow-none"
        letterClass="text-2xl"
      />
      <div className="flex min-w-0 flex-col gap-1.5">
        <div className="flex items-center justify-between gap-2">
          {kind}
          <time className="text-xs whitespace-nowrap text-text-muted">{when(at)}</time>
        </div>
        <h3 className="line-clamp-2 min-w-0 text-[14.5px] leading-snug font-semibold">{title}</h3>
        {target}
        {reason !== null && reason !== "" && (
          <p className="min-w-0 text-[12.5px] leading-snug text-text-secondary">{reason}</p>
        )}
        <div className="mt-auto flex justify-end pt-1.5">{action}</div>
      </div>
    </article>
  );
}

function AuthCard({ todo }: { todo: AuthTodo }) {
  return (
    <Card
      kind={
        <Badge tone="urgent" icon={LockIcon}>
          인증 필요
        </Badge>
      }
      at={todo.at}
      title={todo.title}
      work={todo.work}
      target={
        <TargetLine episodes={todo.episodes} creator={todo.creator}>
          {todo.jobs > 1 && <Tag>작업 {todo.jobs}개</Tag>}
        </TargetLine>
      }
      reason={todo.reason}
      action={
        <Button asChild variant="ghost" className={btnAction}>
          <Link to={jobPath(todo.job_id)}>
            <LockIcon className="size-[15px]" />
            인증
          </Link>
        </Button>
      }
    />
  );
}

function ReceiveFailedCard({ todo }: { todo: ReceiveFailedTodo }) {
  return (
    <Card
      kind={
        <Badge tone="urgent" icon={WarningIcon}>
          받기 실패
        </Badge>
      }
      at={todo.at}
      title={todo.title}
      work={todo.work}
      target={
        <TargetLine episodes={todo.episodes} creator={null}>
          <Tag>{todo.context === "revision" ? "영상 수정본" : "추가 실패"}</Tag>
          {todo.count > 1 && <Tag>{todo.count}개</Tag>}
        </TargetLine>
      }
      reason={todo.reason}
      action={
        <Button asChild variant="ghost" className={btnAction}>
          <Link to={receiveFailedPath(todo)}>{receiveFailedAction(todo)}</Link>
        </Button>
      }
    />
  );
}
