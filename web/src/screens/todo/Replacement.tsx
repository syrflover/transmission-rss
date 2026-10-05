import { useRef, useState } from "react";

import { Button } from "@/components/ui/button";
import { refreshTodoCount } from "@/app/todo-count";
import { ApiError } from "@/lib/api";
import { patch, store } from "@/lib/cached";
import { cn } from "@/lib/utils";

import { btnNeutral, btnPrimary } from "../collect/channels/styles";
import { decideReplacement, fetchJob, type JobDetail, type Replacement } from "./api";
import { Badge, Tag } from "./badges";
import { WarningIcon } from "./icons";
import { KEYS } from "./poll";
import {
  againNotice,
  cardLayout,
  compareRows,
  inEpisodeOrder,
  limitNotices,
  openOnes,
  pathRows,
  pathWarnings,
  planEpisode,
  postLabel,
  stateLine,
  versionFacts,
  type CompareRow,
  type Notice,
} from "./replacementView";

const DECIDE_FAILED = "결정을 보내지 못했어요. 잠시 뒤 다시 시도해 주세요.";
const COMPARE_AGAIN = "다시 비교가 필요해요. 아래에 새로 읽은 비교를 보여줘요.";

type Decision = "replace" | "keep";

/**
 * A person's decision on a job's replacements. One request at a time (`sending` names the plan it is for, so every
 * button of the job is disabled meanwhile). The answer's state is put into the cached job at once and the job is
 * read again; a `conflict` (the plan the card showed is not the one to decide any more) reads the job again and says
 * the comparison has to be made again, and the card then shows what the server has now.
 */
function useDecision(jobId: string) {
  const [sending, setSending] = useState<string | null>(null);
  const [problem, setProblem] = useState<{ plan: string; text: string } | null>(null);
  const busy = useRef(false);

  const reread = async () => {
    try {
      store(KEYS.job(jobId), await fetchJob(jobId));
    } catch {
      // The polling reads it again.
    }
  };

  const decide = async (r: Replacement, decision: Decision) => {
    if (busy.current) return;
    busy.current = true;
    setSending(r.plan_id);
    setProblem(null);
    try {
      const { state } = await decideReplacement(jobId, r.plan_id, r.version, decision);
      patch<JobDetail>(KEYS.job(jobId), (was) => ({
        ...was,
        replacements: was.replacements.map((x) =>
          x.plan_id === r.plan_id && x.version === r.version ? { ...x, state } : x,
        ),
      }));
      refreshTodoCount();
      await reread();
    } catch (e) {
      if (e instanceof ApiError && e.code === "conflict") {
        setProblem({ plan: r.plan_id, text: COMPARE_AGAIN });
        await reread();
      } else {
        setProblem({ plan: r.plan_id, text: e instanceof ApiError ? e.message : DECIDE_FAILED });
      }
    } finally {
      busy.current = false;
      setSending(null);
    }
  };

  return { sending, problem, decide };
}

/**
 * The job's replacements under its head: a decision card for each plan to decide, with the warnings and limits that
 * hold under it, then one short line for each plan that is not (or no longer) to decide. The cards are children of
 * the page itself so the first one can follow the page's top on a computer.
 */
export function ReplacementDecisions({ job }: { job: JobDetail }) {
  const { sending, problem, decide } = useDecision(job.id);
  const open = openOnes(job.replacements);
  const lines = inEpisodeOrder(job.replacements).flatMap((r) => {
    const line = stateLine(r);
    return line === null ? [] : [{ r, line }];
  });
  const many = job.replacements.length > 1;

  return (
    <>
      {open.map((r, i) => {
        const notices = [...pathWarnings(r.paths), ...limitNotices(r)];
        return (
          <DecisionCard
            key={r.plan_id}
            r={r}
            many={many}
            layout={cardLayout(open.length, i)}
            sending={sending !== null}
            problem={problem?.plan === r.plan_id ? problem.text : null}
            onDecide={(decision) => void decide(r, decision)}
            notices={notices}
          />
        );
      })}
      {lines.length > 0 && (
        <ul className="m-0 mt-5 flex list-none flex-col gap-2 p-0 max-[720px]:mt-3">
          {lines.map(({ r, line }) => (
            <li
              key={r.plan_id}
              className={cn(
                "rounded-card border bg-surface-1 px-3.5 py-2.5 text-[13px] leading-snug shadow-(--card-shadow)",
                line.urgent ? "border-urgent" : "border-hairline",
              )}
            >
              {many && <b className="mr-2">{planEpisode(r)}</b>}
              <span className={cn("font-semibold", line.urgent && "text-urgent")}>{line.label}</span>
              {line.detail !== null && line.detail !== "" && (
                <span className="mt-0.5 block text-text-secondary [overflow-wrap:anywhere]">{line.detail}</span>
              )}
            </li>
          ))}
        </ul>
      )}
    </>
  );
}

function DecisionCard({
  r,
  many,
  layout,
  sending,
  problem,
  onDecide,
  notices,
}: {
  r: Replacement;
  many: boolean;
  layout: { sticky: boolean; fixed: boolean };
  sending: boolean;
  problem: string | null;
  onDecide: (decision: Decision) => void;
  notices: readonly Notice[];
}) {
  const again = againNotice(r);
  const rows = compareRows(r);
  return (
    <>
      <section
        aria-label={many ? `${planEpisode(r)} 교체 결정` : "교체 결정"}
        className={cn(
          "mt-5 rounded-card border border-hairline bg-surface-1 p-3.5 shadow-(--card-shadow) max-[720px]:mt-3",
          layout.sticky && "min-[721px]:sticky min-[721px]:top-[calc(var(--topbar-h)+8px)] min-[721px]:z-20",
        )}
      >
        <div className="grid grid-cols-[minmax(0,1fr)_auto] items-start gap-x-5 gap-y-3 max-[720px]:grid-cols-1">
          <div className="flex min-w-0 flex-col gap-2.5">
            {many && <b className="text-[14.5px]">{planEpisode(r)}</b>}
            {again !== null && (
              <div className="flex flex-col gap-1">
                <div className="flex flex-wrap items-center gap-1.5">
                  {again.tags.map((tag) => (
                    <Badge key={tag} tone="check" icon={WarningIcon}>
                      {tag}
                    </Badge>
                  ))}
                </div>
                <p className="text-[13px] leading-snug text-text-secondary [overflow-wrap:anywhere]">{again.reason}</p>
              </div>
            )}
            <dl className="m-0 flex flex-col gap-1.5">
              {r.current !== null && <VersionLine name="현재" facts={versionFacts(r.current)} />}
              {r.new !== null && <VersionLine name="새 자막" facts={versionFacts(r.new)} />}
            </dl>
            {rows.length > 0 && <Compare rows={rows} />}
          </div>
          <div
            role="group"
            aria-label="결정"
            className={cn(
              "flex flex-wrap items-center justify-end gap-2",
              "max-[720px]:flex-nowrap",
              !layout.fixed && "max-[720px]:w-full",
              layout.fixed &&
                "max-[720px]:fixed max-[720px]:right-[max(var(--gutter),env(safe-area-inset-right,0px))] max-[720px]:bottom-[calc(var(--bnav-h)+env(safe-area-inset-bottom,0px)+8px)] max-[720px]:left-[max(var(--gutter),env(safe-area-inset-left,0px))] max-[720px]:z-30 max-[720px]:rounded-xl max-[720px]:border max-[720px]:border-hairline max-[720px]:bg-surface-1 max-[720px]:p-2 max-[720px]:shadow-[0_-6px_18px_rgba(0,0,0,0.12)]",
            )}
          >
            <Button
              type="button"
              variant="ghost"
              className={cn(btnNeutral, "max-[720px]:flex-1")}
              disabled={sending}
              onClick={() => onDecide("keep")}
            >
              현재 유지
            </Button>
            <Button
              type="button"
              variant="ghost"
              className={cn(btnPrimary, "max-[720px]:flex-1")}
              disabled={sending}
              onClick={() => onDecide("replace")}
            >
              새 자막으로 교체
            </Button>
          </div>
        </div>
        {problem !== null && (
          <p role="alert" className="mt-2.5 text-[13px] font-semibold text-urgent">
            {problem}
          </p>
        )}
      </section>
      {notices.length > 0 && (
        <ul className="m-0 mt-3 flex list-none flex-col gap-2 p-0">
          {notices.map((n) => (
            <NoticeItem key={n.key} notice={n} />
          ))}
        </ul>
      )}
    </>
  );
}

/** `현재` or `새 자막` and the facts of that file, each in its own span. */
function VersionLine({ name, facts }: { name: string; facts: readonly string[] }) {
  return (
    <div className="flex min-w-0 flex-wrap items-baseline gap-x-3 gap-y-0.5 text-[13.5px]">
      <dt className="w-[3.25rem] flex-none font-bold">{name}</dt>
      {facts.map((fact) => (
        <dd key={fact} className="m-0 text-text-secondary">
          {fact}
        </dd>
      ))}
    </div>
  );
}

/** The two files' facts side by side, shown only when the creator, format or post differs or the source is unknown. */
function Compare({ rows }: { rows: readonly CompareRow[] }) {
  return (
    <table className="w-full table-fixed border-collapse text-[12.5px] leading-snug max-[720px]:text-xs">
      <thead>
        <tr className="text-left text-text-muted">
          <th scope="col" className="w-[4.5rem] pb-1 font-semibold">
            <span className="sr-only">항목</span>
          </th>
          <th scope="col" className="pb-1 font-semibold">
            현재
          </th>
          <th scope="col" className="pb-1 font-semibold">
            새 자막
          </th>
        </tr>
      </thead>
      <tbody>
        {rows.map((row) => (
          <tr key={row.name} className="border-t border-hairline-soft align-top">
            <th scope="row" className="py-1 pr-2 text-left font-semibold text-text-muted">
              {row.name}
            </th>
            <Cell row={row} value={row.current} />
            <Cell row={row} value={row.next} />
          </tr>
        ))}
      </tbody>
    </table>
  );
}

function Cell({ row, value }: { row: CompareRow; value: string }) {
  return (
    <td className="py-1 pr-2 text-text-secondary [overflow-wrap:anywhere]">
      {row.link === true && value !== "—" ? (
        <a
          href={value}
          target="_blank"
          rel="noreferrer noopener"
          className="rounded-sm underline underline-offset-2 focus-visible:outline-2 focus-visible:outline-focus"
        >
          {postLabel(value)}
        </a>
      ) : (
        value
      )}
    </td>
  );
}

/** A warning: a real condition of this decision, with the exact path when it is about one. */
function NoticeItem({ notice }: { notice: Notice }) {
  return (
    <li className="flex items-start gap-2.5 rounded-card border border-[color-mix(in_srgb,var(--accent-urgent)_55%,var(--hairline))] bg-surface-1 px-3.5 py-2.5 shadow-(--card-shadow)">
      <WarningIcon className="mt-0.5 size-4 flex-none text-urgent" />
      <div className="flex min-w-0 flex-col gap-0.5">
        <p className="text-[13px] font-semibold">{notice.title}</p>
        {notice.path !== null && (
          <p className="text-xs leading-snug font-semibold text-text-primary [overflow-wrap:anywhere]">{notice.path}</p>
        )}
        <p className="text-[12.5px] leading-snug text-text-secondary">{notice.text}</p>
      </div>
    </li>
  );
}

/**
 * The replacements' part of `경로`: for each plan, every path beside the video with what happens (or, once done,
 * happened) to it, and the stored file of the subtitle it replaces, which stays whatever is decided.
 */
export function ReplacementPaths({ replacements }: { replacements: readonly Replacement[] }) {
  const plans = inEpisodeOrder(replacements)
    .map((r) => ({ r, rows: pathRows(r) }))
    .filter((x) => x.rows.length > 0);
  return (
    <>
      {plans.map(({ r, rows }) => (
        <div key={r.plan_id} className="flex min-w-0 flex-col gap-1">
          <p className="text-[13px] leading-snug font-semibold">
            {planEpisode(r)} <Tag>교체 계획</Tag>
          </p>
          <dl className="m-0 grid grid-cols-[max-content_minmax(0,1fr)] gap-x-3.5 gap-y-1 text-[13px] max-[720px]:grid-cols-1 max-[720px]:gap-y-0">
            {rows.map((row) => (
              <div key={row.key} className="contents">
                <dt className="font-semibold text-text-muted max-[720px]:mt-1 max-[720px]:first:mt-0">{row.term}</dt>
                <dd className="m-0 min-w-0 leading-snug text-text-secondary [overflow-wrap:anywhere]">
                  {row.path}
                  {row.note !== null && <span className="ml-2 text-xs text-text-muted">{row.note}</span>}
                </dd>
              </div>
            ))}
          </dl>
        </div>
      ))}
    </>
  );
}
