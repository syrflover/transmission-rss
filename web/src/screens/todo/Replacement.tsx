import { Fragment, useId, useRef, useState } from "react";

import { Button } from "@/components/ui/button";
import { refreshTodoCount } from "@/app/todo-count";
import { ApiError } from "@/lib/api";
import { patch, store } from "@/lib/cached";
import { cn } from "@/lib/utils";

import { btnNeutral, btnPrimary } from "../collect/channels/styles";
import { decideReplacement, decideReplacements, fetchJob, type JobDetail, type Replacement } from "./api";
import { Badge, Tag } from "./badges";
import { ChangesSection } from "./Changes";
import { ChevronIcon, WarningIcon } from "./icons";
import { KEYS } from "./poll";
import {
  againNotice,
  asEpisodeList,
  cardLayout,
  compareRows,
  decisionsFor,
  inEpisodeOrder,
  limitNotices,
  openOnes,
  pathRows,
  pathWarnings,
  planEpisode,
  postLabel,
  rowTags,
  stateLine,
  shownVersions,
  versionFacts,
  type CompareRow,
  type Notice,
} from "./replacementView";

const DECIDE_FAILED = "결정을 보내지 못했어요. 잠시 뒤 다시 시도해 주세요.";
const COMPARE_AGAIN = "다시 비교가 필요해요. 아래에 새로 읽은 비교를 보여줘요.";
const COMPARE_AGAIN_ROW = "다시 비교가 필요해요. 줄을 펼쳐 새로 읽은 비교를 확인해 주세요.";

type Decision = "replace" | "keep";
type Decided = "approved" | "kept";
/** What a card or a row says under its buttons: its plan has to be compared again, or why the decision failed. */
type Problem = { again: true } | { again: false; text: string };

const AGAIN: Problem = { again: true };

/**
 * A person's decisions on a job's replacements. One request at a time (`sending` is set meanwhile, so every button of
 * the job is disabled). The answer's states are put into the cached job at once and the job is read again. A plan
 * the answer finds stale (not the one to decide any more) reads the job again and says the comparison has to be made
 * again, and its card or row then shows what the server has now. `problems` holds that by the plan's placement
 * `position`, since a plan made again has an id of its own. A list's request that fails says so once, in `listProblem`.
 */
function useDecision(jobId: string) {
  const [sending, setSending] = useState(false);
  const [problems, setProblems] = useState<ReadonlyMap<number, Problem>>(new Map());
  const [listProblem, setListProblem] = useState<string | null>(null);
  const busy = useRef(false);

  const reread = async () => {
    try {
      store(KEYS.job(jobId), await fetchJob(jobId));
    } catch {
      // The polling reads it again.
    }
  };

  /**
   * Runs one request, then puts what it decided into the cached job and the stale plans' notice by them. `list` is a
   * request for the whole list, whose failure is said once.
   */
  const send = async (
    plans: readonly Replacement[],
    list: boolean,
    request: () => Promise<Map<string, Decided | "stale">>,
  ) => {
    if (busy.current) return;
    busy.current = true;
    setSending(true);
    setProblems(new Map());
    setListProblem(null);
    try {
      const results = await request();
      patch<JobDetail>(KEYS.job(jobId), (was) => ({
        ...was,
        replacements: was.replacements.map((x) => {
          const state = results.get(x.plan_id);
          const sent = plans.find((r) => r.plan_id === x.plan_id);
          return state !== undefined && state !== "stale" && sent?.version === x.version ? { ...x, state } : x;
        }),
      }));
      const stale = plans.filter((r) => results.get(r.plan_id) === "stale");
      setProblems(new Map(stale.map((r) => [r.position, AGAIN])));
      refreshTodoCount();
      await reread();
    } catch (e) {
      if (e instanceof ApiError && e.code === "conflict") {
        setProblems(new Map(plans.map((r) => [r.position, AGAIN])));
        await reread();
      } else {
        const text = e instanceof ApiError ? e.message : DECIDE_FAILED;
        if (list) setListProblem(text);
        else setProblems(new Map(plans.map((r) => [r.position, { again: false, text }])));
      }
    } finally {
      busy.current = false;
      setSending(false);
    }
  };

  const decide = (r: Replacement, decision: Decision) =>
    send([r], false, async () => {
      const { state } = await decideReplacement(jobId, r.plan_id, r.version, decision);
      return new Map([[r.plan_id, state]]);
    });

  /** `모두 교체` or `모두 유지`: one request for every plan the list shows. */
  const decideAll = (open: readonly Replacement[], decision: Decision) =>
    send(open, true, async () => {
      const { results } = await decideReplacements(jobId, decisionsFor(open, decision));
      return new Map(results.map((r) => [r.plan, r.state]));
    });

  return { sending, problems, listProblem, decide, decideAll };
}

/**
 * The job's replacements under its head: the plans to decide, then one short line for each plan that is not (or no
 * longer) to decide. One plan to decide is a decision card with the warnings and limits that hold under it and its
 * `변경 사항`; several are an episode list (`EpisodeList`). The card and the list are children of the page itself so
 * they can follow the page's top on a computer.
 */
export function ReplacementDecisions({ job }: { job: JobDetail }) {
  const { sending, problems, listProblem, decide, decideAll } = useDecision(job.id);
  const open = openOnes(job.replacements);
  const lines = inEpisodeOrder(job.replacements).flatMap((r) => {
    const line = stateLine(r);
    return line === null ? [] : [{ r, line }];
  });
  const many = job.replacements.length > 1;

  return (
    <>
      {asEpisodeList(open) ? (
        <EpisodeList
          jobId={job.id}
          open={open}
          sending={sending}
          problems={problems}
          listProblem={listProblem}
          onDecide={(r, decision) => void decide(r, decision)}
          onDecideAll={(decision) => void decideAll(open, decision)}
        />
      ) : (
        open.map((r, i) => (
          <Fragment key={r.plan_id}>
            <DecisionCard
              r={r}
              many={many}
              layout={cardLayout(open.length, i)}
              sending={sending}
              problem={problems.get(r.position) ?? null}
              onDecide={(decision) => void decide(r, decision)}
              notices={noticesOf(r)}
            />
            <ChangesSection jobId={job.id} r={r} many={many} />
          </Fragment>
        ))
      )}
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
  problem: Problem | null;
  onDecide: (decision: Decision) => void;
  notices: readonly Notice[];
}) {
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
            <PlanFacts r={r} />
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
            {problem.again ? COMPARE_AGAIN : problem.text}
          </p>
        )}
      </section>
      <Notices notices={notices} />
    </>
  );
}

/**
 * Several plans to decide: the episode list is the main area. Its head says how many and has `모두 유지`·`모두 교체`,
 * which decide every row in one request (a failure of it is said once, under the head); a computer keeps the head at
 * the page's top while the list scrolls. Each row
 * has its episode, its change tags and its own two buttons, and opens to that plan's comparison.
 */
function EpisodeList({
  jobId,
  open,
  sending,
  problems,
  listProblem,
  onDecide,
  onDecideAll,
}: {
  jobId: string;
  open: readonly Replacement[];
  sending: boolean;
  problems: ReadonlyMap<number, Problem>;
  listProblem: string | null;
  onDecide: (r: Replacement, decision: Decision) => void;
  onDecideAll: (decision: Decision) => void;
}) {
  return (
    <section
      aria-label="교체를 기다리는 회차"
      className="mt-5 min-w-0 rounded-card border border-hairline bg-surface-1 shadow-(--card-shadow) max-[720px]:mt-3"
    >
      <div className="flex flex-wrap items-center justify-between gap-x-4 gap-y-2 rounded-t-card border-b border-hairline bg-surface-1 px-3.5 py-2.5 min-[721px]:sticky min-[721px]:top-[calc(var(--topbar-h)+8px)] min-[721px]:z-20">
        <b className="text-[14.5px]">교체를 기다리는 회차 {open.length}개</b>
        <div role="group" aria-label="모든 회차 결정" className="flex gap-2 max-[720px]:w-full">
          <Button
            type="button"
            variant="ghost"
            className={cn(btnNeutral, "max-[720px]:flex-1")}
            disabled={sending}
            onClick={() => onDecideAll("keep")}
          >
            모두 유지
          </Button>
          <Button
            type="button"
            variant="ghost"
            className={cn(btnPrimary, "max-[720px]:flex-1")}
            disabled={sending}
            onClick={() => onDecideAll("replace")}
          >
            모두 교체
          </Button>
        </div>
        {listProblem !== null && (
          <p role="alert" className="m-0 w-full text-[13px] font-semibold text-urgent">
            {listProblem}
          </p>
        )}
      </div>
      <ul className="m-0 list-none p-0">
        {open.map((r) => (
          <EpisodeRow
            key={r.plan_id}
            jobId={jobId}
            r={r}
            sending={sending}
            problem={problems.get(r.position) ?? null}
            onDecide={(decision) => onDecide(r, decision)}
          />
        ))}
      </ul>
    </section>
  );
}

/**
 * One episode of the list: the episode, why it was compared again (only then) and its change tags open the row; the
 * opened row shows the version lines, the warnings and limits, and its `변경 사항`.
 */
function EpisodeRow({
  jobId,
  r,
  sending,
  problem,
  onDecide,
}: {
  jobId: string;
  r: Replacement;
  sending: boolean;
  problem: Problem | null;
  onDecide: (decision: Decision) => void;
}) {
  const [opened, setOpened] = useState(false);
  const panelId = useId();
  const again = againNotice(r);
  return (
    <li className="min-w-0 border-t border-hairline-soft first:border-t-0">
      <div className="flex flex-wrap items-center gap-x-4 gap-y-2 px-3.5 py-2.5">
        <button
          type="button"
          aria-expanded={opened}
          aria-controls={panelId}
          onClick={() => setOpened((was) => !was)}
          className="-mx-1.5 flex min-w-0 flex-1 basis-60 items-center gap-2 rounded-card px-1.5 py-1 text-left hover:bg-surface-2 focus-visible:outline-2 focus-visible:outline-focus"
        >
          <ChevronIcon
            className={cn("size-[15px] flex-none text-text-muted transition-transform", opened && "rotate-90")}
          />
          <b className="w-[3rem] flex-none text-[14px]">{planEpisode(r)}</b>
          <span className="flex min-w-0 flex-1 flex-wrap items-center gap-1.5">
            {again?.tags.map((tag) => (
              <Badge key={tag} tone="check" icon={WarningIcon}>
                {tag}
              </Badge>
            ))}
            {rowTags(r).map((tag) => (
              <Tag key={tag}>{tag}</Tag>
            ))}
          </span>
        </button>
        <div role="group" aria-label={`${planEpisode(r)} 결정`} className="flex flex-none gap-2 max-[720px]:w-full">
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
        <p role="alert" className="px-3.5 pb-2.5 text-[13px] font-semibold text-urgent">
          {problem.again ? COMPARE_AGAIN_ROW : problem.text}
        </p>
      )}
      {opened && (
        <div id={panelId} className="flex min-w-0 flex-col gap-2.5 border-t border-hairline-soft px-3.5 pt-3 pb-3.5">
          <PlanFacts r={r} againTags={false} />
          <Notices notices={noticesOf(r)} />
          <ChangesSection jobId={jobId} r={r} many />
        </div>
      )}
    </li>
  );
}

/** The warnings and limits that hold for a plan: its unexpected paths, then its limits. */
function noticesOf(r: Replacement): Notice[] {
  return [...pathWarnings(r.paths), ...limitNotices(r)];
}

function Notices({ notices }: { notices: readonly Notice[] }) {
  if (notices.length === 0) return null;
  return (
    <ul className="m-0 mt-3 flex list-none flex-col gap-2 p-0">
      {notices.map((n) => (
        <NoticeItem key={n.key} notice={n} />
      ))}
    </ul>
  );
}

/**
 * What a person compares a plan by: why it was compared again (only then), the `현재` and `새 자막` version lines, and
 * the two files side by side when that rule holds. The decision card and an opened episode row both show it; the row
 * has the `다시 비교 필요` tags in its head already (`againTags`).
 */
function PlanFacts({ r, againTags = true }: { r: Replacement; againTags?: boolean }) {
  const again = againNotice(r);
  const rows = compareRows(r);
  const shown = shownVersions(r);
  return (
    <>
      {again !== null && (
        <div className="flex flex-col gap-1">
          {againTags && (
            <div className="flex flex-wrap items-center gap-1.5">
              {again.tags.map((tag) => (
                <Badge key={tag} tone="check" icon={WarningIcon}>
                  {tag}
                </Badge>
              ))}
            </div>
          )}
          <p className="text-[13px] leading-snug text-text-secondary [overflow-wrap:anywhere]">{again.reason}</p>
        </div>
      )}
      <dl className="m-0 flex flex-col gap-1.5">
        {shown.current !== null && <VersionLine name="현재" facts={versionFacts(shown.current)} />}
        {shown.new !== null && <VersionLine name="새 자막" facts={versionFacts(shown.new)} />}
      </dl>
      {rows.length > 0 && <Compare rows={rows} />}
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
