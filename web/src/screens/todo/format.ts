import type { FailureClass, FileFormat, JobRow, JobState, ItemState, UploadSummary, Wait } from "./api";
import { sizeText } from "./bytes.ts";
import { findShown } from "./findState.ts";

export { sizeText };

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

/** The episodes of a job or an item as the server shows them (`episodes_shown`), whole, for the expanded row (`11, 12, 14화`). */
export function episodeList(shown: readonly string[]): string {
  return `${shown.join(", ")}화`;
}

/** The label of a done, failed or waiting item, from its episode as the server shows it: `11` is `11화`. */
export function episodeName(shown: string): string {
  return `${shown}화`;
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
