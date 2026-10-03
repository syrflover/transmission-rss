import { useEffect, useId, useRef, useState } from "react";
import { Link } from "react-router-dom";

import { Button } from "@/components/ui/button";
import { ApiError } from "@/lib/api";
import { when } from "@/lib/time";
import { cn } from "@/lib/utils";

import { btnNeutral } from "../collect/channels/styles";
import { coverOf } from "../library/model";
import { Cover } from "../library/WorkItem";
import { fetchDoneJobs, jobPath, type DonePage, type JobGroups, type JobRow } from "./api";
import { CountChip, FailureTag, OriginTags, StateBadge, originSentence } from "./badges";
import { episodeList, shownState, uploadKept } from "./format";
import { ChevronIcon } from "./icons";
import { TargetLine } from "./TargetLine";

/** The next page of done jobs is read this far before the end of the list comes into view. */
const READ_AHEAD = "0px 0px 600px 0px";

/** The time line under a row's badge. */
function timeLine(job: JobRow): string {
  switch (job.state) {
    case "waiting":
    case "held":
    case "pending":
      return `${when(job.state_at)}부터`;
    case "running":
      return job.progress.total > 1 ? `${job.progress.done}/${job.progress.total}개 받음` : when(job.state_at);
    default:
      return when(job.state_at);
  }
}

/** An upload job's sentence: what it kept, and that some files were left out. */
function uploaded(job: JobRow): string {
  const kept = job.upload !== null ? uploadKept(job.upload) : null;
  const dropped = job.upload?.dropped ?? 0;
  return `올린 파일: ${kept ?? "없음"}.${dropped > 0 ? ` 뺀 파일 ${dropped}개의 이름과 이유는 작업 상세에서 볼 수 있어요.` : ""}`;
}

/** The one sentence of the expanded row: the job's own note, else a plain one for its state. */
function describe(job: JobRow): string {
  if (job.note !== null && job.note !== "") return job.note;
  switch (job.state) {
    case "pending":
      return "worker가 곧 받기 시작해요.";
    case "running":
      return job.stage === "open"
        ? "게시물을 열어 받을 파일을 찾고 있어요."
        : "자막 파일을 받고 있어요.";
    case "waiting":
      return job.wait === "auth"
        ? "사이트가 사람의 확인을 기다려요. 위의 인증 필요 카드나 작업 상세에서 이어가요."
        : "자막이 아직 출처에 연결되지 않아서 다시 확인하기를 기다려요.";
    case "held":
      return "다시 시작하는 사이 받던 파일을 확인하지 못해서 멈췄어요. 같은 파일을 두 번 받지 않으려고 보류했어요.";
    case "failed":
      return "자막을 받지 못했어요. 작업 상세에서 회차별 이유를 볼 수 있어요.";
    case "partial":
      return "일부 회차는 받았고 일부는 받지 못했어요. 작업 상세에서 회차별 이유를 볼 수 있어요.";
    case "done":
      return job.origin === "upload" ? uploaded(job) : "자막을 모두 받았어요.";
  }
}

/** One subsection of `자막 작업`: a heading with its count, then the rows. Hidden when empty. */
export function JobSection({
  title,
  count,
  jobs,
  children,
}: {
  title: string;
  count: number;
  jobs: readonly JobRow[];
  /** After the rows: the sentinel and the status of the loading list. */
  children?: React.ReactNode;
}) {
  const headingId = useId();
  if (jobs.length === 0) return null;
  return (
    <section aria-labelledby={headingId} className="mt-6 first:mt-0">
      <div className="mb-2.5 flex flex-wrap items-center gap-x-2.5 gap-y-1">
        <h3 id={headingId} className="text-[15px] font-bold">
          {title}
        </h3>
        <CountChip>{count}</CountChip>
      </div>
      <ul className="m-0 flex list-none flex-col gap-1.5 p-0">
        {jobs.map((job) => (
          <JobRowItem key={job.id} job={job} />
        ))}
      </ul>
      {children}
    </section>
  );
}

function JobRowItem({ job }: { job: JobRow }) {
  const [open, setOpen] = useState(false);
  const detailId = useId();
  const episodes = job.episodes;
  const received = `${job.progress.done}/${job.progress.total}개`;

  return (
    <li className="min-w-0 rounded-card bg-surface-2 text-text-primary dark:bg-surface-1">
      <button
        type="button"
        aria-expanded={open}
        aria-controls={detailId}
        onClick={() => setOpen((was) => !was)}
        className={cn(
          "grid w-full grid-cols-[48px_minmax(0,1fr)_auto_16px] items-center gap-x-4 rounded-card px-3 py-2.5 text-left hover:bg-surface-1 dark:hover:bg-surface-3",
          open && "bg-surface-1 dark:bg-surface-3",
          "max-[720px]:grid-cols-[48px_minmax(0,1fr)_16px] max-[720px]:items-start max-[720px]:gap-x-3",
        )}
      >
        <Cover
          work={coverOf(job.title)}
          imageUrl={job.work?.cover_url}
          className="h-[72px] w-12 rounded-md max-[720px]:row-span-2"
          letterClass="text-xl"
        />
        <span className="flex min-w-0 flex-col gap-1">
          <span className="line-clamp-2 min-w-0 text-[14.5px] leading-snug font-semibold">{job.title}</span>
          <TargetLine episodes={episodes} creator={job.creator}>
            <OriginTags job={job} />
          </TargetLine>
        </span>
        <span className="flex flex-col items-end gap-1 text-right max-[720px]:col-start-2 max-[720px]:row-start-2 max-[720px]:mt-1 max-[720px]:flex-row max-[720px]:flex-wrap max-[720px]:items-center max-[720px]:justify-start max-[720px]:gap-x-2 max-[720px]:text-left">
          <StateBadge shown={shownState(job)} />
          <span className="text-xs whitespace-nowrap text-text-muted">{timeLine(job)}</span>
        </span>
        <ChevronIcon
          className={cn(
            "size-4 text-text-muted transition-transform max-[720px]:col-start-3 max-[720px]:row-span-2 max-[720px]:row-start-1 max-[720px]:self-center",
            open && "rotate-90",
          )}
        />
      </button>
      {open && (
        <div
          id={detailId}
          className="flex flex-col gap-3 px-3 pt-1 pb-3.5 pl-[76px] max-[720px]:pl-3"
        >
          <p className="text-[13px] leading-relaxed text-text-secondary">
            {job.failure !== null && <FailureTag failure={job.failure} />}
            {describe(job)}
          </p>
          <dl className="m-0 grid grid-cols-[max-content_minmax(0,1fr)] gap-x-3.5 gap-y-1 text-[12.5px] ">
            {originSentence(job) !== null && <Field name="만든 곳">{originSentence(job)}</Field>}
            {job.revision_of !== null && (
              <Field name="수정본">
                {job.revises_job !== null ? (
                  <Link to={jobPath(job.revises_job)} className="underline underline-offset-2">
                    이전에 받은 작업
                  </Link>
                ) : (
                  "이전에 받은 자막의 수정본"
                )}
              </Field>
            )}
            {job.source !== null && <Field name="출처">{job.source}</Field>}
            {episodes.length > 0 && <Field name="회차">{episodeList(episodes)}</Field>}
            {job.origin === "upload" ? (
              <>
                {job.upload !== null && uploadKept(job.upload) !== null && <Field name="올린 파일">{uploadKept(job.upload)}</Field>}
                {job.upload !== null && job.upload.dropped > 0 && <Field name="뺀 파일">{job.upload.dropped}개</Field>}
              </>
            ) : (
              <>
                <Field name="받은 회차">{received}</Field>
                {job.progress.failed > 0 && <Field name="실패">{job.progress.failed}개</Field>}
              </>
            )}
          </dl>
          <div>
            <Button asChild variant="ghost" className={btnNeutral}>
              <Link to={jobPath(job.id)}>자세히</Link>
            </Button>
          </div>
        </div>
      )}
    </li>
  );
}

function Field({ name, children }: { name: string; children: React.ReactNode }) {
  return (
    <>
      <dt className="font-semibold text-text-muted">{name}</dt>
      <dd className="m-0 min-w-0 text-text-secondary [overflow-wrap:anywhere]">{children}</dd>
    </>
  );
}

/** The four subsections of `자막 작업`, in order; `최근 완료` continues by scrolling. */
export function JobGroupsView({ groups }: { groups: JobGroups }) {
  return (
    <>
      <JobSection title="실패" count={groups.failed.length} jobs={groups.failed} />
      <JobSection title="대기 중" count={groups.waiting.length} jobs={groups.waiting} />
      <JobSection title="진행 중" count={groups.running.length} jobs={groups.running} />
      <DoneSection first={groups.done} />
    </>
  );
}

/**
 * `최근 완료`: the first five come with the screen's own read; the rest are read
 * page by page as the end of the list nears, never behind a button. Rows that are
 * shown keep their place: a newly done job joins at the top, and the pages read
 * so far stay below the first five.
 */
function DoneSection({ first }: { first: DonePage }) {
  const [extra, setExtra] = useState<{ items: JobRow[]; next: string | null } | null>(null);
  const [loading, setLoading] = useState(false);
  const [failed, setFailed] = useState<string | null>(null);
  const sentinel = useRef<HTMLDivElement>(null);
  const alive = useRef(true);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);

  const shownIds = new Set(first.items.map((job) => job.id));
  const rest = extra === null ? [] : extra.items.filter((job) => !shownIds.has(job.id));
  const jobs = [...first.items, ...rest];
  // The cursor to go on from: the last page read, else the first page's.
  const cursor = extra === null ? first.next : extra.next;
  const watching = cursor !== null && !loading && failed === null;

  const more = () => {
    if (cursor === null || loading) return;
    setLoading(true);
    fetchDoneJobs(cursor).then(
      (page) => {
        if (!alive.current) return;
        setExtra((was) => ({ items: [...(was?.items ?? []), ...page.items], next: page.next }));
        setLoading(false);
      },
      (e: unknown) => {
        if (!alive.current) return;
        setFailed(e instanceof ApiError ? e.message : "완료한 작업을 더 불러오지 못했어요.");
        setLoading(false);
      },
    );
  };
  const moreRef = useRef(more);
  moreRef.current = more;

  // Watching starts again after each page: a strip that is still within reach reports at once.
  useEffect(() => {
    const element = sentinel.current;
    if (!watching || element === null) return;
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((entry) => entry.isIntersecting)) moreRef.current();
      },
      { rootMargin: READ_AHEAD },
    );
    observer.observe(element);
    return () => observer.disconnect();
  }, [watching, jobs.length]);

  return (
    <JobSection title="최근 완료" count={first.total} jobs={jobs}>
      <div ref={sentinel} aria-hidden="true" className="h-px" />
      {loading && <p className="pt-3 text-[13px] text-text-muted">완료한 작업을 더 불러오는 중이에요.</p>}
      {failed !== null && (
        <div className="flex flex-wrap items-center gap-x-3 gap-y-2 pt-3">
          <p role="alert" className="text-[13px] font-semibold text-urgent">
            {failed}
          </p>
          <Button
            type="button"
            variant="ghost"
            className={btnNeutral}
            onClick={() => {
              setFailed(null);
            }}
          >
            다시 시도
          </Button>
        </div>
      )}
    </JobSection>
  );
}
