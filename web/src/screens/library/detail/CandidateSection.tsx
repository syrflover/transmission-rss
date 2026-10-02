import { useId, useMemo, useState } from "react";

import { Button } from "@/components/ui/button";
import { ApiError } from "@/lib/api";
import { isOpen, newCommandId, sendCommand, type Command } from "@/lib/commands";
import { ago } from "@/lib/time";
import { cn } from "@/lib/utils";

import { btnAction, btnNeutral, hintClass } from "../../collect/channels/styles";
import { MAX_JOB_CANDIDATES } from "../../todo/api";
import { ANISSIA_CAPTIONS_KIND, type AnissiaLink, type CandidateList, type WorkEpisode } from "../api";
import { ChevronIcon } from "../icons";
import { CreateStatus, JobStatusLink, KindTag, PostLink, UpdatedAt } from "./CandidateParts";
import {
  candidateLabel,
  groupsOf,
  isHeld,
  missingOf,
  pickedOf,
  useCandidates,
  useCreateJob,
  type CandidateGroup,
  type Kind,
} from "./candidates";
import type { EpisodeOrder } from "./model";
import { SeasonAnissiaDialog } from "./SeasonAnissiaDialog";

/** The season's candidates as `useCandidates` reads them. */
export type Candidates = ReturnType<typeof useCandidates>;

const chip = "rounded-full bg-surface-2 px-2 py-px text-xs font-bold whitespace-nowrap text-text-secondary";

/** The line under the header: when Anissia was last read, and a refresh that failed. */
function readLine(list: CandidateList): string[] {
  const lines = [list.read_at === null ? "아직 Anissia의 최근 목록을 읽지 않았어요." : `Anissia의 최근 목록을 ${ago(list.read_at)} 읽었어요.`];
  const refresh = list.refresh;
  if (refresh?.state === "failed") {
    lines.push(`새로고침이 ${ago(refresh.finished_at ?? refresh.updated_at)} 실패했어요.${refresh.outcome?.reason ? ` ${refresh.outcome.reason}` : ""}`);
  }
  return lines;
}

/** Whether a conflict's current value is a command still going on: the refresh to follow. */
function openCommand(current: unknown): boolean {
  const command = current as Partial<Command> | undefined;
  return !!command && typeof command.id === "string" && command.state !== undefined && isOpen(command as Command);
}

/**
 * One row of an expanded group: a box to pick it, the episode as Anissia
 * writes it, what kind of candidate it is, how a job stands for it, when
 * Anissia's line was updated, and the post. A candidate a job holds cannot be
 * picked; one whose job failed or was held can.
 */
function CandidateItem({
  row,
  creator,
  checked,
  older,
  onToggle,
}: {
  row: { candidate: CandidateGroup["rows"][number]["candidate"]; kind: Kind };
  creator: string;
  checked: boolean;
  older: boolean;
  onToggle: (id: number) => void;
}) {
  const { candidate: c, kind } = row;
  const id = useId();
  const held = isHeld(c);
  const label = candidateLabel(c.episode);
  return (
    <li className="flex items-start gap-1.5 border-t border-hairline-soft py-1 first:border-t-0" data-testid="candidate-row">
      <label
        htmlFor={id}
        className="flex size-9 flex-none cursor-pointer items-center justify-center has-[:disabled]:cursor-not-allowed max-[720px]:size-10"
      >
        <input
          id={id}
          type="checkbox"
          className="size-[18px] accent-focus max-[720px]:size-5"
          checked={checked && !held}
          disabled={held}
          onChange={() => onToggle(c.id)}
        />
        <span className="sr-only">
          {creator} {label} 고르기
        </span>
      </label>
      <div className="flex min-w-0 flex-1 flex-col gap-0.5 py-1.5">
        <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
          <span className="text-sm font-bold [overflow-wrap:anywhere]">{label}</span>
          <KindTag kind={kind} job={c.job} />
          {kind !== "received" && <JobStatusLink job={c.job} />}
          {older && <span className="text-xs text-text-muted">이전 관찰</span>}
        </div>
        <p className="m-0 flex flex-wrap items-center gap-x-2.5 gap-y-0.5 text-xs text-text-muted [overflow-wrap:anywhere]">
          <UpdatedAt candidate={c} />
          {c.revision && <span>{c.revision.same_post ? "이전과 같은 게시물" : "이전과 다른 게시물"}</span>}
          <PostLink url={c.post_url} />
        </p>
      </div>
    </li>
  );
}

/**
 * A creator: the folded line (name, source, episodes, counts) and, opened, its
 * episodes with `모두 받기` and `고른 N개 받기`. Both make one job for the
 * creator's candidates; the picks of this group never mix with another's.
 */
function GroupItem({
  group,
  workId,
  season,
  open,
  onToggleOpen,
  picked,
  onToggle,
  onMade,
}: {
  group: CandidateGroup;
  workId: string;
  season: number;
  open: boolean;
  onToggleOpen: () => void;
  picked: ReadonlySet<number>;
  onToggle: (id: number) => void;
  onMade: (ids: readonly number[], jobId: string) => void;
}) {
  const bodyId = useId();
  const { phase, create, resend } = useCreateJob(workId, season, onMade);
  const all = missingOf(group);
  const chosen = pickedOf(group, picked);
  const sending = phase.kind === "sending";
  const tooMany = (ids: readonly number[]) => ids.length > MAX_JOB_CANDIDATES;
  const hint =
    all.length === 0
      ? "받을 수 있는 누락 회차가 없어요."
      : tooMany(all)
        ? `한 번에 ${MAX_JOB_CANDIDATES}개까지 받을 수 있어요. 나눠서 골라 주세요.`
        : group.revision > 0
          ? "수정 후보는 빼고 담아요."
          : null;

  return (
    <li className="border-t border-hairline-soft first:border-t-0" data-testid="candidate-group">
      <button
        type="button"
        aria-expanded={open}
        aria-controls={bodyId}
        onClick={onToggleOpen}
        className={cn(
          "flex min-h-12 w-full items-center gap-x-3 gap-y-1 px-4 py-2.5 text-left hover:bg-surface-2 focus:bg-surface-2 max-[720px]:px-3 dark:hover:bg-surface-2 dark:focus:bg-surface-2",
          open && "bg-[color-mix(in_srgb,var(--focus-ring)_5%,var(--surface-1))]",
        )}
      >
        <span className="flex min-w-0 flex-1 flex-wrap items-center gap-x-3 gap-y-1">
          <span className="flex min-w-0 basis-40 flex-1 flex-col gap-0.5">
            <span className="text-sm font-bold [overflow-wrap:anywhere]">
              {group.creator}
              {group.subscribed && <span className="ml-2 text-xs font-semibold text-text-muted">구독 제작자</span>}
            </span>
            <span className="text-xs text-text-muted [overflow-wrap:anywhere]">
              {[group.host, group.range].filter((part) => part).join(" · ")}
            </span>
          </span>
          <span className="flex flex-wrap items-center gap-1.5">
            {group.missing > 0 && <span className={chip}>누락 {group.missing}</span>}
            {group.revision > 0 && <span className={chip}>수정 {group.revision}</span>}
          </span>
        </span>
        <ChevronIcon className={cn("size-4 flex-none text-text-muted transition-transform", open && "rotate-90")} />
      </button>
      {open && (
        <div id={bodyId} className="px-4 pt-1 pb-4 max-[720px]:px-3">
          <ul className="m-0 list-none p-0">
            {group.rows.map((row) => (
              <CandidateRows key={row.key} row={row} creator={group.creator} picked={picked} onToggle={onToggle} />
            ))}
          </ul>
          <div className="mt-3 flex flex-wrap items-center gap-2">
            <Button
              type="button"
              variant="ghost"
              className={btnAction}
              disabled={sending || all.length === 0 || tooMany(all)}
              onClick={() => create(all)}
            >
              모두 받기
            </Button>
            {chosen.length > 0 && (
              <Button type="button" variant="ghost" className={btnAction} disabled={sending || tooMany(chosen)} onClick={() => create(chosen)}>
                고른 {chosen.length}개 받기
              </Button>
            )}
            {hint && <span className={hintClass}>{hint}</span>}
          </div>
          <div className="mt-2 empty:hidden">
            <CreateStatus phase={phase} onResend={resend} />
          </div>
        </div>
      )}
    </li>
  );
}

/** An episode's newest observation, then the older ones of it that are picked. */
function CandidateRows({
  row,
  creator,
  picked,
  onToggle,
}: {
  row: CandidateGroup["rows"][number];
  creator: string;
  picked: ReadonlySet<number>;
  onToggle: (id: number) => void;
}) {
  return (
    <>
      <CandidateItem row={row} creator={creator} checked={picked.has(row.candidate.id)} older={false} onToggle={onToggle} />
      {row.picked.map((older) => (
        <CandidateItem key={older.candidate.id} row={older} creator={creator} checked older onToggle={onToggle} />
      ))}
    </>
  );
}

/**
 * The `자막 후보` section of the chosen season, between the Anissia link and
 * the episode list: the creators Anissia lists for the season's anime, each
 * folded to a line, and the way to receive their subtitles one job at a time.
 * A season without an Anissia link offers the link instead.
 *
 * What the user has picked is a set of candidate IDs, never row positions, so
 * a reading that brings new candidates (or changes who is the newest of an
 * episode) neither moves nor adds a pick.
 */
export function CandidateSection({
  workId,
  link,
  seasonCount,
  onAnissiaChanged,
  episodes,
  order,
  subscribed,
  candidates,
}: {
  workId: string;
  link: AnissiaLink;
  seasonCount: number;
  onAnissiaChanged: (link: AnissiaLink) => void;
  episodes: readonly WorkEpisode[];
  order: EpisodeOrder;
  /** The subscription's creator, whose group comes first. */
  subscribed: string | null;
  candidates: Candidates;
}) {
  const { season } = link;
  const animeNo = link.anime?.anime_no ?? null;
  const { data, error, slow, reload, taken } = candidates;
  const list = data ?? undefined;
  const [opened, setOpened] = useState<ReadonlySet<string>>(new Set());
  const [picked, setPicked] = useState<ReadonlySet<number>>(new Set());
  const [linking, setLinking] = useState(false);
  const [sending, setSending] = useState(false);
  const [refreshError, setRefreshError] = useState<string | null>(null);

  const groups = useMemo(
    () => (list ? groupsOf(list, episodes, order, subscribed, picked) : []),
    [list, episodes, order, subscribed, picked],
  );

  const toggleOpen = (sourceId: string) =>
    setOpened((prev) => {
      const next = new Set(prev);
      if (!next.delete(sourceId)) next.add(sourceId);
      return next;
    });
  const toggle = (id: number) =>
    setPicked((prev) => {
      const next = new Set(prev);
      if (!next.delete(id)) next.add(id);
      return next;
    });
  // The job took these: they are not picks any more, and the list shows them as taken.
  const made = (ids: readonly number[], jobId: string) => {
    setPicked((prev) => new Set([...prev].filter((id) => !ids.includes(id))));
    taken(ids, jobId);
  };

  const refreshing = sending || (list?.refresh != null && isOpen(list.refresh));
  const refresh = async () => {
    if (animeNo === null || refreshing) return;
    setSending(true);
    setRefreshError(null);
    try {
      await sendCommand(newCommandId(), ANISSIA_CAPTIONS_KIND, { anime_no: animeNo });
    } catch (e) {
      // A reading that is already going on is the one to follow.
      if (!(e instanceof ApiError && e.code === "conflict" && openCommand(e.current))) {
        setRefreshError(e instanceof ApiError ? e.message : "새로고침하지 못했어요.");
      }
    }
    await reload();
    setSending(false);
  };

  const titleId = `candidates-title-${season}`;
  const title = seasonCount > 1 ? `시즌 ${season} 자막 후보` : "자막 후보";

  return (
    <section aria-labelledby={titleId} data-testid="candidate-section">
      <div className="flex flex-wrap items-center gap-x-2.5 gap-y-2">
        <h2 id={titleId} className="text-[17px] font-bold">
          {title}
        </h2>
        {groups.length > 0 && <span className={chip}>{groups.length}명</span>}
        {animeNo !== null && (
          <Button
            type="button"
            variant="ghost"
            className={cn(btnNeutral, "ml-auto")}
            disabled={refreshing}
            onClick={() => void refresh()}
          >
            {refreshing ? "읽는 중…" : "새로고침"}
          </Button>
        )}
      </div>

      {animeNo === null ? (
        <div className="mt-2 flex flex-wrap items-center gap-x-3 gap-y-2">
          <p className={cn(hintClass, "m-0 text-[13px]")}>Anissia 작품을 연결하면 자막 후보가 보여요.</p>
          <Button type="button" variant="ghost" className={btnNeutral} aria-haspopup="dialog" onClick={() => setLinking(true)}>
            Anissia 연결
          </Button>
          <SeasonAnissiaDialog workId={workId} link={link} open={linking} onOpenChange={setLinking} onChanged={onAnissiaChanged} />
        </div>
      ) : (
        <>
          {list && (
            <p className={cn(hintClass, "m-0 mt-1.5 text-[12.5px]")}>
              {readLine(list).join(" ")}
            </p>
          )}
          {refreshError && (
            <p role="alert" className="m-0 mt-1.5 text-[13px] font-semibold text-urgent">
              {refreshError}
            </p>
          )}
          {!list ? (
            error !== null ? (
              <p role="alert" className="m-0 mt-2 text-[13px] font-semibold text-urgent">
                {error}
              </p>
            ) : slow ? (
              <p className={cn(hintClass, "m-0 mt-2 text-[13px]")}>자막 후보를 불러오는 중이에요.</p>
            ) : null
          ) : groups.length === 0 ? (
            <p className={cn(hintClass, "m-0 mt-2 text-[13px]")}>아직 이 작품의 자막 후보가 없어요. 새로고침하면 Anissia를 바로 읽어요.</p>
          ) : (
            <ul className="m-0 mt-3 list-none overflow-hidden rounded-card border border-hairline-soft bg-surface-1 p-0 shadow-(--card-shadow)">
              {groups.map((group) => (
                <GroupItem
                  key={group.sourceId}
                  group={group}
                  workId={workId}
                  season={season}
                  open={opened.has(group.sourceId)}
                  onToggleOpen={() => toggleOpen(group.sourceId)}
                  picked={picked}
                  onToggle={toggle}
                  onMade={made}
                />
              ))}
            </ul>
          )}
        </>
      )}
    </section>
  );
}
