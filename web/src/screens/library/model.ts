import type { EpisodeRange, LibraryWork } from "./api";

/**
 * The library list as the screen arranges it: sorts, filters, search and the
 * text of the two status lines. All of it only decides what is shown and in
 * what order; nothing here changes a work or starts a job.
 */

/** A work with what sorting and searching need, computed once per answer. */
export interface Work extends LibraryWork {
  /** The folder name in NFC (a name made on macOS may be decomposed). */
  title: string;
  /** Lower-case NFC title, for the search. */
  haystack: string;
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
      haystack: title.toLocaleLowerCase(),
      initial: initialOf(title),
      hue: hueOf(title),
    };
  });
}

// --- status lines -------------------------------------------------------------------

/** An episode as shown: leading zeros of a whole number go (`01` is `1`); anything else stays as written. */
function shown(episode: string): string {
  return /^\d+$/.test(episode) ? episode.replace(/^0+(?=\d)/, "") : episode;
}

/** `1–3·5–12`: the ranges of consecutive episodes, a single episode alone. */
export function formatRanges(ranges: EpisodeRange[]): string {
  return ranges
    .map(({ first, last }) => {
      const a = shown(first);
      const b = shown(last);
      return a === b ? a : `${a}–${b}`;
    })
    .join("·");
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

// --- sorts --------------------------------------------------------------------------

export type SortKey = "title" | "year" | "added" | "video" | "subtitle";

export const SORTS: { key: SortKey; label: string }[] = [
  { key: "title", label: "제목순" },
  { key: "year", label: "방영연도순" },
  { key: "added", label: "최근 작품 추가순" },
  { key: "video", label: "최근 영상 추가순" },
  { key: "subtitle", label: "최근 자막 추가순" },
];

export const DEFAULT_SORT: SortKey = "subtitle";

const collator = new Intl.Collator("ko");

/** Ascending by title; the ID settles two works with the same name. */
function byTitle(a: Work, b: Work): number {
  return collator.compare(a.title, b.title) || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0);
}

/** Latest first; an unknown time comes after every known one. */
function latestFirst(a: number | null, b: number | null): number {
  if (a === b) return 0;
  if (a === null) return 1;
  if (b === null) return -1;
  return b - a;
}

/** The time a sort looks at. Airing years are not known yet (no season is linked to an AniList entry), so that sort has none. */
function timeOf(work: Work, sort: SortKey): number | null {
  switch (sort) {
    case "added":
      return work.added_at;
    case "video":
      return work.video_added_at;
    case "subtitle":
      return work.subtitle_added_at;
    default:
      return null;
  }
}

/** The works in the order of `sort`: ties and unknown times by title. Returns a new array. */
export function sortWorks(works: readonly Work[], sort: SortKey): Work[] {
  const copy = [...works];
  if (sort === "title" || sort === "year") return copy.sort(byTitle);
  return copy.sort((a, b) => latestFirst(timeOf(a, sort), timeOf(b, sort)) || byTitle(a, b));
}

// --- filters ------------------------------------------------------------------------

export type FilterKey = "all" | "airing" | "complete" | "partial" | "none" | "check";

export const FILTERS: { key: FilterKey; label: string }[] = [
  { key: "all", label: "전체" },
  { key: "airing", label: "방영 중" },
  { key: "complete", label: "자막 다 갖춤" },
  { key: "partial", label: "자막 일부" },
  { key: "none", label: "자막 없음" },
  { key: "check", label: "확인 필요" },
];

function matches(work: Work, filter: FilterKey): boolean {
  switch (filter) {
    case "all":
      return true;
    // Airing information comes with the season's AniList entry, which no work has yet.
    case "airing":
      return false;
    case "complete":
      return work.subtitle_coverage === "all";
    case "partial":
      return work.subtitle_coverage === "some";
    case "none":
      return work.subtitle_coverage === "none";
    case "check":
      return work.subtitle_check_needed || work.missing;
  }
}

/** The works that pass the filter and contain the search text in their title. */
export function selectWorks(works: readonly Work[], filter: FilterKey, search: string): Work[] {
  const needle = normalize(search.trim()).toLocaleLowerCase();
  return works.filter((work) => matches(work, filter) && (needle === "" || work.haystack.includes(needle)));
}
