import { when } from "../../lib/time.ts";
import type {
  Comparison,
  DialogueLine,
  ReplacementChanges,
  StyleChange,
  TimingLine,
} from "./replacementTypes.ts";

/**
 * What the content comparison of a replacement shows (`docs/specs/subtitles.md`, 교체 비교와 승인; the 교체 승인
 * card's reason line in `docs/specs/jobs.md`, 할 일 화면), decided from the API's `Comparison` and lines alone, so
 * the components only draw it and a test can say what appears when. A part that was not compared is never shown as
 * a number: it says `비교 불가` and why.
 */

/** What the section says when the plan was made before the app compared contents. */
export const NOT_COMPARED_YET = "이 계획은 내용을 비교하기 전에 만들어졌어요";

/** What an item says when it was not compared and the API gave no reason. */
const NO_REASON = "이 항목은 비교하지 못했어요";

/** The engine counts a start or end that moved by no more than this as not moved (`compare::TIMING_TOLERANCE_MS`). */
const TIMING_TOLERANCE_MS = 10;

/** How many lines show at first, and how many each `더 보기` adds. */
export const LINES_STEP = 200;

// ---------------------------------------------------------------- the 교체 승인 card

/** The reason line of a `교체 승인` to-do: tags in a fixed order. */
export function todoTags(changes: ReplacementChanges): string[] {
  // No open plan (the job settled between reads): there is nothing to say, least of all 차이 없음.
  if (changes.plans === 0) return [];
  const numbered: [string, number][] = [
    ["대사 추가", changes.added],
    ["대사 변경", changes.changed],
    ["대사 삭제", changes.removed],
    ["타이밍", changes.timing],
    ["스타일", changes.styles],
    ["폰트", changes.fonts],
  ];
  const tags = numbered.filter(([, n]) => n > 0).map(([name, n]) => `${name} ${n}`);
  // A count beside `일부 비교 불가` or `비교 불가` says how many plans, so only when there are several.
  const counted = (text: string, n: number) => (changes.plans > 1 ? `${text} ${n}` : text);
  if (changes.partial > 0) tags.push(counted("일부 비교 불가", changes.partial));
  if (changes.uncompared > 0) {
    // When no plan was compared, the number would only repeat `all of them`.
    tags.push(changes.uncompared < changes.plans ? counted("비교 불가", changes.uncompared) : "비교 불가");
  }
  // Only what was compared whole can say that nothing differs.
  if (tags.length === 0) tags.push("차이 없음");
  return tags;
}

/** One fact of the work detail's `교체 승인` card's time line: whose time, and the time as it reads. */
export interface ReceivedFact {
  label: "현재" | "새 자막";
  text: string;
}

/**
 * The time line of the work detail's `교체 승인` card (`docs/specs/library.md`, 할 일과 회차 목록): when the current
 * and the new subtitle were received, the newest of each when several plans wait, said as the job detail's version
 * lines say it (`replacementView.ts`, `versionFacts`). A current file the app did not manage has its file time.
 */
export function receivedLine(
  todo: { current_received_at: number | null; current_changed_at: number | null; new_received_at: number | null },
  now: number = Date.now(),
): ReceivedFact[] {
  const facts: ReceivedFact[] = [];
  if (todo.current_received_at !== null) {
    facts.push({ label: "현재", text: `${when(todo.current_received_at, now)} 받음` });
  } else if (todo.current_changed_at !== null) {
    facts.push({ label: "현재", text: `파일 시각 ${when(todo.current_changed_at, now)}` });
  }
  if (todo.new_received_at !== null) facts.push({ label: "새 자막", text: `${when(todo.new_received_at, now)} 받음` });
  return facts;
}

/**
 * One plan's part of the `교체 승인` card's sum, by the server's rule (`todo_api::Changes::add`), so a row of the
 * episode list says what the card would for that episode alone.
 */
export function planChanges(comparison: Comparison | null): ReplacementChanges {
  const none = {
    added: 0,
    changed: 0,
    removed: 0,
    timing: 0,
    styles: 0,
    fonts: 0,
    uncompared: 0,
    partial: 0,
    plans: 1,
  };
  if (comparison === null || comparison.state !== "compared") return { ...none, uncompared: 1 };
  const { dialogue, styles, fonts } = comparison;
  const ass = comparison.current.format === "ASS" || comparison.new.format === "ASS";
  const leftOut =
    comparison.not_compared.some((n) => n.item === "dialogue") || (ass && (styles === null || fonts === null));
  return {
    ...none,
    added: dialogue.added,
    changed: dialogue.changed,
    removed: dialogue.removed,
    timing: comparison.timing.count,
    styles: styles === null ? 0 : styles.added.length + styles.removed.length + styles.changed.length,
    fonts: fonts === null ? 0 : fonts.added.length + fonts.removed.length,
    partial: leftOut ? 1 : 0,
  };
}

// ---------------------------------------------------------------- the 변경 사항 section

/** One number tag of an item (`추가 2`). A `dim` one is 0. */
export interface ChangeTag {
  text: string;
  dim: boolean;
}

/** What an item shows when it expands, apart from the dialogue and timing lines the screen fetches. */
export type ItemDetail =
  /** The dialogue lines, below the reasons that part of the dialogue was not compared, one line each. */
  | { kind: "dialogue"; notes: string[] }
  /** The timing lines. */
  | { kind: "timing" }
  | { kind: "styles"; added: string[]; changed: StyleChange[]; removed: string[] }
  | { kind: "fonts"; added: string[]; removed: string[] }
  /** The item was not compared: why. */
  | { kind: "reason"; reason: string };

export type ItemKey = "dialogue" | "timing" | "styles" | "fonts";

/** One of the four items, collapsed at first. An item with no `detail` has nothing to expand. */
export interface ItemView {
  key: ItemKey;
  name: string;
  tags: ChangeTag[];
  detail: ItemDetail | null;
}

/** The section: four items, or one `비교 불가` block when the contents were not compared at all. */
export type ChangesView = { kind: "items"; items: ItemView[] } | { kind: "none"; reason: string };

const count = (text: string, n: number): ChangeTag => ({ text: `${text} ${n}`, dim: n === 0 });
const word = (text: string): ChangeTag => ({ text, dim: false });

/** The items of a plan's comparison; `null` is a plan made before the app compared contents. */
export function changesView(comparison: Comparison | null): ChangesView {
  if (comparison === null) return { kind: "none", reason: NOT_COMPARED_YET };
  if (comparison.state === "unreadable") return { kind: "none", reason: comparison.reason };

  const why = (item: "dialogue" | "styles" | "fonts") =>
    comparison.not_compared.filter((n) => n.item === item).map((n) => n.reason);
  const { dialogue, timing, styles, fonts } = comparison;
  const items: ItemView[] = [];

  // The dialogue is always counted; `not_compared` for it means only that some language had no counterpart. That is
  // said beside the numbers, not instead of them.
  const skipped = why("dialogue");
  const partial = skipped.length > 0 ? [word("일부 비교 불가")] : [];
  const dialogueTotal = dialogue.added + dialogue.changed + dialogue.removed;
  items.push({
    key: "dialogue",
    name: "대사",
    tags: [count("추가", dialogue.added), count("변경", dialogue.changed), count("삭제", dialogue.removed), ...partial],
    detail: dialogueTotal + skipped.length > 0 ? { kind: "dialogue", notes: skipped } : null,
  });
  items.push({
    key: "timing",
    name: "타이밍",
    tags: [count("바뀐 줄", timing.count)],
    detail: timing.count > 0 ? { kind: "timing" } : null,
  });

  if (styles === null) {
    items.push(notCompared("styles", "스타일", why("styles")));
  } else {
    const total = styles.added.length + styles.changed.length + styles.removed.length;
    items.push({
      key: "styles",
      name: "스타일",
      tags: [
        count("추가", styles.added.length),
        count("변경", styles.changed.length),
        count("삭제", styles.removed.length),
      ],
      detail:
        total > 0 ? { kind: "styles", added: styles.added, changed: styles.changed, removed: styles.removed } : null,
    });
  }

  if (fonts === null) {
    items.push(notCompared("fonts", "폰트", why("fonts")));
  } else {
    items.push({
      key: "fonts",
      name: "폰트",
      tags: [count("추가", fonts.added.length), count("삭제", fonts.removed.length)],
      detail:
        fonts.added.length + fonts.removed.length > 0
          ? { kind: "fonts", added: fonts.added, removed: fonts.removed }
          : null,
    });
  }
  return { kind: "items", items };
}

function notCompared(key: ItemKey, name: string, reasons: string[]): ItemView {
  return {
    key,
    name,
    tags: [word("비교 불가")],
    detail: { kind: "reason", reason: reasons.length > 0 ? reasons.join(" ") : NO_REASON },
  };
}

/** Whether opening this item needs the lines the screen fetches. */
export function needsLines(key: ItemKey): boolean {
  return key === "dialogue" || key === "timing";
}

// ---------------------------------------------------------------- times

const pad = (n: number, width = 2) => String(n).padStart(width, "0");

/** `1:05` or, from an hour, `1:02:03`: a time in milliseconds, to the second below. */
export function clock(ms: number): string {
  const total = Math.floor(Math.max(0, ms) / 1000);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  return h > 0 ? `${h}:${pad(m)}:${pad(s)}` : `${m}:${pad(s)}`;
}

/** `0:01:02.300`: a time in milliseconds with its hour and milliseconds, to compare two moments of a line. */
export function exactClock(ms: number): string {
  const t = Math.max(0, Math.floor(ms));
  const h = Math.floor(t / 3_600_000);
  const m = Math.floor((t % 3_600_000) / 60_000);
  const s = Math.floor((t % 60_000) / 1000);
  return `${h}:${pad(m)}:${pad(s)}.${pad(t % 1000, 3)}`;
}

// ---------------------------------------------------------------- lines

/** One text of a dialogue row: the text, or a bar for the line that did not exist on that side. */
export type RowText = { bar: false; text: string } | { bar: true };

/** One dialogue line to draw: its time and class, then the old and the new row. */
export interface DialogueRow {
  key: string;
  /** Its start; `0:12 → 0:13` when a changed line moved. */
  time: string;
  class: string | null;
  kind: DialogueLine["kind"];
  old: RowText;
  new: RowText;
}

const BAR: RowText = { bar: true };

/** The row of each dialogue line, in the order the API gives (time order). */
export function dialogueRows(lines: readonly DialogueLine[]): DialogueRow[] {
  return lines.map((line, i) => {
    const { old, new: next } = line;
    const from = old === undefined ? null : clock(old.start);
    const to = next === undefined ? null : clock(next.start);
    return {
      key: String(i),
      time: from !== null && to !== null && from !== to ? `${from} → ${to}` : (to ?? from ?? ""),
      class: line.class ?? null,
      kind: line.kind,
      // An added line has no old row and a removed one no new row: a bar stands in, never an empty text.
      old: old === undefined ? BAR : { bar: false, text: old.text },
      new: next === undefined ? BAR : { bar: false, text: next.text },
    };
  });
}

/** One moved start or end of a timing line. */
export interface Move {
  name: "시작" | "끝";
  from: string;
  to: string;
}

/** One timing line to draw: its text and class, then what moved. */
export interface TimingRow {
  key: string;
  text: string;
  class: string | null;
  moves: Move[];
}

/** The row of each timing line: its start and/or end that moved, from → to, with hours and milliseconds. */
export function timingRows(lines: readonly TimingLine[]): TimingRow[] {
  return lines.map((line, i) => {
    const fields = [
      { name: "시작" as const, from: line.old.start, to: line.new.start },
      { name: "끝" as const, from: line.old.end, to: line.new.end },
    ];
    // A field that moved by no more than the engine's tolerance has not moved; if none did, any that differs.
    const exceeds = fields.filter((f) => Math.abs(f.to - f.from) > TIMING_TOLERANCE_MS);
    const moved = exceeds.length > 0 ? exceeds : fields.filter((f) => f.to !== f.from);
    return {
      key: String(i),
      text: line.text,
      class: line.class ?? null,
      moves: moved.map((f) => ({ name: f.name, from: exactClock(f.from), to: exactClock(f.to) })),
    };
  });
}

/** How many lines of `total` show when `shown` were asked for, and what the next `더 보기` adds (`n줄 더 보기`). */
export function cutLines(total: number, shown: number): { visible: number; more: number } {
  const visible = Math.min(total, Math.max(shown, LINES_STEP));
  return { visible, more: Math.min(LINES_STEP, total - visible) };
}
