import { Link } from "react-router-dom";

import { Button } from "@/components/ui/button";
import { when } from "@/lib/time";
import { cn } from "@/lib/utils";

import { btnAction } from "../collect/channels/styles";
import { coverOf } from "../library/model";
import { Cover } from "../library/WorkItem";
import {
  jobPath,
  type AuthTodo,
  type EpisodeCheckTodo,
  type PlacementCheckTodo,
  type ReceiveFailedTodo,
  type ReplacementTodo,
  type Todo,
  type VideoCheckTodo,
} from "./api";
import { Tag, TodoBadge } from "./badges";
import { todoTags } from "./changes";
import { LockIcon } from "./icons";
import { candidatesLink } from "./Suggestions";
import { TargetLine } from "./TargetLine";
import { VideoCheckActions } from "./VideoCheck";

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

/** What a `받기 실패` card's button says: where it goes. */
export function receiveFailedAction(todo: ReceiveFailedTodo): string {
  return todo.context === "revision" && todo.work !== null ? "회차 보기" : "기록 보기";
}

/** Where a `회차 확인 필요` card goes: the creator's group in the work's 자막 후보. */
export function episodeCheckPath(todo: EpisodeCheckTodo): string {
  return candidatesLink(todo.work?.id ?? "", todo.season, todo.source_id);
}

/** Why the card asks, in a sentence. */
export function episodeCheckReason(todo: EpisodeCheckTodo): string {
  return todo.reason ?? "정한 대응과 맞지 않아 받지 않은 회차가 있어요.";
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
          {todo.kind === "auth" ? (
            <AuthCard todo={todo} />
          ) : todo.kind === "receive_failed" ? (
            <ReceiveFailedCard todo={todo} />
          ) : todo.kind === "replacement" ? (
            <ReplacementCard todo={todo} />
          ) : todo.kind === "placement_check" ? (
            <PlacementCheckCard todo={todo} />
          ) : todo.kind === "video_check" ? (
            <VideoCheckCard todo={todo} />
          ) : (
            <EpisodeCheckCard todo={todo} />
          )}
        </li>
      ))}
    </ul>
  );
}

/**
 * The shared card: the cover as a full-height column on the left; the kind
 * badge and time, the title, the target line and the reason line on the right,
 * and the button at the bottom right. The red kinds have a red border; the
 * question `회차 확인 필요` asks has the calm blue one.
 */
function Card({
  tone = "urgent",
  kind,
  at,
  title,
  work,
  target,
  reason,
  action,
}: {
  tone?: "urgent" | "check";
  kind: React.ReactNode;
  at: number;
  title: string;
  work: { cover_url: string | null } | null;
  target: React.ReactNode;
  reason: React.ReactNode;
  action: React.ReactNode;
}) {
  return (
    <article
      className={cn(
        "grid h-full min-h-[116px] grid-cols-[64px_minmax(0,1fr)] gap-x-3.5 rounded-card border bg-surface-1 p-3 shadow-(--card-shadow) max-[720px]:grid-cols-[52px_minmax(0,1fr)] max-[720px]:gap-x-3",
        tone === "urgent"
          ? "border-[color-mix(in_srgb,var(--accent-urgent)_55%,var(--hairline))]"
          : "border-[color-mix(in_srgb,var(--focus-ring)_55%,var(--hairline))]",
      )}
    >
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
      kind={<TodoBadge kind="auth" />}
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
      kind={<TodoBadge kind="receive_failed" />}
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

function EpisodeCheckCard({ todo }: { todo: EpisodeCheckTodo }) {
  return (
    <Card
      tone="check"
      kind={<TodoBadge kind="episode_check" />}
      at={todo.at}
      title={todo.title}
      work={todo.work}
      target={
        <TargetLine episodes={todo.episodes} creator={todo.creator}>
          <Tag>{todo.reason === null ? "어긋난 회차" : "대응 미정"}</Tag>
          {todo.season > 1 && <Tag>시즌 {todo.season}</Tag>}
          {todo.sources > 1 && <Tag>확인 {todo.sources}건</Tag>}
        </TargetLine>
      }
      reason={episodeCheckReason(todo)}
      action={
        <Button asChild variant="ghost" className={btnAction}>
          <Link to={episodeCheckPath(todo)}>회차 대응 정하기</Link>
        </Button>
      }
    />
  );
}

/**
 * `교체 승인`: the new subtitle of a work's job waits for a person to approve or refuse replacing the one an episode
 * has. Its reason line is the number tags of what the comparison found (`todoTags`). `비교` opens the job.
 */
function ReplacementCard({ todo }: { todo: ReplacementTodo }) {
  return (
    <Card
      tone="check"
      kind={<TodoBadge kind="replacement" />}
      at={todo.at}
      title={todo.title}
      work={todo.work}
      target={
        <TargetLine episodes={todo.episodes.map(String)} creator={todo.creator}>
          {todo.season !== null && todo.season > 1 && <Tag>시즌 {todo.season}</Tag>}
          {todo.jobs > 1 && <Tag>작업 {todo.jobs}개</Tag>}
        </TargetLine>
      }
      reason={
        <span className="flex flex-wrap items-center gap-1">
          {todoTags(todo.changes).map((tag) => (
            <Tag key={tag}>{tag}</Tag>
          ))}
        </span>
      }
      action={
        <Button asChild variant="ghost" className={btnAction}>
          <Link to={jobPath(todo.job_id)}>비교</Link>
        </Button>
      }
    />
  );
}

/** A job's received files whose episode a person has to say: its source and how many files; `확인` opens the job. */
function PlacementCheckCard({ todo }: { todo: PlacementCheckTodo }) {
  return (
    <Card
      tone="check"
      kind={<TodoBadge kind="episode_check" />}
      at={todo.at}
      title={todo.title}
      work={todo.work}
      target={
        <TargetLine episodes={[]} creator={todo.creator}>
          {todo.files.length > 0 && <Tag>받은 파일 {todo.files.length}개</Tag>}
          {todo.season !== null && todo.season > 1 && <Tag>시즌 {todo.season}</Tag>}
        </TargetLine>
      }
      reason={todo.reason ?? "받은 파일의 회차를 정하지 못했어요."}
      action={
        <Button asChild variant="ghost" className={btnAction}>
          <Link to={jobPath(todo.job_id)}>확인</Link>
        </Button>
      }
    />
  );
}

/**
 * A video whose episode its name does not give: the season folder and the file name, and why; `파일 보기` opens the
 * work's 파일 card and `확인함` stops asking about it.
 */
function VideoCheckCard({ todo }: { todo: VideoCheckTodo }) {
  return (
    <Card
      tone="check"
      kind={<TodoBadge kind="episode_check" />}
      at={todo.at}
      title={todo.title}
      work={todo.work}
      target={<p className="min-w-0 text-[12.5px] leading-snug font-semibold [overflow-wrap:anywhere]">{todo.path}</p>}
      reason={todo.reason}
      action={<VideoCheckActions todo={todo} />}
    />
  );
}
