/**
 * The shapes of a job's placements and its 배치 확인 (`placements` and `confirm` of the job read,
 * `crates/trss-web/src/jobs_api.rs`, `jobs_api/placement.rs`). They have no imports so the logic that reads them
 * (`placementTable.ts`) runs under `node --test`; `api.ts` re-exports them.
 */

/** What came of a received file once stored and applied (`trss_jobs::place`). */
export type PlacementOutcome = "applied" | "stored" | "existing" | "no_video" | "held" | "failed" | "dropped";

/** One received file as it was placed: its episode, what came of it and where it is. */
export interface Placement {
  position: number;
  /** The receipt it was made from (a `JobFile`'s `id`). */
  file_id: string;
  /** The received file's name (with its folder in a package). */
  name: string;
  kind: "subtitle" | "font" | "attachment" | "companion" | "other";
  format: "ass" | "srt" | "smi" | "other" | null;
  /** The season's episode it is on; `null` while a person has to say, or for a file on no episode. */
  episode: number | null;
  /** The episode the candidate said. */
  anissia_episode: string | null;
  /** Why a person has to say its episode (`회차 확인 필요`). */
  question: string | null;
  /** The episode number the file's name says, as written (`13`), when it does. */
  named: string | null;
  /**
   * What put the file on `episode`: its source's episode mapping (`mapped`), its own number while the source has no
   * mapping (`same_number`, which follows the one decided later), or a person's choice or the same number where no
   * source is known (`explicit`).
   */
  assignment: "mapped" | "same_number" | "explicit" | null;
  action: "apply" | "store" | "drop";
  /** `null` while under way. */
  outcome: PlacementOutcome | null;
  note: string | null;
  /** Server paths: the video it was put beside, its applied copy while it is there, its stored file. */
  video: string | null;
  applied: string | null;
  stored: string | null;
  /**
   * For a kept font: `unchanged` (its Google Drive file was not received: it did not change), `same` (received, with
   * the bytes of a font kept before) or `new` (kept as a new file); `null` otherwise.
   */
  font_receipt: "unchanged" | "same" | "new" | null;
}

/** One episode a row of the 배치 확인 can go on. */
export interface ConfirmEpisode {
  episode: number;
  /** The base name of the episode's video when it has exactly one (`Show S01E01.mkv`), else `null`. */
  video: string | null;
  /** How many videos the library has for the episode; a subtitle is applied beside one only when there is exactly one. */
  videos: number;
  /** The episode has a subtitle already: a file applied there waits for a replacement comparison and approval. */
  subtitle: boolean;
}

/** What a job asks a person in its 배치 확인 (`jobs_api/placement.rs`). */
export interface ConfirmView {
  /**
   * `whole`: the whole plan of an upload or a find job before anything is kept; `relocate`: a relocation's plan (its
   * rows and `relocations`) before any copy moves, confirmed as it is; `held`: only the rows it asks about.
   */
  scope: "whole" | "relocate" | "held";
  /** The `position`s of the rows the person places: every one is sent back. */
  positions: number[];
  /** The season's episode count, when known. */
  total: number | null;
  /** The episodes a row can go on, in order: 1 to `total`, else up to the last known one. */
  episodes: ConfirmEpisode[];
}

/** One row of the answer to a 배치 확인. */
export interface PlacementChoice {
  position: number;
  /** The episode it goes on; `null` for a row on no episode. */
  episode: number | null;
  /** `false`: kept only (`적용하지 않음`), with the `episode` still recorded. */
  apply: boolean;
}

/**
 * One removal of a relocation job (재배치, `jobs_api/placement.rs`): an applied copy it takes off its old episode once
 * the person confirms.
 */
export interface Relocation {
  id: string;
  /** The placement that applies the same stored subtitle on its new episode; `null`: it is applied there already. */
  position: number | null;
  /** The episode the copy is on, and its path in the work folder. */
  episode: number;
  path: string;
  /**
   * `planned` until the person confirms; then `intended` and `set_aside` while it is taken off, `done` (taken off),
   * `kept` (left where it is) or `held` (what became of it is not known).
   */
  state: "planned" | "intended" | "set_aside" | "done" | "kept" | "held";
  /** Why a `kept` or `held` copy is where it is. */
  reason: string | null;
}
