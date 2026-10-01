import type { EpisodeRange, WorkEpisode, WorkSeason } from "../api";
import { formatRanges } from "../model";

/**
 * What the work detail screen derives from the answer. It only reads the
 * recorded files: whether an episode has a video or a subtitle is whether a
 * file is recorded for it (never a later state such as applied or approved).
 */

/** An episode as shown: leading zeros of a whole number go (`01` is `1`); anything else stays as written. */
export function shownEpisode(episode: string): string {
  return /^\d+$/.test(episode) ? episode.replace(/^0+(?=\d)/, "") : episode;
}

/** `12화` for a number, the text itself for anything else (`SP`). */
export function episodeLabel(episode: string): string {
  const shown = shownEpisode(episode);
  return /^\d/.test(shown) ? `${shown}화` : shown;
}

/** The id of an episode's row, so a link can find it. */
export function rowId(season: number, episode: string): string {
  return `ep-s${season}-e${shownEpisode(episode)}`;
}

/** The ranges of consecutive whole-numbered episodes, then each other episode alone (as the library list does). */
function rangesOf(episodes: readonly WorkEpisode[]): EpisodeRange[] {
  const whole: { n: bigint; written: string }[] = [];
  const others: string[] = [];
  for (const { episode } of episodes) {
    if (/^\d+$/.test(episode)) whole.push({ n: BigInt(episode), written: episode });
    else others.push(episode);
  }
  whole.sort((a, b) => (a.n < b.n ? -1 : a.n > b.n ? 1 : 0));
  const out: EpisodeRange[] = [];
  let last: bigint | null = null;
  for (const { n, written } of whole) {
    if (last !== null && n === last + 1n) out[out.length - 1].last = written;
    else out.push({ first: written, last: written });
    last = n;
  }
  for (const written of others) out.push({ first: written, last: written });
  return out;
}

export interface SeasonSummary {
  number: number;
  /** `영상 1–12화` or `영상 없음` (a recorded range when the folder is gone). */
  video: string;
  subtitle: string;
}

/** A season's tile text: the episodes that have a video, and those that have a subtitle. */
export function summarize(season: WorkSeason, missing: boolean): SeasonSummary {
  const line = (label: string, has: (e: WorkEpisode) => boolean) => {
    const ranges = rangesOf(season.episodes.filter(has));
    if (ranges.length === 0) return `${label} 없음`;
    return `${missing ? `${label} 기록` : label} ${formatRanges(ranges)}화`;
  };
  return {
    number: season.number,
    video: line("영상", (e) => e.video.length > 0),
    subtitle: line("자막", (e) => e.subtitle.length > 0),
  };
}

/** The season shown first: the latest one. */
export function defaultSeason(seasons: readonly WorkSeason[]): number | null {
  return seasons.length === 0 ? null : seasons[seasons.length - 1].number;
}

export type EpisodeOrder = "latest" | "first";

export const ORDERS: { key: EpisodeOrder; label: string }[] = [
  { key: "latest", label: "최신 화부터" },
  { key: "first", label: "1화부터" },
];

/** The episodes in the order asked for (the answer's order is ascending). Returns a new array. */
export function inOrder(episodes: readonly WorkEpisode[], order: EpisodeOrder): WorkEpisode[] {
  return order === "latest" ? [...episodes].reverse() : [...episodes];
}

/** The last part of a relative path. */
export function baseName(path: string): string {
  return path.slice(path.lastIndexOf("/") + 1);
}
