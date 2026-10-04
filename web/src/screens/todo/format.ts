import type { FailureClass, FileFormat, JobRow, JobState, ItemState, UploadSummary, Wait } from "./api";
import { findShown } from "./findState.ts";

/** What a failure class is called (`docs/specs/jobs.md`, 공통 수신 결과와 실패 분류). */
export const FAILURE_LABEL: Record<FailureClass, string> = {
  missing: "원본 없음",
  expired: "만료",
  not_a_file: "파일 아님",
  changed: "출처 구조 바뀜",
  network: "네트워크 실패",
  no_subtitle: "자막 없음",
  needs_input: "추가 입력 필요",
};

/** What a checked file's format is called. */
export const FORMAT_LABEL: Record<FileFormat, string> = {
  zip: "ZIP",
  ass: "ASS",
  srt: "SRT",
  smi: "SMI",
  other: "그 밖의 형식",
};

/** `자막 2개 · 폰트 1개 · 압축 파일 1개`: what an upload job kept, by kind; `null` when it kept nothing. */
export function uploadKept(upload: Pick<UploadSummary, "subtitles" | "fonts" | "archives">): string | null {
  const parts = [
    upload.subtitles > 0 ? `자막 ${upload.subtitles}개` : null,
    upload.fonts > 0 ? `폰트 ${upload.fonts}개` : null,
    upload.archives > 0 ? `압축 파일 ${upload.archives}개` : null,
  ].filter((part): part is string => part !== null);
  return parts.length > 0 ? parts.join(" · ") : null;
}

/** An episode as shown: leading zeros of a whole number go (`01` is `1`); anything else stays as written. */
function shown(episode: string): string {
  return /^\d+$/.test(episode) ? episode.replace(/^0+(?=\d)/, "") : episode;
}

/** How many segments of the episode list the line names before it says `외 N개`. */
const SEGMENTS = 3;

export interface EpisodeLabel {
  /** `11화`, `2–3화`, `2·5화`, `1–4·7화`: what is shown bold. */
  label: string;
  /** Episodes past the shortened list (`외 3개`); 0 when the list is whole. */
  more: number;
}

/**
 * The episodes of a to-do or a job in a line: consecutive whole numbers as a
 * range (`2–3화`), the rest joined by `·`, and a long list shortened (`1–4·7화`
 * and `외 3개`). An empty list has no label.
 */
export function episodeLabel(episodes: readonly string[]): EpisodeLabel | null {
  if (episodes.length === 0) return null;
  const segments: { text: string; count: number }[] = [];
  let run: number[] = [];
  const flush = () => {
    if (run.length === 0) return;
    const first = run[0];
    const last = run[run.length - 1];
    segments.push({ text: run.length === 1 ? String(first) : `${first}–${last}`, count: run.length });
    run = [];
  };
  for (const episode of episodes) {
    const text = shown(episode);
    if (/^\d+$/.test(text)) {
      const n = Number(text);
      if (run.length > 0 && n === run[run.length - 1] + 1) run.push(n);
      else {
        flush();
        run = [n];
      }
    } else {
      flush();
      segments.push({ text, count: 1 });
    }
  }
  flush();
  const head = segments.slice(0, SEGMENTS);
  const more = segments.slice(SEGMENTS).reduce((sum, s) => sum + s.count, 0);
  return { label: `${head.map((s) => s.text).join("·")}화`, more };
}

/** The episodes of a job or an item, whole, for the expanded row (`11, 12, 14화`). */
export function episodeList(episodes: readonly string[]): string {
  return `${episodes.map(shown).join(", ")}화`;
}

/** The label of a done, failed or waiting item: `11` is `11화`. */
export function episodeName(episode: string): string {
  return `${shown(episode)}화`;
}

/** Bytes as `22 KB`, `1.4 MB`; a size under 1 KB is `512 B`. */
export function sizeText(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const kb = bytes / 1024;
  if (kb < 1024) return `${kb < 10 ? kb.toFixed(1).replace(/\.0$/, "") : Math.round(kb)} KB`;
  const mb = kb / 1024;
  return `${mb < 10 ? mb.toFixed(1).replace(/\.0$/, "") : Math.round(mb)} MB`;
}

/**
 * The state a badge names; `waiting` splits by what it waits for, and `running` by its stage. A find job (직접 찾기)
 * whose server browser is open for the user is `finding`, one the user finished `finishing` until it ends, and one
 * that ended with no file kept `nothing` (`받은 파일 없음`, not `받음`).
 */
export type Shown =
  | "failed"
  | "partial"
  | "auth"
  | "subtitle"
  | "placement"
  | "approval"
  | "video"
  | "waiting"
  | "pending"
  | "held"
  | "open"
  | "receive"
  | "store"
  | "apply"
  | "finding"
  | "finishing"
  | "nothing"
  | "done";

export function shownState(
  job: Pick<JobRow, "state" | "wait" | "stage"> & Partial<Pick<JobRow, "origin" | "upload" | "finishing">>,
): Shown {
  if (job.origin === "find") {
    const upload = job.upload ?? null;
    const kept = upload === null ? 0 : upload.subtitles + upload.fonts + upload.archives;
    const find = findShown({ state: job.state, wait: job.wait, finishing: job.finishing === true, kept });
    if (find !== null) return find;
  }
  switch (job.state) {
    case "waiting":
      return job.wait ?? "waiting";
    case "running":
      return job.stage === "open" || job.stage === "store" || job.stage === "apply" ? job.stage : "receive";
    default:
      return job.state;
  }
}

export function shownItem(state: ItemState, wait: Wait | null): Shown | "item-pending" {
  switch (state) {
    case "waiting":
      return wait === "auth" ? "auth" : "subtitle";
    case "running":
      return "receive";
    case "pending":
      return "item-pending";
    default:
      return state;
  }
}

/** A job that has ended: its detail stops polling. */
export function ended(state: JobState): boolean {
  return state === "done" || state === "failed" || state === "partial";
}
