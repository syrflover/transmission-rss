import { weekdayLongFromMonday } from "@/lib/weekday";

import type { Quarter } from "./api";

/** The name of a weekday counted from Monday (0); nothing for a number outside 0–6. */
export const weekdayName = (weekday: number) => weekdayLongFromMonday(weekday) ?? "";

/** "9월 30일" for `2026-09-30`. */
export function monthDay(date: string): string {
  const [, month, day] = date.split("-").map(Number);
  return `${month}월 ${day}일`;
}

/** "2026년 4분기". */
export const quarterName = (q: Quarter) => `${q.year}년 ${q.number}분기`;
