import type { EpisodeRange, FilterKey, LibraryWork, SortKey } from "./api";

/**
 * The library list as the screen shows it: the names of the sorts and filters
 * (the server sorts, filters and searches, see `src/store/library/page.rs`) and
 * the text of the two status lines. Nothing here changes a work or starts a job.
 */

/** A work with what its cover placeholder needs, computed once per answer. */
export interface Work extends LibraryWork {
  /** The folder name in NFC (a name made on macOS may be decomposed). */
  title: string;
  /** The first letter or digit of the title, for the cover placeholder. */
  initial: string;
  /** A hue (0-359) for the cover placeholder, the same for the same title. */
  hue: number;
}

const normalize = (text: string) => text.normalize("NFC");

function hueOf(text: string): number {
  let hash = 2166136261;
  for (let i = 0; i < text.length; i++) {
    hash ^= text.charCodeAt(i);
    hash = Math.imul(hash, 16777619);
  }
  return (hash >>> 0) % 360;
}

function initialOf(title: string): string {
  const letter = /[\p{L}\p{N}]/u.exec(title);
  return (letter ? letter[0] : (Array.from(title)[0] ?? "?")).toLocaleUpperCase();
}

/** The title in NFC and what its cover placeholder is made of. */
export function coverOf(name: string): Pick<Work, "title" | "hue" | "initial"> {
  const title = normalize(name);
  return { title, initial: initialOf(title), hue: hueOf(title) };
}

export function prepare(works: LibraryWork[]): Work[] {
  return works.map((work) => {
    const title = normalize(work.name);
    return {
      ...work,
      title,
      initial: initialOf(title),
      hue: hueOf(title),
    };
  });
}

// --- status lines -------------------------------------------------------------------

/** `1–3·5–12`: the ranges of consecutive episodes as the server shows them, a single episode alone. */
export function formatRanges(ranges: EpisodeRange[]): string {
  return ranges.map(({ text }) => text).join("·");
}

/** The video line: `영상 1–12화`, `영상 없음`, or `폴더 없음` when the work's folder is gone. */
export function videoLine(work: LibraryWork): string {
  if (work.missing) return "폴더 없음";
  return work.video.length === 0 ? "영상 없음" : `영상 ${formatRanges(work.video)}화`;
}

/** The subtitle line: `자막 1–3·5–12화` or `자막 없음`; a work whose folder is gone cannot be checked. */
export function subtitleLine(work: LibraryWork): string {
  if (work.missing) return "자막 확인 불가";
  return work.subtitle.length === 0 ? "자막 없음" : `자막 ${formatRanges(work.subtitle)}화`;
}

// --- sorts and filters ----------------------------------------------------------------

export const SORTS: { key: SortKey; label: string }[] = [
  { key: "title", label: "제목순" },
  { key: "year", label: "방영연도순" },
  { key: "added", label: "최근 작품 추가순" },
  { key: "video", label: "최근 영상 추가순" },
  { key: "subtitle", label: "최근 자막 추가순" },
];

export const DEFAULT_SORT: SortKey = "subtitle";

export const FILTERS: { key: FilterKey; label: string }[] = [
  { key: "all", label: "전체" },
  { key: "airing", label: "방영 중" },
  { key: "complete", label: "자막 다 갖춤" },
  { key: "partial", label: "자막 일부" },
  { key: "none", label: "자막 없음" },
  { key: "check", label: "확인 필요" },
];
