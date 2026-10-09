/**
 * The names of the weekdays, Sunday first, the order `Date.getDay()` and Anissia count them in. Imports nothing,
 * so the pure helpers that use it and their tests run without the app. Each caller says what it shows for a
 * number outside 0–6 (`weekdayName` of the schedule shows nothing, the import review `?요일`, the Anissia tabs
 * `신작`); the arrays give `undefined` there.
 */

/** `일`, `월` ... `토`. */
export const WEEKDAYS_SHORT: readonly string[] = ["일", "월", "화", "수", "목", "금", "토"];

/** `일요일`, `월요일` ... `토요일`. */
export const WEEKDAYS_LONG: readonly string[] = WEEKDAYS_SHORT.map((day) => `${day}요일`);

/**
 * The long name of a weekday counted from Monday (0), as the schedule counts them; `undefined` for anything but
 * a whole number from 0 to 6.
 */
export function weekdayLongFromMonday(weekday: number): string | undefined {
  return Number.isInteger(weekday) && weekday >= 0 && weekday <= 6 ? WEEKDAYS_LONG[(weekday + 1) % 7] : undefined;
}
