import { useEffect, useId, useRef } from "react";
import { Link, useParams } from "react-router-dom";

import { Button } from "@/components/ui/button";
import { when } from "@/lib/time";
import { cn } from "@/lib/utils";

import { EmptyState, usePageTitle } from "../ScreenFrame";
import { btnNeutral } from "../collect/channels/styles";
import { coverOf } from "../library/model";
import { Cover } from "../library/WorkItem";
import { fetchJob, jobPath, type JobDetail, type JobRow } from "./api";
import { CountChip, FailureTag, OriginTags, StateBadge, Tag, originSentence } from "./badges";
import { ended, shownState } from "./format";
import { BackIcon } from "./icons";
import { JobAuth } from "./JobAuth";
import { JobResults } from "./JobResults";
import { JobSteps } from "./JobSteps";
import { UploadResults } from "./UploadResults";
import { KEYS, usePolled } from "./poll";
import { TargetLine } from "./TargetLine";
import { useScreenPrepare, type ScreenPrepare } from "./useScreenPrepare";

/** How often a job that is still going is read while the page is visible. */
const JOB_MS = 2000;

const LOAD_FAILED = "작업을 불러오지 못했어요.";

function BackLink() {
  return (
    <Link
      to="/todo"
      className="inline-flex min-h-9 max-w-full items-center gap-1.5 rounded-md text-[13px] font-semibold text-text-secondary hover:text-text-primary"
    >
      <BackIcon className="size-[15px] flex-none" />할 일
    </Link>
  );
}

/**
 * A subtitle job's page, one for every job: the way back, the head, the steps,
 * the check on the site (the remote screen) while the job has one, the result
 * of each episode, the path and the log, in one column. It reads the job again
 * every two seconds while the page is visible and the job has not ended. Opening
 * the page asks once for the job's server browser screen to be prepared;
 * reading the job again never does.
 */
export function JobDetailScreen() {
  const { jobId = "" } = useParams();
  // One page per job: nothing carries over to another.
  return <JobPage key={jobId} jobId={jobId} />;
}

function JobPage({ jobId }: { jobId: string }) {
  const job = usePolled<JobDetail>(
    KEYS.job(jobId),
    (signal) => fetchJob(jobId, signal),
    (data) => (data !== undefined && ended(data.state) ? null : JOB_MS),
    LOAD_FAILED,
  );
  usePageTitle(job.data?.title ?? "작업");
  const prepare = useScreenPrepare(jobId, job.data);

  if (job.data) return <Page job={job.data} prepare={prepare} />;

  return (
    <section className="mx-auto flex max-w-[880px] flex-col items-start gap-3 pt-4 pb-4">
      <BackLink />
      {job.code === "not_found" ? (
        <div className="w-full pt-2">
          <EmptyState>이 작업을 찾지 못했어요. 지워졌거나 주소가 틀렸을 수 있어요.</EmptyState>
        </div>
      ) : job.error !== null ? (
        <p role="alert" className="text-[13px] font-semibold text-urgent">
          {job.error}
        </p>
      ) : job.slow ? (
        <p className="text-[13px] text-text-muted">작업을 불러오는 중이에요.</p>
      ) : null}
    </section>
  );
}

/** What the time next to the badge says: since when it is so, or when it ended. */
function timeSentence(job: JobRow): string {
  const at = when(job.state_at);
  switch (job.state) {
    case "failed":
      return `${at} 실패`;
    case "partial":
      return `${at} 일부 실패`;
    case "done":
      return job.origin === "upload" ? `${at} 올림` : `${at} 받음`;
    default:
      return `${at}부터`;
  }
}

function Page({ job, prepare }: { job: JobDetail; prepare: ScreenPrepare }) {
  const title = useRef<HTMLHeadingElement>(null);
  const screen = prepare.screen;
  // A phone shows the check box in its first screen: the head shrinks while the job has a screen.
  const compact = screen !== null;

  // Arriving takes focus to the head, once per job.
  useEffect(() => {
    title.current?.focus({ preventScroll: true });
  }, []);

  return (
    <article className="mx-auto max-w-[880px] pb-4">
      <div className="pt-4 max-[720px]:pt-2">
        <BackLink />
      </div>

      <header className="flex items-start gap-4 pt-2.5 max-[720px]:gap-3 max-[720px]:pt-1">
        <Cover
          work={coverOf(job.title)}
          imageUrl={job.work?.cover_url}
          className={cn("h-[88px] w-[59px] max-[720px]:h-[68px] max-[720px]:w-12", compact && "max-[720px]:hidden")}
          letterClass="text-2xl"
        />
        <div className="flex min-w-0 flex-1 flex-col gap-1.5">
          <h1
            ref={title}
            tabIndex={-1}
            className="text-[22px] leading-[1.3] font-bold tracking-[-0.005em] outline-none max-[720px]:text-[16.5px]"
          >
            {job.work !== null ? (
              <Link
                to={`/library/${encodeURIComponent(job.work.id)}`}
                className="rounded-sm underline-offset-4 hover:underline focus-visible:outline-2 focus-visible:outline-focus"
              >
                {job.title}
              </Link>
            ) : (
              job.title
            )}
          </h1>
          <TargetLine episodes={job.episodes} creator={job.creator} className="text-[14px] max-[720px]:text-[13px]">
            {job.season !== null && <Tag>시즌 {job.season}</Tag>}
            <OriginTags job={job} />
            {job.source !== null && <span className="min-w-0 text-xs text-text-muted [overflow-wrap:anywhere]">{job.source}</span>}
          </TargetLine>
          {(originSentence(job) !== null || job.revision_of !== null || job.revises_attributed) && (
            <p className="flex flex-wrap items-center gap-x-2.5 gap-y-1 text-[13px] text-text-secondary">
              {originSentence(job) !== null && <span>{originSentence(job)}</span>}
              {job.revision_of !== null &&
                (job.revises_job !== null ? (
                  <Link
                    to={jobPath(job.revises_job)}
                    className="rounded-sm underline underline-offset-2 focus-visible:outline-2 focus-visible:outline-focus"
                  >
                    수정본: 이전에 받은 작업 보기
                  </Link>
                ) : (
                  <span>이전에 받은 자막의 수정본이에요. 지금 자막은 바꾸지 않아요.</span>
                ))}
              {job.revises_attributed && <span>제작자를 붙인 자막의 수정본이에요. 지금 자막은 바꾸지 않아요.</span>}
            </p>
          )}
          <div className="mt-1 flex flex-wrap items-center gap-x-2.5 gap-y-1.5">
            <StateBadge shown={shownState(job)} withDone />
            <span className="text-xs text-text-muted">{timeSentence(job)}</span>
            {job.work !== null && (
              <Button asChild variant="ghost" className={cn(btnNeutral, "ml-auto", compact && "max-[720px]:hidden")}>
                <Link to={`/library/${encodeURIComponent(job.work.id)}`}>작품 보기</Link>
              </Button>
            )}
          </div>
          {job.note !== null && job.note !== "" && (
            <p className={cn("text-[13.5px] leading-relaxed text-text-secondary max-[720px]:text-[13px]", compact && "max-[720px]:line-clamp-2")}>
              {job.failure !== null && <FailureTag failure={job.failure} />}
              {job.note}
            </p>
          )}
        </div>
      </header>

      <div className={cn("mt-5", compact && "max-[720px]:mt-3")}>
        <JobSteps steps={job.steps} />
      </div>

      {screen !== null && <JobAuth jobId={job.id} screen={screen} prepare={prepare} />}

      {job.origin === "upload" ? (
        <Part title="올린 파일">
          <UploadResults files={job.items.flatMap((item) => item.files)} dropped={job.dropped} />
        </Part>
      ) : (
        <Part title="회차별 결과">
          <JobResults items={job.items} />
        </Part>
      )}

      <Part title="경로">
        <dl className="m-0 grid grid-cols-[max-content_minmax(0,1fr)] gap-x-3.5 gap-y-1 text-[13px] max-[720px]:grid-cols-1 max-[720px]:gap-y-0">
          <dt className="font-semibold text-text-muted">수신 영역</dt>
          <dd className="m-0 min-w-0 leading-snug text-text-secondary [overflow-wrap:anywhere]">{job.receive_dir}</dd>
        </dl>
      </Part>

      <Part title="기록" count={`${job.log.length}건`}>
        {job.log.length === 0 ? (
          <p className="text-[13px] text-text-muted">아직 남은 기록이 없어요.</p>
        ) : (
          <ol className="m-0 list-none rounded-card border border-hairline-soft bg-surface-1 p-0 px-3.5 shadow-(--card-shadow)">
            {job.log.map((entry, i) => (
              <li
                key={`${entry.at}:${i}`}
                className="grid grid-cols-[8.5rem_minmax(0,1fr)] gap-x-3 border-t border-hairline-soft py-2 text-[13px] leading-snug first:border-t-0 max-[720px]:grid-cols-1 max-[720px]:gap-y-0.5"
              >
                <time className="text-xs whitespace-nowrap text-text-muted">{when(entry.at)}</time>
                <span className="min-w-0 [overflow-wrap:anywhere]">
                  {entry.message}
                  {entry.detail !== null && entry.detail !== "" && (
                    <span className="ml-2 text-xs text-text-muted">{entry.detail}</span>
                  )}
                </span>
              </li>
            ))}
          </ol>
        )}
      </Part>
    </article>
  );
}

function Part({ title, count, children }: { title: string; count?: string; children: React.ReactNode }) {
  const headingId = useId();
  return (
    <section aria-labelledby={headingId} className="mt-8 max-[720px]:mt-6">
      <div className="mb-3 flex flex-wrap items-center gap-x-2.5 gap-y-1 max-[720px]:mb-2">
        <h2 id={headingId} className="text-[17px] font-bold">
          {title}
        </h2>
        {count !== undefined && <CountChip>{count}</CountChip>}
      </div>
      {children}
    </section>
  );
}
