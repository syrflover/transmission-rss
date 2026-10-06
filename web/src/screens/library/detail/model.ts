import type { EpisodeRange, FuzzyDate, SeasonInfo, WorkEpisode, WorkSeason } from "../api";
import { formatRanges } from "../model";
import { shownEpisode } from "./episodeKey.ts";

/**
 * What the work detail screen derives from the answer. It only reads the
 * recorded files: whether an episode has a video or a subtitle is whether a
 * file is recorded for it (never a later state such as applied or approved).
 */

/** `episodeLabel` and `shownEpisode` live in `episodeKey.ts`, which the pure helpers import without the app. */
export { episodeLabel, shownEpisode } from "./episodeKey.ts";

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

/** A date as far as AniList knows it: `2022년 7월 2일`, `2022년 7월`, `2022년`; `null` when not even the year is known. */
export function fuzzyDate(date: FuzzyDate): string | null {
  if (date.year === null) return null;
  const parts = [`${date.year}년`];
  if (date.month !== null) {
    parts.push(`${date.month}월`);
    if (date.day !== null) parts.push(`${date.day}일`);
  }
  return parts.join(" ");
}

/** A short label for how the linked entries stand, `null` when the dates say it all. */
export function airingBadge(info: SeasonInfo): string | null {
  switch (info.airing?.state) {
    case "releasing":
      return "방영 중";
    case "not_yet_released":
      return "방영 예정";
    case "hiatus":
      return "휴방";
    case "cancelled":
      return "취소";
    default:
      return null;
  }
}

/** The `방영` cell: the first entry's start through the last entry's end, `미상` when unknown. */
export function airingText(info: SeasonInfo): string {
  const airing = info.airing;
  if (!airing) return "미상";
  const start = fuzzyDate(airing.start);
  const end = fuzzyDate(airing.end);
  if (airing.state === "releasing") return start ? `${start} ~` : "미상";
  if (start && end) return start === end ? start : `${start} ~ ${end}`;
  if (start) return airing.state === "finished" ? `${start} ~ 종료일 미상` : `${start} ~`;
  return end ? `~ ${end}` : "미상";
}

/** `13화` for an entry with a known count, `미상` otherwise. */
export function episodesText(count: number | null): string {
  return count === null ? "미상" : `${count}화`;
}

/** The air day of an episode (`7월 2일 (토)`, with the year when it is not this year's). */
export function airDay(at: number, now: Date = new Date()): string {
  const d = new Date(at);
  const weekday = ["일", "월", "화", "수", "목", "금", "토"][d.getDay()];
  const year = d.getFullYear() === now.getFullYear() ? "" : `${d.getFullYear()}년 `;
  return `${year}${d.getMonth() + 1}월 ${d.getDate()}일 (${weekday})`;
}

/** Anissia's `ON`, `OFF` and `END` as words. */
export function anissiaStatusText(status: string): string {
  switch (status) {
    case "ON":
      return "방영 중";
    case "OFF":
      return "휴방";
    case "END":
      return "종영";
    default:
      return status;
  }
}
