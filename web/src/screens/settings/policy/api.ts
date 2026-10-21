import { api } from "@/lib/api";
import type { Policy, PolicyBody, PolicyRange, SubtitleFormat } from "@/lib/api-types";

/** The cache key of the policy (`/api/settings/policy`), shared by the settings list row and its panel. */
export const POLICY_KEY = "settings:policy";

/*
 * The shape of the policy is generated (`lib/api-types`, `cargo types`).
 */
export type { Policy, PolicyRange, SubtitleFormat, WorkFormatOrder } from "@/lib/api-types";

/** What a save sends besides the version: the form's values. */
export type PolicyValues = Omit<PolicyBody, "version">;

export function loadPolicy(signal?: AbortSignal): Promise<Policy> {
  return api<Policy>("/settings/policy", { signal });
}

export function savePolicy(version: number, values: PolicyValues): Promise<Policy> {
  const body: PolicyBody = { version, ...values };
  return api<Policy>("/settings/policy", { method: "PUT", body });
}

/** True when a `conflict` answer carries the stored policy, as `current`. */
export function isPolicy(value: unknown): value is Policy {
  const policy = value as Partial<Policy> | null;
  return (
    typeof policy === "object" &&
    policy !== null &&
    Array.isArray(policy.format_order) &&
    typeof policy.idle_timeout_seconds === "number" &&
    typeof policy.max_concurrent_jobs === "number" &&
    typeof policy.version === "number" &&
    typeof policy.limits === "object"
  );
}

export const FORMAT_LABEL: Record<SubtitleFormat, string> = { ass: "ASS", srt: "SRT", smi: "SMI" };

/** `ASS → SRT → SMI`. */
export function formatOrderText(order: SubtitleFormat[]): string {
  return order.map((format) => FORMAT_LABEL[format]).join(" → ");
}

/** `5분`, or `1분 30초` for a stored time that is not a whole minute. */
export function idleText(seconds: number): string {
  const minutes = Math.floor(seconds / 60);
  const rest = seconds % 60;
  if (rest === 0) return `${minutes}분`;
  return minutes === 0 ? `${rest}초` : `${minutes}분 ${rest}초`;
}

/** The idle time as the whole minutes the input shows. */
export function idleMinutesText(seconds: number): string {
  return String(Math.round(seconds / 60));
}

/** The idle time limits in whole minutes: the smallest and largest whole minute within the seconds. */
export function idleMinuteRange(limits: Policy["limits"]): PolicyRange {
  return {
    min: Math.ceil(limits.idle_timeout_seconds.min / 60),
    max: Math.floor(limits.idle_timeout_seconds.max / 60),
  };
}

/** `text` as a whole number within `range`, or `null` when it is empty, not whole or out of range. */
export function inRange(text: string, range: PolicyRange): number | null {
  if (!/^\d+$/.test(text.trim())) return null;
  const value = Number(text);
  return value >= range.min && value <= range.max ? value : null;
}

/** What the settings list shows for the item: `ASS → SRT → SMI · 5분 · 1개`. */
export function policySummary(policy: Policy): string {
  return `${formatOrderText(policy.format_order)} · ${idleText(policy.idle_timeout_seconds)} · ${policy.max_concurrent_jobs}개`;
}
