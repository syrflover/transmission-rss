import { useState } from "react";
import { Link } from "react-router-dom";

import { cn } from "@/lib/utils";
import { dateTime } from "@/lib/time";

import { ApiError } from "@/lib/api";

import { applyStored, type EpisodeFailure, type EpisodeRevision, type FailureFile, type StoredSubtitle, type WorkEpisode, type WorkFile, type WorkSeason } from "../api";
import { jobPath } from "../../todo/api";
import { CheckIcon, ChevronIcon, MinusIcon } from "../icons";
import { airDay, baseName, episodeLabel, inOrder, ORDERS, rowId, type EpisodeOrder } from "./model";
import { EmptyState } from "../../ScreenFrame";
import { inputClass } from "../../collect/channels/styles";
import { RetryActions } from "../../collect/history/RetryActions";
import { retryStatus, useItemRetry } from "../../collect/history/useRetry";
import type { Candidate } from "../api";
import { candidatesOf } from "./candidates";
import { candidateNote, EpisodePicks, type EpisodeCandidateSource } from "./EpisodeCandidates";
import { SubtitleFiles } from "./SubtitleCreators";

/**
 * One kind of file of an episode: `영상 ✓` or `자막 −`. The label is the same
 * size and weight whether or not the file is there; the mark and the colour say
 * which, and the hidden text says it to assistive technology (`보유`/`없음`, or
 * `기록 있음`/`기록 없음` for a work whose folder is gone, which holds nothing
 * now).
 */
function Hold({ label, on, missing }: { label: string; on: boolean; missing: boolean }) {
  const Mark = on ? CheckIcon : MinusIcon;
  const said = missing ? (on ? "기록 있음" : "기록 없음") : on ? "보유" : "없음";
  return (
    <span className={cn("inline-flex items-center gap-1 text-[13px] font-semibold", on ? "text-text-secondary" : "text-text-muted")}>
      {label}
      <Mark className={cn("size-[1em] flex-none", on && !missing && "text-ok")} />
      <span className="sr-only">{said}</span>
    </span>
  );
}

function Cell({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="min-w-0">
      <dt className="text-[12px] font-semibold text-text-muted">{label}</dt>
      <dd className="m-0 mt-1 flex flex-col gap-1 text-[12.5px] leading-snug text-text-primary">{children}</dd>
    </div>
  );
}

function Files({ files }: { files: WorkFile[] }) {
  if (files.length === 0) return <span className="text-text-muted">없음</span>;
  return files.map((file) => (
    <span key={file.path} title={file.path} className="font-mono text-[12px] break-all">
      {baseName(file.path)}
    </span>
  ));
}

/** What became of each file of a failed replacement, as the expanded row says it. */
const FAILURE_FILE: Record<FailureFile["role"], string> = { old: "이전 영상", new: "새 영상" };
const FAILURE_STATE: Record<FailureFile["state"], string> = {
  kept: "그대로 있음",
  removed: "지움",
  received_name: "받은 이름 그대로",
  missing: "없어짐",
  not_received: "받지 못함",
};

/**
 * `다시 받기` of a revision whose download stopped: the same command as a
 * history row's, for the revision's history item. Once it ends the work is
 * read again, so the row shows the replacement going on (the failure gone) or
 * still failed.
 */
function FailureRetry({ failure, onRetried }: { failure: EpisodeFailure; onRetried: () => Promise<void> }) {
  const { phase, submit, resend, recheck } = useItemRetry(failure.history_item_id, failure.command, onRetried);
  const status = retryStatus(phase);
  return (
    <div className="mt-0.5 flex flex-col gap-1.5">
      <p
        role={phase.kind === "idle" ? undefined : "status"}
        className={cn("text-xs leading-[1.45] [overflow-wrap:anywhere] empty:hidden", status?.urgent ? "text-urgent" : "text-text-secondary")}
      >
        {status?.text}
      </p>
      <RetryActions phase={phase} onSubmit={submit} onResend={resend} onRecheck={recheck} />
    </div>
  );
}

/**
 * A replacement of the episode's video that failed: why, and the two files with
 * what became of each. Neither file is taken as the episode's video here. One
 * whose download stopped offers `다시 받기`, or says why it does not.
 */
function Failure({ failure, onRetried }: { failure: EpisodeFailure; onRetried: () => Promise<void> }) {
  return (
    <div className="col-span-full min-w-0">
      <dt className="text-[12px] font-semibold text-urgent">받기 실패 · {dateTime(failure.at)}</dt>
      <dd className="m-0 mt-1 flex flex-col gap-1.5 text-[12.5px] leading-snug text-text-primary">
        <span className="[overflow-wrap:anywhere]">{failure.reason}</span>
        <ul className="m-0 flex list-none flex-col gap-1 p-0">
          {failure.files.map((file) => (
            <li key={file.role} className="flex flex-wrap items-baseline gap-x-2 gap-y-0.5">
              <span className="font-semibold text-text-secondary">{FAILURE_FILE[file.role]}</span>
              <span className="text-text-muted">{FAILURE_STATE[file.state]}</span>
              {file.path !== null && (
                <span title={file.path} className="min-w-0 font-mono text-[12px] break-all">
                  {baseName(file.path)}
                </span>
              )}
            </li>
          ))}
        </ul>
        {failure.can_retry ? (
          <FailureRetry failure={failure} onRetried={onRetried} />
        ) : (
          failure.retry_blocked !== null && <span className="text-xs text-text-muted">{failure.retry_blocked}</span>
        )}
      </dd>
    </div>
  );
}

/**
 * The quiet version line of a video replaced by a higher revision: `v1 › v2`
 * and when, dimmed, with no warning words or colour.
 */
function VersionLine({ revision }: { revision: EpisodeRevision }) {
  return (
    <span className="flex flex-wrap items-center gap-x-2 gap-y-0.5 text-[12px] text-text-muted">
      <span className="rounded-full border border-hairline px-1.5 font-mono text-[11.5px] leading-[1.5] text-text-secondary">
        {revision.from === null ? revision.to : `${revision.from} › ${revision.to}`}
      </span>
      <span>{dateTime(revision.replaced_at)} 수정본으로 교체</span>
    </span>
  );
}

const STORED_FORMAT: Record<StoredSubtitle["format"], string> = { ass: "ASS", srt: "SRT", smi: "SMI", other: "그 밖의 형식" };

/** A stored subtitle shown on an episode with none: applying it is the job's that stored it. */
function StoredLine({
  workId,
  stored,
  hasVideo,
  onApplied,
}: {
  workId: string;
  stored: StoredSubtitle;
  hasVideo: boolean;
  onApplied: () => Promise<void>;
}) {
  const [phase, setPhase] = useState<{ kind: "idle" } | { kind: "sending" } | { kind: "sent"; job: string } | { kind: "error"; text: string }>({
    kind: "idle",
  });
  const apply = async () => {
    setPhase({ kind: "sending" });
    try {
      const { job_id } = await applyStored(workId, stored.id);
      setPhase({ kind: "sent", job: job_id });
      await onApplied();
    } catch (e) {
      setPhase({ kind: "error", text: e instanceof ApiError ? e.message : "적용을 요청하지 못했어요." });
    }
  };
  const facts = [stored.creator ?? "제작자 알 수 없음", STORED_FORMAT[stored.format], `${dateTime(stored.stored_at)} 받음`];
  return (
    <span className="flex flex-col gap-1">
      <span title={stored.name} className="font-mono text-[12px] break-all">
        {stored.name}
      </span>
      <span className="text-[12px] text-text-muted">{facts.join(" · ")}</span>
      {phase.kind === "sent" ? (
        <span role="status" className="text-xs text-text-secondary">
          적용을 맡겼어요.{" "}
          <Link to={jobPath(phase.job)} className="rounded-sm underline underline-offset-2 focus-visible:outline-2 focus-visible:outline-focus">
            작업 보기
          </Link>
        </span>
      ) : !stored.can_apply ? (
        <span className="text-xs text-text-muted">
          {stored.format === "other" ? "자동으로 적용하지 않는 형식이에요." : "받은 작업의 기록이 없어 여기서 적용할 수 없어요."}
        </span>
      ) : !hasVideo ? (
        <span className="text-xs text-text-muted">영상이 들어오면 적용할 수 있어요.</span>
      ) : (
        <span className="flex flex-wrap items-center gap-2">
          <button
            type="button"
            onClick={() => void apply()}
            disabled={phase.kind === "sending"}
            className="inline-flex min-h-7 items-center rounded-md border border-hairline px-2.5 text-[12.5px] font-semibold text-text-primary hover:bg-surface-2 focus-visible:outline-2 focus-visible:outline-focus disabled:opacity-60 max-[720px]:min-h-9"
          >
            {phase.kind === "sending" ? "요청하는 중…" : "적용"}
          </button>
          {phase.kind === "error" && (
            <span role="alert" className="text-xs text-urgent">
              {phase.text}
            </span>
          )}
        </span>
      )}
    </span>
  );
}

/**
 * A received subtitle waiting for the episode's video (`영상 대기`): why, and
 * the file. It is not beside a video, so it is not the episode's subtitle, and
 * the job applies it by itself once the video comes.
 */
function AwaitingLine({ stored }: { stored: StoredSubtitle }) {
  const facts = [stored.creator ?? "제작자 알 수 없음", STORED_FORMAT[stored.format], `${dateTime(stored.stored_at)} 받음`];
  return (
    <span className="flex flex-col gap-1">
      <span title={stored.name} className="font-mono text-[12px] break-all">
        {stored.name}
      </span>
      <span className="text-[12px] text-text-muted">{facts.join(" · ")}</span>
    </span>
  );
}

/** The time a file was added, or `미상` when it was there before the app first looked. */
const addedAt = (file: WorkFile) => (file.added_at === null ? "미상" : dateTime(file.added_at));

/**
 * The episode's stored subtitles: those waiting for its video, those waiting for the user to approve a replacement
 * (`교체 승인`), and the others (`보관본 있음`).
 */
function storedOf(episode: WorkEpisode) {
  return {
    awaiting: episode.stored.filter((stored) => stored.awaiting_video),
    approval: episode.stored.filter((stored) => !stored.awaiting_video && stored.approval_job !== null),
    others: episode.stored.filter((stored) => !stored.awaiting_video && stored.approval_job === null),
  };
}

function Details({
  id,
  workId,
  season,
  animeNo,
  episode,
  picks,
  source,
  onRetried,
  onCreatorChanged,
}: {
  id: string;
  workId: string;
  season: number;
  /** The Anissia anime the season is linked to; the creators of a subtitle file are its creators. */
  animeNo: number | null;
  episode: WorkEpisode;
  /** The subtitle candidates of the episode, none without a source. */
  picks: readonly Candidate[];
  source: EpisodeCandidateSource | undefined;
  onRetried: () => Promise<void>;
  onCreatorChanged: () => void;
}) {
  const { awaiting, approval, others } = storedOf(episode);
  return (
    <dl id={id} className="m-0 grid grid-cols-[repeat(auto-fit,minmax(170px,1fr))] gap-x-4 gap-y-3 px-4 pt-1 pb-4 max-[720px]:px-3">
      {episode.failure !== null && <Failure failure={episode.failure} onRetried={onRetried} />}
      <Cell label="영상 파일">
        <Files files={episode.video} />
      </Cell>
      <Cell label="자막 파일">
        <SubtitleFiles
          workId={workId}
          season={season}
          animeNo={animeNo}
          files={episode.subtitle}
          onChanged={onCreatorChanged}
        />
      </Cell>
      <Cell label="추가 시각">
        {episode.video.map((file) => (
          <span key={file.path}>
            영상 · {addedAt(file)}
          </span>
        ))}
        {episode.subtitle.map((file) => (
          <span key={file.path}>
            자막 · {addedAt(file)}
          </span>
        ))}
      </Cell>
      {awaiting.length > 0 && (
        <Cell label="영상 대기">
          <span className="text-text-secondary">
            {episode.video.length === 0 ? "영상이 아직 없어 영상이 들어오면 적용해요." : "영상이 들어와 곧 적용해요."}
          </span>
          {awaiting.map((stored) => (
            <AwaitingLine key={stored.id} stored={stored} />
          ))}
        </Cell>
      )}
      {approval.length > 0 && (
        <Cell label="교체 승인">
          <span className="text-text-secondary">현재 자막을 이 자막으로 바꿀지 작업에서 비교해 정해요.</span>
          {approval.map((stored) => (
            <span key={stored.id} className="flex flex-col gap-1">
              <AwaitingLine stored={stored} />
              <Link
                to={jobPath(stored.approval_job ?? "")}
                className="inline-flex min-h-6 items-center self-start rounded-sm text-xs font-semibold underline underline-offset-2 focus-visible:outline-2 focus-visible:outline-focus max-[720px]:min-h-9"
              >
                작업 보기
              </Link>
            </span>
          ))}
        </Cell>
      )}
      {episode.subtitle.length === 0 && others.length > 0 && (
        <Cell label="보관본">
          {others.map((stored) => (
            <StoredLine key={stored.id} workId={workId} stored={stored} hasVideo={episode.video.length > 0} onApplied={onRetried} />
          ))}
        </Cell>
      )}
      {source && picks.length > 0 && <EpisodePicks candidates={picks} season={season} source={source} />}
    </dl>
  );
}

function Row({
  workId,
  season,
  animeNo,
  episode,
  missing,
  picks,
  source,
  open,
  onToggle,
  onRetried,
  onCreatorChanged,
}: {
  workId: string;
  season: number;
  animeNo: number | null;
  episode: WorkEpisode;
  missing: boolean;
  picks: readonly Candidate[];
  source: EpisodeCandidateSource | undefined;
  open: boolean;
  onToggle: () => void;
  onRetried: () => Promise<void>;
  onCreatorChanged: () => void;
}) {
  const id = rowId(season, episode.episode);
  const detailId = `${id}-files`;
  const { awaiting, approval, others } = storedOf(episode);
  return (
    <li className="border-t border-hairline-soft first:border-t-0">
      <button
        type="button"
        id={id}
        aria-expanded={open}
        aria-controls={detailId}
        onClick={onToggle}
        className={cn(
          "grid min-h-12 w-full scroll-mt-[calc(var(--topbar-h)+16px)] grid-cols-[4.2em_minmax(0,1fr)_auto_16px] items-center gap-x-3 px-4 py-2.5 text-left hover:bg-surface-2 focus:bg-surface-2 max-[720px]:px-3 dark:hover:bg-surface-2 dark:focus:bg-surface-2",
          open && "bg-[color-mix(in_srgb,var(--focus-ring)_5%,var(--surface-1))]",
        )}
      >
        <span className="text-sm font-bold">{episodeLabel(episode.episode)}</span>
        <span className="flex min-w-0 flex-col gap-1">
          <span className="flex flex-wrap items-center gap-x-4 gap-y-1">
            <Hold label="영상" on={episode.video.length > 0} missing={missing} />
            {/* A subtitle received for a video still to come is held: `자막 ✓` with `영상 대기`. */}
            <Hold label="자막" on={episode.subtitle.length > 0 || awaiting.length > 0} missing={missing} />
            {awaiting.length > 0 && (
              <span className="inline-flex items-center rounded-full border border-hairline px-2 py-px text-xs font-bold whitespace-nowrap text-text-secondary">
                영상 대기
              </span>
            )}
            {episode.failure !== null && (
              <span className="inline-flex items-center rounded-full border border-urgent px-2 py-px text-xs font-bold whitespace-nowrap text-urgent">
                받기 실패
              </span>
            )}
            {/* The current subtitle stays until the user approves the new one in its job. */}
            {approval.length > 0 && (
              <span className="inline-flex items-center rounded-full border border-focus px-2 py-px text-xs font-bold whitespace-nowrap text-focus">
                교체 승인
              </span>
            )}
            {/* Quiet and uncoloured: a candidate to look at, not a to-do. */}
            {picks.length > 0 && <span className="text-xs text-text-muted [overflow-wrap:anywhere]">{candidateNote(picks)}</span>}
            {/* A stored subtitle to apply is a choice, not a held subtitle: the check above stays `−`. */}
            {episode.subtitle.length === 0 && awaiting.length === 0 && others.length > 0 && (
              <span className="text-xs text-text-muted">보관본 있음</span>
            )}
          </span>
          {episode.revision !== null && <VersionLine revision={episode.revision} />}
        </span>
        {/* The air day is AniList's schedule of a releasing entry; blank when there is none. */}
        <span className="text-[12.5px] text-text-muted">{episode.air_at === null ? null : airDay(episode.air_at)}</span>
        <ChevronIcon className={cn("size-4 text-text-muted transition-transform", open && "rotate-90")} />
      </button>
      {open && (
        <Details
          id={detailId}
          workId={workId}
          season={season}
          animeNo={animeNo}
          episode={episode}
          picks={picks}
          source={source}
          onRetried={onRetried}
          onCreatorChanged={onCreatorChanged}
        />
      )}
    </li>
  );
}

const NO_PICKS: readonly Candidate[] = [];

interface EpisodeListProps {
  workId: string;
  /** The Anissia anime the season is linked to, `null` without a link. */
  animeNo: number | null;
  /** A subtitle file's creator changed: the page reads the work again. */
  onCreatorChanged: () => void;
  season: WorkSeason;
  /** How many seasons the work has: a single season's list needs no season in its title. */
  seasonCount: number;
  missing: boolean;
  order: EpisodeOrder;
  onOrder: (order: EpisodeOrder) => void;
  /** Reads the work again after a `다시 받기` of a failed replacement ended. */
  onRetried: () => Promise<void>;
  /** The season's subtitle candidates; without them the rows say nothing about candidates. */
  candidates?: EpisodeCandidateSource;
}

/**
 * The episodes of the chosen season, latest first unless `1화부터` is chosen.
 * A row says whether a video and a subtitle file are recorded for the episode,
 * with a `받기 실패` badge while a replacement of its video by a higher
 * revision has failed and a quiet version line once one went through; pressing
 * it opens the files and when each was added, and the failed replacement's two
 * files with why (and `다시 받기` when its download stopped). An episode with
 * no subtitle that other creators have a candidate for says so quietly, and its
 * opened row has `받기` for each. One with no subtitle but a stored one (another
 * episode of a package, `보관본 있음`) says so quietly too, and its opened row has
 * the stored subtitles with `적용`, which the job that stored it does.
 */
export function EpisodeList({
  workId,
  animeNo,
  onCreatorChanged,
  season,
  seasonCount,
  missing,
  order,
  onOrder,
  onRetried,
  candidates,
}: EpisodeListProps) {
  const [opened, setOpened] = useState<ReadonlySet<string>>(new Set());
  const toggle = (episode: string) =>
    setOpened((prev) => {
      const next = new Set(prev);
      if (!next.delete(episode)) next.add(episode);
      return next;
    });
  const title = missing ? "마지막으로 본 기록" : seasonCount > 1 ? `시즌 ${season.number} 회차` : "회차";

  return (
    <section aria-labelledby="episodes-title">
      <div className="mb-3 flex flex-wrap items-center gap-x-2.5 gap-y-2">
        <h2 id="episodes-title" className="text-[17px] font-bold">
          {title}
        </h2>
        <span className="rounded-full bg-surface-2 px-2 py-px text-xs font-bold text-text-secondary">{season.episodes.length}</span>
        {season.episodes.length > 1 && (
          <select
            aria-label="회차 순서"
            value={order}
            onChange={(e) => onOrder(e.target.value as EpisodeOrder)}
            className={cn(inputClass, "ml-auto w-[150px] max-w-full border py-1")}
          >
            {ORDERS.map((o) => (
              <option key={o.key} value={o.key}>
                {o.label}
              </option>
            ))}
          </select>
        )}
      </div>
      {season.episodes.length === 0 ? (
        <EmptyState>이 시즌 폴더에서 회차로 읽은 영상·자막 파일이 없어요.</EmptyState>
      ) : (
        <ul className="m-0 list-none overflow-hidden rounded-card border border-hairline-soft bg-surface-1 p-0 shadow-(--card-shadow)">
          {inOrder(season.episodes, order).map((episode) => (
            <Row
              key={episode.episode}
              workId={workId}
              animeNo={animeNo}
              onCreatorChanged={onCreatorChanged}
              season={season.number}
              episode={episode}
              missing={missing}
              picks={candidates ? candidatesOf(candidates.list, episode, candidates.subscribed) : NO_PICKS}
              source={candidates}
              open={opened.has(episode.episode)}
              onToggle={() => toggle(episode.episode)}
              onRetried={onRetried}
            />
          ))}
        </ul>
      )}
    </section>
  );
}
