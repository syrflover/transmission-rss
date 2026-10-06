import { when } from "../../lib/time.ts";

/** What came of unpacking a received archive. */
export interface UnpackResult {
  /**
   * `done`: its files were taken out; `failed`: it could not be unpacked (풀지 못함, with `reason`) and stays in
   * the receive area; `retry`: a try this machine failed (a full disk, a limit; with `reason`) waits for the next;
   * `volume`: a later volume of the split archive `first`, unpacked with it.
   */
  state: "done" | "failed" | "retry" | "volume";
  reason: string | null;
  /**
   * How many tries failed for this machine, out of {@link UNPACK_TRIES}, and for `failed` the try after them that
   * the archive's own reason ended; 0 for none.
   */
  tries: number;
  /**
   * For `retry`: when the next try goes at the latest; `null` once a worker started since, which tries it at once.
   * A time gone by is the next run's too (the job is put back in line, or a person holds it).
   */
  retry_at: number | null;
  /** For `volume`: the first volume's name. */
  first: string | null;
  /** For `done`: how many files it held, and how many of them are subtitles and fonts. */
  files: number | null;
  subtitles: number | null;
  fonts: number | null;
}

/** How many tries an archive gets when this machine fails them (the worker's `UNPACK_TRIES`). */
export const UNPACK_TRIES = 3;

/**
 * What came of unpacking an archive, in a phrase: `파일 14개를 풀었어요 (자막 12 · 폰트 2)`, `풀지 못함` (urgent,
 * with why in `reason`, and how many tries it took when this machine failed them), that a try this machine failed
 * waits for the next (which try failed, why, and when the next goes), or that a later volume goes with its first
 * (named in `reason`).
 */
export function unpackText(
  unpack: UnpackResult,
  now: number = Date.now(),
): { text: string; reason: string | null; urgent: boolean } {
  switch (unpack.state) {
    case "done": {
      const kinds = [
        unpack.subtitles ? `자막 ${unpack.subtitles}` : null,
        unpack.fonts ? `폰트 ${unpack.fonts}` : null,
      ].filter((part): part is string => part !== null);
      return {
        text: `파일 ${unpack.files ?? 0}개를 풀었어요${kinds.length > 0 ? ` (${kinds.join(" · ")})` : ""}`,
        reason: null,
        urgent: false,
      };
    }
    case "failed":
      return {
        text: "풀지 못함",
        reason: unpack.tries > 0 ? `${unpack.reason} · ${unpack.tries}번 시도했어요` : unpack.reason,
        urgent: true,
      };
    case "retry": {
      const next =
        unpack.retry_at !== null && unpack.retry_at > now
          ? `${when(unpack.retry_at, now)} 또는 worker가 다시 시작할 때`
          : "다음 실행 때";
      return {
        text: `다시 풀기를 기다려요 (${UNPACK_TRIES}번 중 ${unpack.tries}번째 시도 실패)`,
        reason: [unpack.reason, `다음 시도: ${next}`].filter((part) => part !== null).join(" · "),
        urgent: false,
      };
    }
    case "volume":
      return {
        text: "나뉜 조각 · 첫 조각과 함께 풀어요",
        reason: unpack.first !== null ? `첫 조각: ${unpack.first}` : null,
        urgent: false,
      };
  }
}
