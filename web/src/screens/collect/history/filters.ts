import { HISTORY_RESULTS, type HistoryCounts, type HistoryFilter, type HistoryResult } from "./api";

/**
 * The list's filters live in the URL so a link (the status board's `실패·중복 N개`,
 * a reload, the back button) opens the same list:
 * `/collect/history?result=add_failed,duplicate&channel=<id>`.
 */

/** Row labels of the results. */
export const RESULT_LABEL: Record<HistoryResult, string> = {
  received: "받음",
  no_match: "규칙 불일치",
  excluded: "제외",
  duplicate: "중복",
  add_failed: "추가 실패",
};

export interface ResultChip {
  id: string;
  label: string;
  results: HistoryResult[];
}

/** The result filters, in display order. The first is every result. */
export const RESULT_CHIPS: readonly ResultChip[] = [
  { id: "all", label: "전체", results: [] },
  { id: "received", label: "받음", results: ["received"] },
  { id: "no_match", label: "규칙 불일치", results: ["no_match"] },
  { id: "excluded", label: "제외", results: ["excluded"] },
  { id: "failed", label: "실패·중복", results: ["add_failed", "duplicate"] },
];

/** The result codes of a `result` query value, in a fixed order; unknown codes are dropped. */
export function parseResults(value: string | null): HistoryResult[] {
  if (!value) return [];
  const asked = new Set(value.split(",").map((code) => code.trim()));
  return HISTORY_RESULTS.filter((code) => asked.has(code));
}

export function readFilter(params: URLSearchParams): HistoryFilter {
  return {
    results: parseResults(params.get("result")),
    channel: params.get("channel") || null,
  };
}

/**
 * The query string for a filter, with the result codes joined by a plain comma
 * (`?result=add_failed,duplicate`); nothing chosen gives an empty string.
 */
export function writeFilter(filter: HistoryFilter): string {
  const parts: string[] = [];
  if (filter.results.length > 0) parts.push(`result=${filter.results.join(",")}`);
  if (filter.channel) parts.push(`channel=${encodeURIComponent(filter.channel)}`);
  return parts.length === 0 ? "" : `?${parts.join("&")}`;
}

export function sameResults(a: readonly HistoryResult[], b: readonly HistoryResult[]): boolean {
  return a.length === b.length && a.every((code) => b.includes(code));
}

/** How many records a chip stands for under the current channel filter. */
export function chipCount(chip: ResultChip, counts: HistoryCounts): number {
  if (chip.results.length === 0) return counts.total;
  return chip.results.reduce((sum, code) => sum + counts[code], 0);
}

/** The label for a result set that no chip stands for (a hand-written link). */
export function customLabel(results: readonly HistoryResult[]): string {
  return results.map((code) => RESULT_LABEL[code]).join("·");
}
