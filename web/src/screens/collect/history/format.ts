const WEEKDAYS = ["일", "월", "화", "수", "목", "금", "토"];

/** A stable key for the local calendar day of a time. */
export function dayKey(millis: number): string {
  const d = new Date(millis);
  return `${d.getFullYear()}-${d.getMonth() + 1}-${d.getDate()}`;
}

/** `9월 28일 (월)`, with the year when it is not this year. */
export function dayHeading(millis: number, now: number = Date.now()): string {
  const d = new Date(millis);
  const year = d.getFullYear() === new Date(now).getFullYear() ? "" : `${d.getFullYear()}년 `;
  return `${year}${d.getMonth() + 1}월 ${d.getDate()}일 (${WEEKDAYS[d.getDay()]})`;
}

/** `13:40`, under a date heading. */
export function clock(millis: number): string {
  const d = new Date(millis);
  return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
}
