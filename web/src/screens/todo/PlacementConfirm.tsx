import { useRef, useState } from "react";

import { Button } from "@/components/ui/button";
import { refreshTodoCount } from "@/app/todo-count";
import { ApiError } from "@/lib/api";
import { store } from "@/lib/cached";
import { cn } from "@/lib/utils";

import { btnPrimary } from "../collect/channels/styles";
import { confirmPlacement, fetchJob, type ConfirmView, type JobDetail, type Placement } from "./api";
import { KEYS } from "./poll";
import {
  REPLACES,
  initialChoice,
  leadText,
  mappedNote,
  requestRows,
  restSummary,
  tableRows,
  unsetCount,
  videoText,
  type Choice,
  type RestSummary,
} from "./placementTable";

const SEND_FAILED = "배치를 보내지 못했어요. 잠시 뒤 다시 시도해 주세요.";
const PLAN_CHANGED = "배치 계획이 바뀌었어요. 아래에 새로 읽은 표를 보여줘요.";

const SKIP = "skip";
const UNSET = "unset";

/**
 * The person's answer to a job's 배치 확인. The choices are only what the person changed (by position); every other
 * row reads as the plan has it, so a job read again while the table is open keeps what was chosen. One request at a
 * time. A `conflict` (the job no longer waits, or its rows changed) reads the job again, and the table then shows
 * what the server has now.
 */
export function useConfirm(jobId: string) {
  const [sending, setSending] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);
  const busy = useRef(false);

  const reread = async () => {
    try {
      store(KEYS.job(jobId), await fetchJob(jobId));
    } catch {
      // The polling reads it again.
    }
  };

  const send = async (rows: NonNullable<ReturnType<typeof requestRows>>, removals: readonly string[] = []) => {
    if (busy.current) return;
    busy.current = true;
    setSending(true);
    setProblem(null);
    try {
      await confirmPlacement(jobId, rows, removals);
      refreshTodoCount();
      await reread();
    } catch (e) {
      if (e instanceof ApiError && e.code === "conflict") {
        setProblem(PLAN_CHANGED);
        await reread();
      } else {
        setProblem(e instanceof ApiError ? e.message : SEND_FAILED);
      }
    } finally {
      busy.current = false;
      setSending(false);
    }
  };

  return { sending, problem, send };
}

/**
 * `배치 확인`: the table `파일 → 회차 → 붙을 영상 이름` of a job that waits for a person to place its files, in the
 * job's main area. `적용` sends the whole table at once; a held row (보류한 줄) has to be chosen first. Nothing is
 * written to the work folder before. A plan with only fonts and attachments has no table: the summary and `적용`.
 */
export function PlacementConfirm({ job, confirm }: { job: JobDetail; confirm: ConfirmView }) {
  const rows = tableRows(job.placements, confirm);
  const rest = restSummary(job.placements, confirm);
  const [changed, setChanged] = useState<ReadonlyMap<number, Choice>>(new Map());
  const { sending, problem, send } = useConfirm(job.id);

  const choices = new Map<number, Choice>(rows.map((p) => [p.position, changed.get(p.position) ?? initialChoice(p)]));
  const unset = unsetCount(choices);
  const whole = confirm.scope === "whole";
  const noTable = rows.length === 0;

  const choose = (position: number, choice: Choice) => setChanged((was) => new Map(was).set(position, choice));
  const apply = () => {
    const body = requestRows(rows, choices);
    if (body !== null) void send(body);
  };

  return (
    <section
      aria-label={whole ? "배치 확인" : "보류한 파일의 회차"}
      className="mt-5 rounded-card border border-hairline bg-surface-1 p-3.5 shadow-(--card-shadow) max-[720px]:mt-3"
    >
      <h2 className="text-[17px] font-bold">{whole ? "배치 확인" : "보류한 파일의 회차"}</h2>
      <p className="mt-1 text-[13.5px] leading-relaxed text-text-secondary max-[720px]:text-[13px]">
        {leadText(rows, confirm)}
      </p>

      {!noTable && (
        <div role="table" aria-label="파일 → 회차 → 붙을 영상 이름" className="mt-3 flex flex-col">
          <div
            role="row"
            className="grid grid-cols-[minmax(0,1.3fr)_11rem_minmax(0,1fr)] gap-x-4 border-b border-hairline pb-1.5 text-xs font-semibold text-text-muted max-[720px]:hidden"
          >
            <span role="columnheader">파일</span>
            <span role="columnheader">회차</span>
            <span role="columnheader">붙을 영상 이름</span>
          </div>
          {rows.map((p) => (
            <Row
              key={p.position}
              p={p}
              choice={choices.get(p.position) ?? UNSET}
              episodes={confirm.episodes}
              disabled={sending}
              onChoose={(choice) => choose(p.position, choice)}
            />
          ))}
        </div>
      )}

      <Rest rest={rest} />

      <div className="mt-3.5 flex flex-col items-end gap-2 max-[720px]:items-stretch">
        {unset > 0 && (
          <p className="text-xs text-text-secondary">
            회차를 고르지 않은 파일이 {unset}개 있어요. 모두 고르면 적용할 수 있어요.
          </p>
        )}
        <Button
          type="button"
          variant="ghost"
          className={cn(btnPrimary, "max-[720px]:w-full")}
          disabled={sending || unset > 0}
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

/** One file: its name and why, the episode to choose, and the video it goes beside. A phone stacks the three. */
function Row({
  p,
  choice,
  episodes,
  disabled,
  onChoose,
}: {
  p: Placement;
  choice: Choice;
  episodes: ConfirmView["episodes"];
  disabled: boolean;
  onChoose: (choice: Choice) => void;
}) {
  const video = videoText(choice, episodes);
  const note = mappedNote(p, choice);
  return (
    <div
      role="row"
      className="grid grid-cols-[minmax(0,1.3fr)_11rem_minmax(0,1fr)] items-start gap-x-4 gap-y-2 border-b border-hairline-soft py-2.5 last:border-b-0 max-[720px]:grid-cols-1 max-[720px]:gap-y-1.5"
    >
      <div role="cell" className="flex min-w-0 flex-col gap-0.5">
        <span className="text-[13px] leading-snug font-semibold [overflow-wrap:anywhere]">{p.name}</span>
        {p.question !== null && (
          <span className="text-xs leading-snug font-semibold text-urgent [overflow-wrap:anywhere]">{p.question}</span>
        )}
        {p.action === "store" && p.note !== null && p.note !== "" && (
          <span className="text-xs leading-snug text-text-secondary [overflow-wrap:anywhere]">{p.note}</span>
        )}
        {note !== null && <span className="text-xs leading-snug text-text-muted">{note}</span>}
      </div>
      <div role="cell" className="min-w-0">
        <select
          aria-label={`${p.name}의 회차`}
          value={choice === "unset" ? UNSET : String(choice)}
          disabled={disabled}
          onChange={(e) => {
            const v = e.target.value;
            onChoose(v === SKIP ? SKIP : Number(v));
          }}
          className="min-h-9 w-full rounded-[10px] border border-hairline bg-surface-2 px-2.5 text-[13px] text-text-primary hover:border-text-muted focus-visible:border-focus focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-focus max-[720px]:min-h-11 max-[720px]:text-base"
        >
          {choice === "unset" && (
            <option value={UNSET} disabled>
              회차 고르기
            </option>
          )}
          {episodes.map((e) => (
            <option key={e.episode} value={e.episode}>
              {e.episode}화
            </option>
          ))}
          <option value={SKIP}>적용하지 않음</option>
        </select>
      </div>
      <div role="cell" className="flex min-w-0 flex-col gap-0.5">
        <span
          className={cn(
            "text-[13px] leading-snug [overflow-wrap:anywhere]",
            choice === "skip" || choice === "unset" ? "text-text-muted" : "text-text-primary",
          )}
        >
          {video.text}
        </span>
        {video.replaces && <span className="text-xs leading-snug text-text-secondary">{REPLACES}</span>}
      </div>
    </div>
  );
}

/** What the table leaves out: the fonts and attachments kept with the package, and the files dropped. */
function Rest({ rest }: { rest: RestSummary }) {
  if (rest.kept.length === 0 && rest.dropped.length === 0) return null;
  return (
    <div className="mt-3 flex flex-col gap-2 border-t border-hairline-soft pt-3">
      {rest.kept.length > 0 && (
        <div className="flex flex-col gap-1">
          <p className="text-[13px] leading-snug text-text-secondary">
            {rest.kept.map((g) => `${g.kind} ${g.names.length}개`).join(" · ")}는 이 작품에 보관만 해요.
          </p>
          <ul className="m-0 list-none p-0 text-xs leading-snug text-text-muted">
            {rest.kept
              .flatMap((g) => g.names)
              .map((name, i) => (
                <li key={`${name}:${i}`} className="[overflow-wrap:anywhere]">
                  {name}
                </li>
              ))}
          </ul>
        </div>
      )}
      {rest.dropped.length > 0 && (
        <div className="flex flex-col gap-1">
          <p className="text-[13px] leading-snug text-text-secondary">
            뺀 파일 {rest.dropped.length}개는 보관하지 않아요.
          </p>
          <ul className="m-0 list-none p-0 text-xs leading-snug text-text-muted">
            {rest.dropped.map((d, i) => (
              <li key={`${d.name}:${i}`} className="[overflow-wrap:anywhere]">
                {d.name}
                {d.note !== null && d.note !== "" && <span> · {d.note}</span>}
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}
