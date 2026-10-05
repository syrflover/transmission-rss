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
}
