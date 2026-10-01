import { api } from "@/lib/api";
import { forget } from "@/lib/cached";

/**
 * The home screen's data (`src/web/schedule_api.rs`, `src/web/setup_api.rs`).
 * Times are Unix milliseconds; the dates are Asia/Seoul calendar days.
 */

/**
 * `영상 받기` is off: `paused` (`받기 멈춤`). Anissia marks the anime `OFF`:
 * `off` (`결방`), which stands in place of the video and the subtitle line.
 */
export type VideoState = "received" | "downloading" | "waiting" | "upcoming" | "paused" | "off";

/**
 * The last three are the states the subtitle side will fill; the server does
 * not send them yet, and the card has their lines ready.
 */
export type SubtitleState = "received" | "waiting" | "downloading" | "auth_required" | "episode_unconfirmed";

export interface Card {
  rule_id: string;
  anime_no: number;
  title: string;
  /** `HH:MM`, which goes past 24 for a late-night programme; `null` without a time. */
  time: string | null;
  air_at: number;
  episode: number | null;
  video: VideoState;
  /** `null` when the subscription takes no subtitles or is paused. */
  subtitle: SubtitleState | null;
  creator: string | null;
  /** The work of the connected season; the card opens it. */
  work_id: string | null;
  cover_url: string | null;
}

export interface Day {
  /** `YYYY-MM-DD`. */
  date: string;
  /** 0 (Monday) to 6 (Sunday). */
  weekday: number;
  today: boolean;
  cards: Card[];
}

export interface Quarter {
  year: number;
  number: number;
}

export interface Week {
  start: string;
  end: string;
  today: string;
  quarter: Quarter;
  days: Day[];
  next_quarter: { quarter: Quarter; subscriptions: number; title_waiting: number };
}

export type Step = "folder" | "import";

export interface FirstRun {
  /** A step is neither done nor skipped, so the checklist is shown. */
  active: boolean;
  steps: { step: Step; done: boolean; skipped: boolean }[];
}

export interface Home {
  now: number;
  /** Set while the checklist takes the place of the schedule. */
  first_run: FirstRun | null;
  week: Week | null;
}

export const WEEK_KEY = "schedule:week";

/** Something the home screen shows changed (a folder, a channel, a subscription): read it again next time. */
export function forgetWeek(): void {
  forget(WEEK_KEY);
}

export function loadHome(signal?: AbortSignal): Promise<Home> {
  return api<Home>("/schedule/week", { signal });
}

/** Skips a step of the checklist, or takes the skip back. */
export function setSkipped(step: Step, skipped: boolean): Promise<FirstRun> {
  return api<FirstRun>(`/first-run/${step}`, { method: "PUT", body: { skipped } });
}
