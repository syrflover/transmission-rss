import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

import { btnPrimary } from "../collect/channels/styles";
import type { ConfirmView, JobDetail } from "./api";
import { useConfirm } from "./PlacementConfirm";
import { REPLACES } from "./placementTable";
import { moves, relocationLead, relocationRequest, type Move, type MoveLine } from "./relocation";

/**
 * The 배치 확인 of a relocation job (재배치): the copies a mapping change takes off their old episodes beside the
 * episodes their subtitles go on, confirmed as a whole with `적용`. Nothing moves before.
 */
export function RelocationConfirm({ job, confirm }: { job: JobDetail; confirm: ConfirmView }) {
  const { sending, problem, send } = useConfirm(job.id);
  const shown = moves(
    job.placements.filter((p) => confirm.positions.includes(p.position)),
    job.relocations.filter((r) => r.state === "planned"),
    confirm.episodes,
  );
  const apply = () => {
    const body = relocationRequest(job.placements, confirm, job.relocations);
    void send(body.rows, body.removals);
  };

  return (
    <section
      aria-label="배치 확인"
      className="mt-5 rounded-card border border-hairline bg-surface-1 p-3.5 shadow-(--card-shadow) max-[720px]:mt-3"
    >
      <h2 className="text-[17px] font-bold">배치 확인</h2>
      <p className="mt-1 text-[13.5px] leading-relaxed text-text-secondary max-[720px]:text-[13px]">
        {relocationLead(job.relocations)}
      </p>
      <MoveTable moves={shown} />
      <div className="mt-3.5 flex flex-col items-end gap-2 max-[720px]:items-stretch">
        <Button
          type="button"
          variant="ghost"
          className={cn(btnPrimary, "max-[720px]:w-full")}
          disabled={sending}
          onClick={apply}
        >
          적용
        </Button>
        {problem !== null && (
          <p role="alert" className="text-[13px] font-semibold text-urgent">
            {problem}
          </p>
        )}
      </div>
    </section>
  );
}

/** What came of a confirmed relocation: each copy taken off or left, and each new episode's application. */
export function RelocationResults({ job }: { job: JobDetail }) {
  const shown = moves(job.placements, job.relocations, null);
  if (shown.length === 0) return <p className="text-[13px] text-text-muted">옮긴 적용본이 없어요.</p>;
  return (
    <div className="rounded-card border border-hairline bg-surface-1 px-3.5 py-1 shadow-(--card-shadow)">
      <MoveTable moves={shown} />
    </div>
  );
}

/** `파일 → 옛 회차 → 새 회차`: one row per stored subtitle. A phone stacks the three. */
function MoveTable({ moves }: { moves: readonly Move[] }) {
  const grid = "grid grid-cols-[minmax(0,1.1fr)_minmax(0,1fr)_minmax(0,1fr)] gap-x-4";
  return (
    <div role="table" aria-label="파일 → 옛 회차 → 새 회차" className="mt-3 flex flex-col">
      <div
        role="row"
        className={cn(grid, "border-b border-hairline pb-1.5 text-xs font-semibold text-text-muted max-[720px]:hidden")}
      >
        <span role="columnheader">파일</span>
        <span role="columnheader">옛 회차</span>
        <span role="columnheader">새 회차</span>
      </div>
      {moves.map((m) => (
        <div
          key={m.key}
          role="row"
          className={cn(
            grid,
            "items-start gap-y-1.5 border-b border-hairline-soft py-2.5 last:border-b-0 max-[720px]:grid-cols-1",
          )}
        >
          <span role="cell" className="min-w-0 text-[13px] leading-snug font-semibold [overflow-wrap:anywhere]">
            {m.name}
          </span>
          <div role="cell" className="flex min-w-0 flex-col gap-1.5">
            {m.off.map((line, i) => (
              <Line key={i} line={line} />
            ))}
          </div>
          <div role="cell" className="min-w-0">
            <Line line={m.on} />
          </div>
        </div>
      ))}
    </div>
  );
}

function Line({ line }: { line: MoveLine }) {
  return (
    <div className="flex min-w-0 flex-col gap-0.5">
      <span className={cn("text-[13px] leading-snug", line.urgent ? "font-semibold text-urgent" : "text-text-primary")}>
        {line.text}
      </span>
      {line.detail !== null && line.detail !== "" && (
        <span className="text-xs leading-snug text-text-muted [overflow-wrap:anywhere]">{line.detail}</span>
      )}
      {line.replaces && <span className="text-xs leading-snug text-text-secondary">{REPLACES}</span>}
    </div>
  );
}
