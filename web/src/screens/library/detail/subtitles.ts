import { creatorText, receivedAt, sameDay } from "../storage.ts";
import { episodeLabel, shownEpisode } from "./episodeKey.ts";

/**
 * The pure parts of the `자막` card (`library.md`, 오른쪽 카드 열): what each stored copy says of itself and which
 * actions it is offered, the line of the format order and the moves of its editor. Nothing here reads the server or
 * decides what may be applied; the server sends `choice`, `can_add` and `blocked`, and these only show them. It has no
 * `@/` imports so it runs under `node --test`.
 */

/** A subtitle format the app applies; the settings' policy names the same three. */
export type SubtitleFormat = "ass" | "srt" | "smi";

/** The order of the subtitle formats a work picks by: its own (`own`) or the global one. */
export interface FormatOrder {
  order: SubtitleFormat[];
  own: boolean;
}

/** A stored copy of a subtitle, as the `자막` card lists it. */
export interface SubtitleCopy {
  id: string;
  season: number;
  /** As written in the file names. */
  episode: string;
  name: string;
  /** A format code; a code the app does not apply is read as `그 밖의 형식`. */
  format: string;
  /** When it was stored (Unix milliseconds). */
  stored_at: number;
  /** Relative to the work folder. */
  stored_path: string;
  /** The copies of it that lie beside a video now; none when it is not the episode's subtitle. */
  applied: { path: string; applied_at: number }[];
  /** `apply`: the episode has no subtitle, so applying is direct; `compare`: it opens the replacement comparison. */
  choice: "apply" | "compare" | null;
  /** `추가 적용`: the same creator's other format, which never takes off the applied copy. */
  can_add: boolean;
  /** Why the copy cannot be chosen; `null` for a copy that can, and for the applied copy itself. */
  blocked: string | null;
}

/** The copies of one creator; `null` is `제작자 알 수 없음`. */
export interface SubtitleCreatorCopies {
  creator: string | null;
  copies: SubtitleCopy[];
}

/** What the `자막` card shows (`GET /api/library/works/{id}` `subtitles`). */
export interface WorkSubtitles {
  format_order: FormatOrder;
  /** By creator's name, `제작자 알 수 없음` last; the copies by season, episode, then newest first. */
  creators: SubtitleCreatorCopies[];
}

/** The words of the formats the app applies; any other code is `그 밖의 형식`. */
const FORMAT_TEXT: Record<SubtitleFormat, string> = { ass: "ASS", srt: "SRT", smi: "SMI" };

export const OTHER_FORMAT = "그 밖의 형식";

export function formatText(format: string): string {
  return Object.hasOwn(FORMAT_TEXT, format) ? FORMAT_TEXT[format as SubtitleFormat] : OTHER_FORMAT;
}

/** `SRT → ASS → SMI`. */
export function orderText(order: readonly SubtitleFormat[]): string {
  return order.map(formatText).join(" → ");
}

/** Whose order it is: `이 작품의 순서` when the work has its own, else `전역 순서`. */
export function orderOwnerText(order: FormatOrder): string {
  return order.own ? "이 작품의 순서" : "전역 순서";
}

/** The line a folded card shows: `제작자 2명 · 자막 5개`, or that the season has none. */
export function cardSummary(groups: readonly SubtitleCreatorCopies[]): string {
  if (groups.length === 0) return "보관한 자막 없음";
  const copies = groups.reduce((sum, group) => sum + group.copies.length, 0);
  return `제작자 ${groups.length}명 · 자막 ${copies}개`;
}

/** The groups that have a copy of the season, in the order the server sent; the others are left out. */
export function seasonGroups(subtitles: WorkSubtitles, season: number): SubtitleCreatorCopies[] {
  return subtitles.creators
    .map((group) => ({ creator: group.creator, copies: group.copies.filter((copy) => copy.season === season) }))
    .filter((group) => group.copies.length > 0);
}

/** The head of a creator's group. */
export function groupTitle(group: SubtitleCreatorCopies): string {
  return creatorText(group.creator);
}

/** What an action asks for: `apply` and `compare` choose the copy (`mode: "apply"`); `add` puts it beside. */
export type CopyAction = "apply" | "compare" | "add";

export const ACTION_LABEL: Record<CopyAction, string> = { apply: "적용", compare: "교체 비교", add: "추가 적용" };

/**
 * What a screen reader calls an action's button: `2화 ASS 9월 7일 04:50 받음 교체 비교`. The copies of one episode and
 * format differ by when they were received, as the row says.
 */
export function actionName(view: CopyView, action: CopyAction): string {
  return `${view.episode} ${view.format} ${view.received} ${ACTION_LABEL[action]}`;
}

/** The mode the request carries. */
export function actionMode(action: CopyAction): "apply" | "add" {
  return action === "add" ? "add" : "apply";
}

/** The place a copy is applied at and the place it is stored at, which are two files and never one. */
export interface Places {
  /** Relative to the work folder, one per copy beside a video. */
  applied: string[];
  stored: string;
}

export interface CopyView {
  id: string;
  /** `2화`. */
  episode: string;
  format: string;
  /** `9월 7일 받음`, with the time when another copy of the episode and format came the same day. */
  received: string;
  name: string;
  /** The copy lies beside a video: the one the episode uses now (`적용`). */
  isApplied: boolean;
  /** Only for an applied copy. */
  places: Places | null;
  /** In the order they are shown; none when the copy is blocked. */
  actions: CopyAction[];
  /** The sentence that stands in place of the buttons. */
  blocked: string | null;
}

/** The actions a copy is offered: none when the server says why it cannot be chosen. */
export function actionsOf(copy: SubtitleCopy): CopyAction[] {
  if (copy.blocked !== null) return [];
  const out: CopyAction[] = [];
  if (copy.choice === "apply") out.push("apply");
  if (copy.choice === "compare") out.push("compare");
  if (copy.can_add) out.push("add");
  return out;
}

/**
 * `copy` as its card row shows it. `group` is every copy of its creator: another of the same season, episode and format
 * received the same day makes the date carry the time.
 */
export function copyView(copy: SubtitleCopy, group: readonly SubtitleCopy[]): CopyView {
  const shared = group.some(
    (other) =>
      other.id !== copy.id &&
      other.season === copy.season &&
      shownEpisode(other.episode) === shownEpisode(copy.episode) &&
      other.format === copy.format &&
      sameDay(other.stored_at, copy.stored_at),
  );
  const isApplied = copy.applied.length > 0;
  return {
    id: copy.id,
    episode: episodeLabel(copy.episode),
    format: formatText(copy.format),
    received: receivedAt(copy.stored_at, shared),
    name: copy.name,
    isApplied,
    places: isApplied ? { applied: copy.applied.map((a) => a.path), stored: copy.stored_path } : null,
    actions: actionsOf(copy),
    blocked: copy.blocked,
  };
}

/** What the card does once the server answered: `compare` when the job plans a replacement to approve. */
export type ApplyOutcome = { kind: "compare"; job: string } | { kind: "sent"; job: string };

export function applyOutcome(answer: { job_id: string; compare: boolean }): ApplyOutcome {
  return { kind: answer.compare ? "compare" : "sent", job: answer.job_id };
}

// --- the order editor -----------------------------------------------------------------------------------------------

/** `order` with the format at `index` moved by `by`; the same order when it cannot move. */
export function moveFormat(order: readonly SubtitleFormat[], index: number, by: -1 | 1): SubtitleFormat[] {
  const next = [...order];
  const to = index + by;
  if (index < 0 || index >= next.length || to < 0 || to >= next.length) return next;
  [next[index], next[to]] = [next[to], next[index]];
  return next;
}

export function sameOrder(a: readonly SubtitleFormat[], b: readonly SubtitleFormat[]): boolean {
  return a.length === b.length && a.every((format, index) => format === b[index]);
}

/**
 * Whether `저장` is offered: the draft differs from the order shown, or the work still follows the global order (saving
 * the same order then makes it the work's own).
 */
export function canSaveOrder(draft: readonly SubtitleFormat[], current: FormatOrder): boolean {
  return !current.own || !sameOrder(draft, current.order);
}
