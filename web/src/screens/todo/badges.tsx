import type { ComponentProps, ComponentType, ReactNode } from "react";

import { cn } from "@/lib/utils";

import type { FailureClass, JobRow } from "./api";
import { CheckIcon, ClockIcon, DownloadIcon, LockIcon, PauseIcon, WarningIcon } from "./icons";
import { FAILURE_LABEL, type Shown } from "./format";

type IconType = ComponentType<ComponentProps<"svg">>;

const base =
  "inline-flex flex-none items-center gap-1 rounded-full border px-2.5 py-0.5 text-xs font-bold whitespace-nowrap";

/**
 * A state badge. It always has a border and an icon, never colour alone. Red
 * is for what needs the user (`urgent`); the rest are neutral. `ok` is the one
 * green: the check on something received.
 */
export function Badge({
  tone,
  icon: Icon,
  children,
  className,
}: {
  tone: "urgent" | "neutral" | "ok";
  icon: IconType;
  children: ReactNode;
  className?: string;
}) {
  return (
    <span
      className={cn(
        base,
        tone === "urgent" ? "border-urgent text-urgent" : "border-hairline text-text-secondary",
        tone === "ok" && "font-semibold text-text-primary",
        className,
      )}
    >
      <Icon className={cn("size-3", tone === "ok" && "text-ok")} />
      {children}
    </span>
  );
}

const SHOWN: Record<Exclude<Shown, "done">, { tone: "urgent" | "neutral"; icon: IconType; text: string }> = {
  failed: { tone: "urgent", icon: WarningIcon, text: "실패" },
  partial: { tone: "urgent", icon: WarningIcon, text: "일부 실패" },
  auth: { tone: "urgent", icon: LockIcon, text: "인증 필요" },
  subtitle: { tone: "neutral", icon: ClockIcon, text: "자막 대기" },
  waiting: { tone: "neutral", icon: ClockIcon, text: "대기" },
  pending: { tone: "neutral", icon: ClockIcon, text: "시작 대기" },
  held: { tone: "neutral", icon: PauseIcon, text: "보류" },
  open: { tone: "neutral", icon: DownloadIcon, text: "게시물 여는 중" },
  receive: { tone: "neutral", icon: DownloadIcon, text: "받는 중" },
};

/**
 * A job's state. A `done` job has no badge in a list (its heading says it);
 * the detail passes `withDone` to show `받음`.
 */
export function StateBadge({ shown, withDone = false }: { shown: Shown; withDone?: boolean }) {
  if (shown === "done") {
    return withDone ? (
      <Badge tone="ok" icon={CheckIcon}>
        받음
      </Badge>
    ) : null;
  }
  const { tone, icon, text } = SHOWN[shown];
  return (
    <Badge tone={tone} icon={icon}>
      {text}
    </Badge>
  );
}

/** One episode's state in the job detail. */
export function ItemBadge({ shown }: { shown: Shown | "item-pending" }) {
  if (shown === "done") {
    return (
      <Badge tone="ok" icon={CheckIcon}>
        받음
      </Badge>
    );
  }
  if (shown === "item-pending") {
    return (
      <Badge tone="neutral" icon={ClockIcon}>
        대기
      </Badge>
    );
  }
  const { tone, icon, text } = SHOWN[shown];
  // An item that failed is just `실패`; `일부 실패` and the opening stage belong to the job.
  return (
    <Badge tone={tone} icon={icon}>
      {shown === "failed" ? "실패" : text}
    </Badge>
  );
}

/** The number of a heading's list, as a number only. */
export function CountChip({ children }: { children: ReactNode }) {
  return (
    <span className="inline-flex items-center rounded-full border border-hairline px-2.5 py-px text-xs font-semibold text-text-secondary tabular-nums">
      {children}
    </span>
  );
}

/** A small context tag in a target line (`영상 수정본`, `작업 2개`). */
export function Tag({ children }: { children: ReactNode }) {
  return (
    <span className="inline-flex items-center rounded-md border border-hairline-soft bg-surface-2 px-1.5 py-px text-[11.5px] leading-snug font-semibold whitespace-nowrap text-text-secondary">
      {children}
    </span>
  );
}

/**
 * How a job came to be, when it was not a pick: `자동` for the subscribed creator's episode the app made into a
 * job, `수정본` for a revision of a subtitle received before.
 */
export function OriginTags({
  job,
}: {
  job: Pick<JobRow, "origin" | "revision_of" | "creator" | "revises_attributed">;
}) {
  return (
    <>
      {job.origin === "auto" && <Tag>자동</Tag>}
      {job.origin === "upload" && <Tag>올림</Tag>}
      {job.origin === "upload" && job.creator === null && <Tag>제작자 알 수 없음</Tag>}
      {(job.revision_of !== null || job.revises_attributed) && <Tag>수정본</Tag>}
    </>
  );
}

/** What made a job, as a sentence: `구독 제작자 자동 수신`, `직접 올림`, or `null` for a pick. */
export function originSentence(job: Pick<JobRow, "origin">): string | null {
  switch (job.origin) {
    case "auto":
      return "구독 제작자 자동 수신";
    case "upload":
      return "직접 올림";
    case "pick":
      return null;
  }
}

/** A failure's class before its reason (`원본 없음`, `만료`). */
export function FailureTag({ failure, className }: { failure: FailureClass; className?: string }) {
  return (
    <span className={cn("mr-1.5 align-[1px]", className)}>
      <Tag>{FAILURE_LABEL[failure]}</Tag>
    </span>
  );
}
