import { api } from "@/lib/api";
import { forget } from "@/lib/cached";
import type { FirstRun, Home, SkipBody, Step } from "@/lib/api-types";

/**
 * The home screen's data (`src/web/schedule_api.rs`, `src/web/setup_api.rs`).
 * Times are Unix milliseconds; the dates are Asia/Seoul calendar days.
 */

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
  const body: SkipBody = { skipped };
  return api<FirstRun>(`/first-run/${step}`, { method: "PUT", body });
}
