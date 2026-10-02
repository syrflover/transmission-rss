import type { JobRow, JobState, ItemState, Wait } from "./api";

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

/** The state a badge names; `waiting` splits by what it waits for. */
export type Shown =
  | "failed"
  | "partial"
  | "auth"
  | "subtitle"
  | "waiting"
  | "pending"
  | "held"
  | "open"
  | "receive"
  | "done";

export function shownState(job: Pick<JobRow, "state" | "wait" | "stage">): Shown {
  switch (job.state) {
    case "waiting":
      return job.wait === "auth" ? "auth" : job.wait === "subtitle" ? "subtitle" : "waiting";
    case "running":
      return job.stage === "open" ? "open" : "receive";
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
