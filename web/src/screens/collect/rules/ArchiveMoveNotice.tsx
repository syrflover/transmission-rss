import { Button } from "@/components/ui/button";
import { isOpen } from "@/lib/commands";
import { ago, dateTime } from "@/lib/time";
import { cn } from "@/lib/utils";
import type { ArchiveMove, Rule } from "@/lib/api-types";

import { btnNeutral } from "../channels/styles";
import { moveStillTells } from "./moveStillTells";
import type { MovePhase } from "./useArchiveMove";

// `start` (a new rule or subscription) and `resume` (`영상 받기` on) are the
// moves the server makes itself for a rule whose work folder is in the archive
// folder: the rule collects only after the folder came over. Both read alike.
const MOVING = "보관 폴더의 작품 폴더를 수집 폴더로 옮기는 중이에요. 끝나면 받기 시작해요.";
const MOVED = "작품 폴더를 보관 폴더에서 수집 폴더로 옮기고 받기 시작했어요.";
/** The rule was turned on and its folder stayed on purpose; the reason says why. */
const KEPT = "받기 시작했어요. 폴더는 옮기지 않았어요.";
const FAILED_AFTER = "규칙은 멈춘 채 아무것도 받지 않아요. 까닭을 해결한 뒤 영상 받기를 다시 켜 주세요.";

interface Line {
  title: string;
  detail: string | null;
  failed: boolean;
  at: number | null;
}

function endedLine(move: ArchiveMove): Line {
  const { command } = move;
  const reason = command.outcome?.reason ?? null;
  const at = command.finished_at;
  if (command.state === "failed") {
    return {
      title: "옮기지 못했어요.",
      detail: [reason ?? "까닭은 알 수 없어요.", FAILED_AFTER].join(" "),
      failed: true,
      at,
    };
  }
  if (command.outcome?.result === "kept") {
    return { title: KEPT, detail: reason, failed: false, at };
  }
  return { title: MOVED, detail: reason, failed: false, at };
}

function phaseLine(phase: MovePhase): Line | null {
  switch (phase.kind) {
    case "waiting":
      return {
        title: MOVING,
        detail: "접수됐어요. 끝나면 여기에 결과가 나와요.",
        failed: false,
        at: null,
      };
    case "unconfirmed":
      return {
        title: MOVING,
        detail: "어디까지 됐는지 확인하지 못했어요. 결과를 알 수 없어 다시 확인하고 있어요.",
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
  onRecheck: () => void;
}

/**
 * Where the rule's work folder went on its last move out of the archive
 * folder, in sentences: moving, moved, kept (and why) or not moved (and why).
 */
export function ArchiveMoveNotice({ rule, phase, onRecheck }: ArchiveMoveNoticeProps) {
  let line = phaseLine(phase);
  if (line === null && rule.archive_move && !isOpen(rule.archive_move.command)) {
    const move = rule.archive_move;
    line = moveStillTells(move, rule.state) ? endedLine(move) : null;
  }
  if (line === null) return null;

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
      {phase.kind === "unconfirmed" && (
        <div className="flex flex-wrap gap-2">
          <Button type="button" variant="ghost" className={btnNeutral} onClick={onRecheck}>
            다시 확인
          </Button>
        </div>
      )}
    </section>
  );
}
