import type { Quarter } from "./api";

const WEEKDAYS = ["월요일", "화요일", "수요일", "목요일", "금요일", "토요일", "일요일"];

/** The name of a weekday counted from Monday (0). */
export const weekdayName = (weekday: number) => WEEKDAYS[weekday] ?? "";

/** "9월 30일" for `2026-09-30`. */
export function monthDay(date: string): string {
  const [, month, day] = date.split("-").map(Number);
  return `${month}월 ${day}일`;
}

/** "2026년 4분기". */
export const quarterName = (q: Quarter) => `${q.year}년 ${q.number}분기`;
