import { useState } from "react";

import { cn } from "@/lib/utils";
import { dateTime } from "@/lib/time";

import type { WorkEpisode, WorkFile, WorkSeason } from "../api";
import { CheckIcon, ChevronIcon, MinusIcon } from "../icons";
import { baseName, episodeLabel, inOrder, ORDERS, rowId, type EpisodeOrder } from "./model";
import { EmptyState } from "../../ScreenFrame";
import { inputClass } from "../../collect/channels/styles";

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

/** The time a file was added, or `미상` when it was there before the app first looked. */
const addedAt = (file: WorkFile) => (file.added_at === null ? "미상" : dateTime(file.added_at));

function Details({ id, episode }: { id: string; episode: WorkEpisode }) {
  return (
    <dl id={id} className="m-0 grid grid-cols-[repeat(auto-fit,minmax(170px,1fr))] gap-x-4 gap-y-3 px-4 pt-1 pb-4 max-[720px]:px-3">
      <Cell label="영상 파일">
        <Files files={episode.video} />
      </Cell>
      <Cell label="자막 파일">
        <Files files={episode.subtitle} />
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
    </dl>
  );
}

function Row({
  season,
  episode,
  missing,
  open,
  onToggle,
}: {
  season: number;
  episode: WorkEpisode;
  missing: boolean;
  open: boolean;
  onToggle: () => void;
}) {
  const id = rowId(season, episode.episode);
  const detailId = `${id}-files`;
  return (
    <li className="border-t border-hairline-soft first:border-t-0">
      <button
        type="button"
        id={id}
        aria-expanded={open}
        aria-controls={detailId}
        onClick={onToggle}
        className={cn(
          "grid min-h-12 w-full scroll-mt-[calc(var(--topbar-h)+16px)] grid-cols-[4.2em_minmax(0,1fr)_16px] items-center gap-x-3 px-4 py-2.5 text-left hover:bg-surface-2 focus:bg-surface-2 max-[720px]:px-3 dark:hover:bg-surface-2 dark:focus:bg-surface-2",
          open && "bg-[color-mix(in_srgb,var(--focus-ring)_5%,var(--surface-1))]",
        )}
      >
        <span className="text-sm font-bold">{episodeLabel(episode.episode)}</span>
        <span className="flex flex-wrap items-center gap-x-4 gap-y-1">
          <Hold label="영상" on={episode.video.length > 0} missing={missing} />
          <Hold label="자막" on={episode.subtitle.length > 0} missing={missing} />
        </span>
        <ChevronIcon className={cn("size-4 text-text-muted transition-transform", open && "rotate-90")} />
      </button>
      {open && <Details id={detailId} episode={episode} />}
    </li>
  );
}

interface EpisodeListProps {
  season: WorkSeason;
  /** How many seasons the work has: a single season's list needs no season in its title. */
  seasonCount: number;
  missing: boolean;
  order: EpisodeOrder;
  onOrder: (order: EpisodeOrder) => void;
}

/**
 * The episodes of the chosen season, latest first unless `1화부터` is chosen.
 * A row says whether a video and a subtitle file are recorded for the episode;
 * pressing it opens the files and when each was added.
 */
export function EpisodeList({ season, seasonCount, missing, order, onOrder }: EpisodeListProps) {
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
              season={season.number}
              episode={episode}
              missing={missing}
              open={opened.has(episode.episode)}
              onToggle={() => toggle(episode.episode)}
            />
          ))}
        </ul>
      )}
    </section>
  );
}
