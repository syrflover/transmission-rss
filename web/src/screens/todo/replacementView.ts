import { when } from "../../lib/time.ts";
import { sizeText } from "./bytes.ts";
import type { Replacement, ReplacementPath, ReplacementVersion } from "./replacementTypes.ts";

/**
 * What the decision card of a job's replacement (`docs/specs/subtitles.md`, 교체 비교와 승인) shows, decided from
 * the API's `Replacement` alone, so the components only draw it and a test can say what appears when.
 */

/** The plans in episode order. */
export function inEpisodeOrder(replacements: readonly Replacement[]): Replacement[] {
  return [...replacements].sort((a, b) => a.episode - b.episode || a.position - b.position);
}

/** The plans a person can decide now, in episode order. */
export function openOnes(replacements: readonly Replacement[]): Replacement[] {
  return inEpisodeOrder(replacements).filter((r) => r.state === "open");
}

/** The rows (placement positions) whose replacement waits for the person's decision. */
export function waitingPositions(replacements: readonly Replacement[]): Set<number> {
  return new Set(openOnes(replacements).map((r) => r.position));
}

/**
 * How the cards of one job are laid out. Only the first card follows the page on a computer, and the phone's buttons
 * are fixed above the bottom menu only when there is one card to decide: several fixed bars would cover each other.
 */
export function cardLayout(open: number, index: number): { sticky: boolean; fixed: boolean } {
  return { sticky: index === 0, fixed: open === 1 };
}

/**
 * The facts of one version line, each its own item: when it was received (or, for a file the app did not manage,
 * the file's own time), its size and its dialogue lines. A fact the API does not know is left out.
 */
export function versionFacts(version: ReplacementVersion, now: number = Date.now()): string[] {
  const facts: string[] = [];
  if (version.received_at !== null) facts.push(`${when(version.received_at, now)} 받음`);
  else if (version.changed_at !== null) facts.push(`파일 시각 ${when(version.changed_at, now)}`);
  facts.push(sizeText(version.size));
  if (version.lines !== null) facts.push(`대사 ${version.lines}줄`);
  return facts;
}

/** One row of the side-by-side comparison: a name and the two files' values. A `null` link has no post. */
export interface CompareRow {
  name: string;
  current: string;
  next: string;
  /** Set for a post: the value is an address to link. */
  link?: boolean;
}

const UNKNOWN = "알 수 없음";

/** What a subtitle's format is called (the formats a replacement's files can have). */
const FORMAT: Record<NonNullable<ReplacementVersion["format"]>, string> = {
  ass: "ASS",
  srt: "SRT",
  smi: "SMI",
  other: "그 밖의 형식",
};

function creatorOf(version: ReplacementVersion): string {
  if (version.creator !== null) return version.creator;
  // An unmanaged file's source is not known; a stored one without a creator is the user's `제작자 알 수 없음`.
  return version.managed ? "제작자 알 수 없음" : "출처 미상";
}

/** The side-by-side rows, or none when the API says the version lines are enough. */
export function compareRows(r: Replacement): CompareRow[] {
  if (!r.side_by_side || r.current === null || r.new === null) return [];
  const { current, new: next } = r;
  return [
    { name: "제작자", current: creatorOf(current), next: creatorOf(next) },
    {
      name: "형식",
      current: current.format === null ? UNKNOWN : FORMAT[current.format],
      next: next.format === null ? UNKNOWN : FORMAT[next.format],
    },
    { name: "게시물", current: current.post ?? "—", next: next.post ?? "—", link: true },
    { name: "인코딩", current: current.encoding ?? "—", next: next.encoding ?? "—" },
  ];
}

export interface Notice {
  /** Stable key within the list. */
  key: string;
  title: string;
  /** The exact path the warning is about. */
  path: string | null;
  text: string;
}

/** The warnings of the paths the person may not expect, each with its exact path; none when no path has one. */
export function pathWarnings(paths: readonly ReplacementPath[]): Notice[] {
  const out: Notice[] = [];
  for (const p of paths) {
    if (p.warning === "overwrite_unmanaged") {
      out.push({
        key: `overwrite:${p.path}`,
        title: "trss가 관리하지 않는 파일을 덮어써요",
        path: p.path,
        text: "승인하면 이 파일의 바이트를 `제작자 알 수 없음` 보관본으로 남기고 덮어써요. 그 보관본은 이 회차의 지난 자막에서 다시 고를 수 있어요.",
      });
    } else if (p.warning === "remove_applied") {
      out.push({
        key: `remove:${p.path}`,
        title: "이전에 적용한 파일을 지워요",
        path: p.path,
        text: "승인하면 이 적용본을 지워요. 그 자막의 보관본은 그대로 남아요.",
      });
    }
  }
  return out;
}

/** Which of the two subtitles' dialogue lines could not be read, in words. */
function uncounted(r: Replacement): string | null {
  const current = r.current !== null && r.current.lines === null;
  const next = r.new !== null && r.new.lines === null;
  if (current && next) return "현재 자막과 새 자막의";
  if (current) return "현재 자막의";
  if (next) return "새 자막의";
  return null;
}

/** The limits of the comparison that hold, as warnings; none when `limits` is empty. */
export function limitNotices(r: Replacement): Notice[] {
  const out: Notice[] = [];
  for (const limit of r.limits) {
    if (limit === "unknown_source") {
      out.push({
        key: limit,
        title: "출처 미상",
        path: null,
        text: "현재 파일을 누가 만들었는지 몰라서 두 자막의 출처를 견줄 수 없어요.",
      });
    } else if (limit === "lines_unknown") {
      out.push({
        key: limit,
        title: "대사 수를 읽지 못함",
        path: null,
        text: `${uncounted(r) ?? "한 자막의"} 대사 줄 수를 읽지 못했어요.`,
      });
    }
  }
  return out;
}

/** `다시 비교 필요`, and `새 수정본 발견` when a newer revision came, with the reason; `null` for a first comparison. */
export function againNotice(r: Replacement): { tags: string[]; reason: string } | null {
  if (r.again === null) return null;
  return {
    tags: r.again.new_revision ? ["새 수정본 발견", "다시 비교 필요"] : ["다시 비교 필요"],
    reason: r.again.reason,
  };
}

/** What a plan that is not (or no longer) to decide says in the job's main area. */
export interface StateLine {
  label: string;
  detail: string | null;
  urgent: boolean;
}

/** The short line for a plan the person is not deciding now; `null` for an `open` one, which has the card. */
export function stateLine(r: Replacement): StateLine | null {
  switch (r.state) {
    case "open":
      return null;
    case "approved":
      return { label: "승인한 교체를 반영하는 중이에요", detail: null, urgent: false };
    case "kept":
      return { label: "현재 자막을 유지했어요", detail: null, urgent: false };
    case "done":
      return { label: "새 자막으로 교체했어요", detail: null, urgent: false };
    case "stale":
      return r.new_revision
        ? {
            label: "새 수정본 발견 · 다시 비교 필요",
            detail: `${r.reason ?? "같은 출처의 새 수정본이 들어왔어요"}. 새 수정본을 받은 작업에서 비교해요.`,
            urgent: false,
          }
        : { label: "다시 비교 필요", detail: r.reason, urgent: false };
    case "held":
      return { label: "보류", detail: r.reason, urgent: false };
    case "failed":
      return { label: "교체하지 못했어요", detail: r.reason, urgent: true };
  }
}

const ACTION: Record<ReplacementPath["action"], string> = {
  add: "추가",
  replace: "교체",
  remove: "제거",
  keep: "그대로 둠",
};

/** What a `done` plan did to each path. */
const DONE: Record<ReplacementPath["action"], string> = {
  add: "추가함",
  replace: "교체함",
  remove: "제거함",
  keep: "그대로 둠",
};

/** One line of the plan's paths: what happens to a path, and which. */
export interface PathRow {
  key: string;
  term: string;
  path: string;
  /** A fact about it that holds whatever is decided. */
  note: string | null;
}

/**
 * The plan's real changes for the 경로 part: each path beside the video with its action, then the current subtitle's
 * stored file, which stays whatever is decided. A `done` plan says what it did, and the subtitle it replaced is the
 * previous one. A plan that went `stale` is replaced by a newer one, and a `kept` plan changed nothing: they have none.
 */
export function pathRows(r: Replacement): PathRow[] {
  if (r.state === "stale" || r.state === "kept") return [];
  const done = r.state === "done";
  const rows: PathRow[] = r.paths.map((p) => ({
    key: `${p.action}:${p.path}`,
    term: (done ? DONE : ACTION)[p.action],
    path: p.path,
    note: null,
  }));
  if (r.current?.stored != null) {
    rows.push({
      key: `stored:${r.current.stored}`,
      term: done ? "이전 보관본" : "현재 보관본",
      path: r.current.stored,
      note: "그대로 남아요",
    });
  }
  return rows;
}

/** `3화`: the episode a plan is for. */
export function planEpisode(r: Replacement): string {
  return `${r.episode}화`;
}

/** A post's address as a short link text: its host and path, without the scheme or a trailing slash. */
export function postLabel(url: string): string {
  const bare = url.replace(/^[a-z][a-z0-9+.-]*:\/\//i, "").replace(/\/+$/, "");
  return bare === "" ? url : bare;
}
