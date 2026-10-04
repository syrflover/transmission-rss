import { useId, useState, type ComponentProps, type ComponentType } from "react";

import { when } from "@/lib/time";
import { cn } from "@/lib/utils";

import type { Step, StepKind } from "./api";
import { CheckIcon, ChevronIcon, CircleIcon, ClockIcon, DotIcon, WarningIcon } from "./icons";

const LABEL: Record<StepKind, string> = {
  found: "후보 발견",
  open: "게시물 열기",
  auth: "인증",
  receive: "받기",
  placement: "배치 확인",
  store: "보관",
  approval: "교체 승인",
  apply: "적용",
};

type IconType = ComponentType<ComponentProps<"svg">>;

const MARK: Record<Step["state"], { icon: IconType; tone: string; word: string }> = {
  done: { icon: CheckIcon, tone: "border-ok text-ok", word: "완료" },
  current: { icon: DotIcon, tone: "border-focus text-focus", word: "진행 중" },
  waiting: { icon: ClockIcon, tone: "border-text-muted border-dashed text-text-secondary", word: "대기" },
  failed: { icon: WarningIcon, tone: "border-urgent text-urgent", word: "실패" },
  partial: { icon: WarningIcon, tone: "border-urgent text-urgent", word: "일부 실패" },
  upcoming: { icon: CircleIcon, tone: "border-hairline text-text-muted", word: "예정" },
};

/** The step the job is at: one that is moving, stopped for something, or ended badly. */
function isHere(step: Step): boolean {
  return step.state === "current" || step.state === "waiting" || step.state === "failed" || step.state === "partial";
}

function Mark({ state }: { state: Step["state"] }) {
  const { icon: Icon, tone } = MARK[state];
  return (
    <span
      aria-hidden="true"
      className={cn("inline-flex size-5 flex-none items-center justify-center rounded-full border-[1.5px] bg-surface-1", tone)}
    >
      <Icon className="size-3" />
    </span>
  );
}

/**
 * The steps the job went through and the one it is at, with their times. A
 * horizontal track that wraps on a computer; on a phone it folds into one line
 * about the current step, which opens the whole list.
 */
export function JobSteps({ steps }: { steps: readonly Step[] }) {
  const [open, setOpen] = useState(false);
  const listId = useId();
  const here = steps.find(isHere);
  // A finished job has no step it is at: the line speaks of the last one it went through.
  const summary = here ?? [...steps].reverse().find((s) => s.state === "done") ?? steps[0];
  if (steps.length === 0) return null;
  const number = steps.indexOf(summary) + 1;

  return (
    <div>
      <button
        type="button"
        aria-expanded={open}
        aria-controls={listId}
        onClick={() => setOpen((was) => !was)}
        className="hidden min-h-10 w-full items-center gap-2.5 rounded-card border border-hairline bg-surface-1 px-3 text-left text-[13.5px] shadow-(--card-shadow) hover:border-text-secondary max-[720px]:flex"
      >
        <Mark state={summary.state} />
        <span className="flex min-w-0 flex-1 flex-wrap items-baseline gap-x-2.5">
          <b className="font-bold">{LABEL[summary.step]}</b>
          <span className="text-text-secondary">{MARK[summary.state].word}</span>
          {summary.at !== null && <span className="text-xs text-text-muted">{when(summary.at)}</span>}
        </span>
        <span className="text-xs text-text-muted tabular-nums">
          {number}/{steps.length}
        </span>
        <ChevronIcon className={cn("size-4 flex-none text-text-muted transition-transform", open && "rotate-90")} />
      </button>

      <ol
        id={listId}
        aria-label="작업 단계"
        className={cn(
          "m-0 grid list-none grid-cols-[repeat(auto-fit,minmax(150px,1fr))] gap-2 p-0",
          "max-[720px]:mt-2 max-[720px]:grid-cols-2",
          !open && "max-[720px]:hidden",
        )}
      >
        {steps.map((step) => {
          const mark = MARK[step.state];
          return (
            <li
              key={step.step}
              aria-current={step === here ? "step" : undefined}
              className={cn(
                "flex min-w-0 flex-col gap-1 rounded-card border bg-surface-1 px-3 py-2.5 shadow-(--card-shadow)",
                step.state === "failed" || step.state === "partial"
                  ? "border-urgent"
                  : step === here
                    ? "border-focus"
                    : "border-hairline",
              )}
            >
              <span className="flex items-center gap-2 text-[13.5px] leading-tight font-bold">
                <Mark state={step.state} />
                <span className={cn("min-w-0", step.state === "upcoming" && "font-semibold text-text-muted")}>
                  {LABEL[step.step]}
                  <span className="sr-only"> {mark.word}</span>
                </span>
              </span>
              {step.at !== null && (
                <time className="pl-7 text-[11.5px] leading-snug text-text-muted max-[720px]:pl-0">{when(step.at)}</time>
              )}
              {step.note !== null && step.note !== "" && (
                <span className="pl-7 text-[11.5px] leading-snug text-text-secondary max-[720px]:pl-0">{step.note}</span>
              )}
            </li>
          );
        })}
      </ol>
    </div>
  );
}
