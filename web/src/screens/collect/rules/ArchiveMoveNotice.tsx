import { Button } from "@/components/ui/button";
import { isOpen } from "@/lib/commands";
import { ago, dateTime } from "@/lib/time";
import { cn } from "@/lib/utils";

import { btnAction, btnNeutral } from "../channels/styles";
import type { ArchiveDirection, ArchiveMove, Rule } from "./api";
import type { MovePhase } from "./useArchiveMove";

const MOVING: Record<ArchiveDirection, string> = {
  archive: "보관 폴더로 옮기는 중이에요.",
  restore: "수집 폴더로 옮기는 중이에요.",
};

const MOVED: Record<ArchiveDirection, string> = {
  archive: "보관 폴더로 옮겼어요.",
  restore: "수집 폴더로 옮겼어요.",
};

/** The rule changed state and its folder stayed on purpose; the reason says why. */
const KEPT: Record<ArchiveDirection, string> = {
  archive: "규칙을 보관했어요. 폴더는 옮기지 않았어요.",
  restore: "규칙을 복원했어요. 폴더는 옮기지 않았어요.",
};

const FAILED_AFTER: Record<ArchiveDirection, string> = {
  archive: "규칙은 보관된 채 폴더는 제자리에 있어요.",
  restore: "폴더를 되돌리지 못해서 규칙은 보관된 채로 있어요.",
};

/** Whether the ended move still says something about the rule as it is now. */
function stillTelling(move: ArchiveMove, rule: Rule): boolean {
  if (move.command.state === "failed") return true;
  return move.direction === "archive" ? rule.state === "archived" : rule.state === "active";
}

interface Line {
  title: string;
  detail: string | null;
  failed: boolean;
  at: number | null;
}

function endedLine(move: ArchiveMove): Line {
  const { command, direction } = move;
  const reason = command.outcome?.reason ?? null;
  const at = command.finished_at;
  if (command.state === "failed") {
    return {
      title: "옮기지 못했어요.",
      detail: [reason ?? "까닭은 알 수 없어요.", FAILED_AFTER[direction]].join(" "),
      failed: true,
      at,
    };
  }
  if (command.outcome?.result === "kept") {
    return { title: KEPT[direction], detail: reason, failed: false, at };
  }
  return { title: MOVED[direction], detail: reason, failed: false, at };
}

function phaseLine(phase: MovePhase): Line | null {
  switch (phase.kind) {
    case "sending":
      return { title: MOVING[phase.direction], detail: "접수하는 중이에요.", failed: false, at: null };
    case "waiting":
      return {
        title: MOVING[phase.direction],
        detail: "접수됐어요. 끝나면 여기에 결과가 나와요.",
        failed: false,
        at: null,
      };
    case "unconfirmed":
      return {
        title: MOVING[phase.direction],
        detail: phase.lost
          ? "접수됐는지 확인했더니 서버에 요청이 없어요. 같은 요청을 다시 보낼 수 있어요."
          : "접수됐는지 확인하지 못했어요. 결과를 알 수 없어 다시 확인하고 있어요.",
        failed: false,
        at: null,
      };
    case "ended":
      return endedLine(phase.move);
    case "idle":
      return null;
  }
}

interface ArchiveMoveNoticeProps {
  rule: Rule;
  phase: MovePhase;
  /** `다시 옮기기`: archive the archived rule again. */
  onMoveAgain: () => void;
  onResend: () => void;
  onRecheck: () => void;
}

/**
 * Where the rule's work folder went on the last `보관`·`복원`, in sentences:
 * moving, moved, kept (and why) or not moved (and why). After an archive that
 * could not move the folder, `다시 옮기기` tries again.
 */
export function ArchiveMoveNotice({ rule, phase, onMoveAgain, onResend, onRecheck }: ArchiveMoveNoticeProps) {
  let line = phaseLine(phase);
  let move: ArchiveMove | null = phase.kind === "ended" ? phase.move : null;
  if (line === null && rule.archive_move && !isOpen(rule.archive_move.command)) {
    move = rule.archive_move;
    line = stillTelling(move, rule) ? endedLine(move) : null;
  }
  if (line === null) return null;

  const moveAgain =
    phase.kind !== "sending" &&
    phase.kind !== "waiting" &&
    phase.kind !== "unconfirmed" &&
    move?.direction === "archive" &&
    move.command.state === "failed" &&
    rule.state === "archived";

  return (
    <section
      role="status"
      aria-label="폴더 이동"
      data-testid="archive-move"
      className={cn(
        "flex min-w-0 flex-col gap-2 rounded-xl border bg-surface-2 px-3.5 py-3 text-[13px] leading-normal",
        line.failed ? "border-[color-mix(in_srgb,var(--accent-urgent)_50%,transparent)]" : "border-hairline",
      )}
    >
      <p className={cn("font-bold", line.failed ? "text-urgent" : "text-text-primary")}>
        {line.title}
        {line.at !== null && (
          <span className="ml-1.5 font-normal text-text-muted" title={dateTime(line.at)}>
            {ago(line.at)}
          </span>
        )}
      </p>
      {line.detail && <p className="break-words text-text-secondary">{line.detail}</p>}
      {(moveAgain || phase.kind === "unconfirmed") && (
        <div className="flex flex-wrap gap-2">
          {moveAgain && (
            <Button type="button" variant="ghost" className={btnAction} onClick={onMoveAgain}>
              다시 옮기기
            </Button>
          )}
          {phase.kind === "unconfirmed" &&
            (phase.lost ? (
              <Button type="button" variant="ghost" className={btnAction} onClick={onResend}>
                같은 요청 다시 보내기
              </Button>
            ) : (
              <Button type="button" variant="ghost" className={btnNeutral} onClick={onRecheck}>
                다시 확인
              </Button>
            ))}
        </div>
      )}
    </section>
  );
}
