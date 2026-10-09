import { Fragment, useEffect, useId, useRef } from "react";
import { Link, useParams } from "react-router-dom";

import { Button } from "@/components/ui/button";
import { when } from "@/lib/time";
import { cn } from "@/lib/utils";

import { EmptyState, usePageTitle } from "../ScreenFrame";
import { btnNeutral } from "../collect/channels/styles";
import { coverOf } from "../library/model";
import { Cover } from "../library/WorkItem";
import { fetchJob, jobPath, type JobDetail, type JobRow, type JobScreen } from "./api";
import { CountChip, FailureTag, OriginTags, StateBadge, Tag, originSentence } from "./badges";
import { ended, episodeName, shownState } from "./format";
import { FindFinish } from "./FindFinish";
import { BackIcon } from "./icons";
import { JobAuth } from "./JobAuth";
import { JobResults, PackageResults } from "./JobResults";
import { PlacementConfirm } from "./PlacementConfirm";
import { RelocationConfirm, RelocationResults } from "./RelocationTable";
import { ReplacementDecisions, ReplacementPaths } from "./Replacement";
import { cardLayout, openOnes } from "./replacementView";
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
 * A subtitle job's page, one for every job: the way back, the head, the
 * decision on replacing an episode's subtitle (the card and its notes) while the
 * job has one, the steps, the check on the site (the remote screen) while the job has one, the result
 * of each episode, the path and the log, in one column. A find job (직접 찾기)
 * puts its remote screen first, with `받기 끝내기` under it, and lists the files
 * it received instead of episodes. It reads the job again
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
      return job.origin === "upload"
        ? `${at} 올림`
        : job.origin === "find"
          ? `${at} 끝냄`
          : job.origin === "relocate"
            ? `${at} 옮김`
            : `${at} 받음`;
    default:
      return `${at}부터`;
  }
}

/** What a find job that has no screen yet shows while the worker opens its post. */
const OPENING: JobScreen = { state: "preparing", run: null, bound: null, note: null, popup: false, limits: null };

function Page({ job, prepare }: { job: JobDetail; prepare: ScreenPrepare }) {
  const title = useRef<HTMLHeadingElement>(null);
  const find = job.origin === "find" && job.receiving;
  const screen =
    prepare.screen ?? (find && (job.state === "pending" || job.state === "running") ? OPENING : null);
  // A phone shows the check box in its first screen: the head shrinks while the job has a screen.
  const compact = screen !== null;
  // The phone's decision buttons are fixed above the bottom menu: the page leaves room so they never cover its end.
  const fixedBar = cardLayout(openOnes(job.replacements).length, 0).fixed;

  // Arriving takes focus to the head, once per job.
  useEffect(() => {
    title.current?.focus({ preventScroll: true });
  }, []);

  return (
    <article className={cn("mx-auto max-w-[880px] pb-4", fixedBar && "max-[720px]:pb-20")}>
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
          <TargetLine segments={job.episode_segments} creator={job.creator} className="text-[14px] max-[720px]:text-[13px]">
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

      <ReplacementDecisions job={job} />

      {job.confirm !== null &&
        (job.confirm.scope === "relocate" ? (
          <RelocationConfirm job={job} confirm={job.confirm} />
        ) : (
          <PlacementConfirm job={job} confirm={job.confirm} />
        ))}

      {find &&
        (screen !== null ? (
          <JobAuth jobId={job.id} screen={screen} prepare={prepare} find>
            <FindFinish job={job} />
          </JobAuth>
        ) : (
          <div className="mt-6 max-[720px]:mt-4">
            <FindFinish job={job} />
          </div>
        ))}

      <div className={cn("mt-5", compact && "max-[720px]:mt-3")}>
        <JobSteps steps={job.steps} />
      </div>

      {!find && screen !== null && <JobAuth jobId={job.id} screen={screen} prepare={prepare} />}

      {job.origin === "upload" || job.origin === "find" ? (
        <>
          <Part title={job.origin === "find" ? "받은 파일" : "올린 파일"}>
            <UploadResults
              files={job.items.flatMap((item) => item.files)}
              dropped={job.dropped}
              empty={job.origin === "find" ? "받은 파일이 없어요." : undefined}
            />
          </Part>
          {job.confirm === null && job.placements.length > 0 && (
            <Part title="회차별 결과">
              <PackageResults placements={job.placements} replacements={job.replacements} />
            </Part>
          )}
        </>
      ) : job.origin === "relocate" ? (
        job.confirm === null && (
          <Part title="옮긴 적용본">
            <RelocationResults job={job} />
          </Part>
        )
      ) : (
        <Part title="회차별 결과">
          <JobResults items={job.items} placements={job.placements} replacements={job.replacements} />
        </Part>
      )}

      <Part title="경로">
        <JobPaths job={job} />
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

/** One file's paths, or the receive area's: a term and its path per line. */
function PathList({ rows }: { rows: readonly (readonly [string, string | null])[] }) {
  const shown = rows.filter((row): row is readonly [string, string] => row[1] !== null);
  return (
    <dl className="m-0 grid grid-cols-[max-content_minmax(0,1fr)] gap-x-3.5 gap-y-1 text-[13px] max-[720px]:grid-cols-1 max-[720px]:gap-y-0">
      {shown.map(([term, path]) => (
        <Fragment key={term}>
          <dt className="font-semibold text-text-muted max-[720px]:mt-1 max-[720px]:first:mt-0">{term}</dt>
          <dd className="m-0 min-w-0 leading-snug text-text-secondary [overflow-wrap:anywhere]">{path}</dd>
        </Fragment>
      ))}
    </dl>
  );
}

/**
 * `경로`: for each file placed, its video, applied copy and stored file, apart; then the job's folder in the receive
 * area, while something is in it.
 */
function JobPaths({ job }: { job: JobDetail }) {
  const placed = job.placements.filter((p) => p.video !== null || p.applied !== null || p.stored !== null);
  const received = job.items.some((item) => item.files.some((file) => file.path !== null));
  return (
    <div className="flex flex-col gap-3">
      {placed.map((p) => (
        <div key={p.position} className="flex min-w-0 flex-col gap-1">
          <p className="text-[13px] leading-snug font-semibold [overflow-wrap:anywhere]">
            {p.episode !== null && <span className="mr-2">{episodeName(String(p.episode))}</span>}
            <span className="font-normal text-text-secondary">{p.name}</span>
          </p>
          <PathList
            rows={[
              ["영상", p.video],
              ["적용본", p.applied],
              ["보관본", p.stored],
            ]}
          />
        </div>
      ))}
      <ReplacementPaths replacements={job.replacements} />
      {(received || placed.length === 0) && <PathList rows={[["수신 영역", job.receive_dir]]} />}
    </div>
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
