/**
 * The shapes of a job's replacements (`replacements` of the job read, `src/web/jobs_api/replacement.rs`). They have
 * no imports so the logic that reads them (`replacementView.ts`) runs under `node --test`; `api.ts` re-exports them.
 */

/**
 * One subtitle of a decision card's version lines: `현재` (the file beside the video) or `새 자막` (the stored file
 * that would replace it).
 */
export interface ReplacementVersion {
  /** When the app received it (Unix ms); `null` for a file the app did not manage. */
  received_at: number | null;
  /** The file's change time (Unix ms), only for a current file the app did not manage. */
  changed_at: number | null;
  /** Bytes. */
  size: number;
  /** Dialogue lines; `null` when the app cannot count them. */
  lines: number | null;
  creator: string | null;
  format: "ass" | "srt" | "smi" | "other" | null;
  /** The post it came from. */
  post: string | null;
  encoding: string | null;
  /** Whether the app manages it: a stored subtitle, or a copy it applied with the bytes it applied. */
  managed: boolean;
  /** Its server path: the file beside the video (`current`) or the stored file (`new`). */
  path: string;
  /** For `current`: its stored file, which stays whatever is decided. */
  stored: string | null;
}

/** What a plan does to one path beside the video. */
export interface ReplacementPath {
  /** The full server path. */
  path: string;
  action: "add" | "replace" | "remove" | "keep";
  managed: boolean;
  /**
   * A change the person may not expect: `overwrite_unmanaged` (a file the app did not manage is replaced) or
   * `remove_applied` (an earlier applied copy at another path goes).
   */
  warning: "overwrite_unmanaged" | "remove_applied" | null;
}

/** Where a replacement plan is: `open` is the one a person decides. */
export type ReplacementState = "open" | "approved" | "kept" | "done" | "stale" | "held" | "failed";

/** The latest replacement plan of a row whose episode had a subtitle (`docs/specs/subtitles.md`, 교체 비교와 승인). */
export interface Replacement {
  /** With `version`: what a decision names. Never shown. */
  plan_id: string;
  version: number;
  /** The placement's `position` it belongs to. */
  position: number;
  state: ReplacementState;
  /** Why it is `stale`, `held` or `failed`. */
  reason: string | null;
  /** `stale` because a newer revision of the same source came. */
  new_revision: boolean;
  season: number;
  episode: number;
  /** Set when the version before went stale: `다시 비교 필요`, and `새 수정본 발견` with `new_revision`. */
  again: { reason: string; new_revision: boolean } | null;
  current: ReplacementVersion | null;
  new: ReplacementVersion | null;
  /** Show the two files' facts side by side: the creator, format or post differs, or the current source is unknown. */
  side_by_side: boolean;
  paths: ReplacementPath[];
  /** The limits of the comparison that hold, only those. */
  limits: ("unknown_source" | "lines_unknown")[];
  /** What differs between the two files' contents; `null` for a plan made before the app compared contents. */
  comparison: Comparison | null;
}

/** What was read of one side of a comparison. */
export interface ComparedSide {
  format: "ASS" | "SRT" | "WebVTT" | "SMI";
  encoding: "UTF-8" | "UTF-16" | "CP949";
  /** The cues of all its tracks. */
  cues: number;
}

/** A style the two files both have, with each field that differs. */
export interface StyleChange {
  name: string;
  fields: { field: string; old: string; new: string }[];
}

/** The items of a comparison that can be left out, with why. */
export type ComparedItem = "dialogue" | "styles" | "fonts";

/**
 * The comparison a plan keeps (`docs/specs/subtitles.md`, 교체 비교와 승인). `unreadable` says why a file's content
 * could not be read, `compared` the counts of the four items and what could not be compared. Its dialogue and timing
 * lines come from `fetchReplacementLines`, not with the job.
 */
export type Comparison =
  | { state: "unreadable"; reason: string }
  | {
      state: "compared";
      current: ComparedSide;
      new: ComparedSide;
      dialogue: { added: number; changed: number; removed: number };
      timing: { count: number };
      /** `null` when the styles cannot be compared (see `not_compared`). */
      styles: { added: string[]; removed: string[]; changed: StyleChange[] } | null;
      /** `null` when the fonts cannot be compared. */
      fonts: { added: string[]; removed: string[] } | null;
      not_compared: { item: ComparedItem; reason: string }[];
    };

/** A dialogue line's text and where it is, in milliseconds. */
export interface PlacedText {
  text: string;
  start: number;
  end: number;
}

/** A line of the dialogue comparison. An added line has no `old`, a removed one no `new`. */
export interface DialogueLine {
  kind: "added" | "changed" | "removed";
  /** The SMI language class, only when a file has several. */
  class?: string;
  old?: PlacedText;
  new?: PlacedText;
}

/** A line whose start or end moved, in milliseconds. */
export interface TimingLine {
  text: string;
  class?: string;
  old: { start: number; end: number };
  new: { start: number; end: number };
}

/** The lines of a plan's comparison (`/subtitle-jobs/{id}/replacements/{plan}/lines`). */
export interface ReplacementLines {
  dialogue: DialogueLine[];
  timing: TimingLine[];
}

/**
 * What a `교체 승인` to-do sums over the open plans it counts: the lines and items that differ, and how many plans
 * have no comparison to count (none was made, or a file could not be read).
 */
export interface ReplacementChanges {
  added: number;
  changed: number;
  removed: number;
  timing: number;
  styles: number;
  fonts: number;
  uncompared: number;
  /**
   * Compared plans that left a part out: a language of the dialogue with no counterpart, or the styles and fonts of
   * an ASS set against another format.
   */
  partial: number;
  /** The open plans summed, `uncompared` and `partial` of them among them. */
  plans: number;
}

/** One of several decisions sent at once: the plan, the version the list showed, and what to do. */
export interface ReplacementDecision {
  plan: string;
  version: number;
  decision: "replace" | "keep";
}

/** What became of one decision sent with others: written, or `stale` for a plan that changed since it was read. */
export interface ReplacementResult {
  plan: string;
  state: "approved" | "kept" | "stale";
}
