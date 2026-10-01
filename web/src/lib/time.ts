const MINUTE = 60_000;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;

/** "방금", "5분 전", "3시간 전", "2일 전", or the date for anything older than a month. */
export function ago(at: number, now: number = Date.now()): string {
  const diff = Math.max(0, now - at);
  if (diff < MINUTE) return "방금";
  if (diff < HOUR) return `${Math.floor(diff / MINUTE)}분 전`;
  if (diff < DAY) return `${Math.floor(diff / HOUR)}시간 전`;
  if (diff < 30 * DAY) return `${Math.floor(diff / DAY)}일 전`;
  return dateTime(at);
}

/**
 * Asia/Seoul is UTC+9 all year (no daylight saving), so its calendar is read
 * from the moment shifted by nine hours. The weekly schedule and the collection
 * times are Seoul's, whatever the viewer's time zone is.
 */
const SEOUL_OFFSET = 9 * HOUR;

function seoul(at: number) {
  const d = new Date(at + SEOUL_OFFSET);
  return {
    month: d.getUTCMonth() + 1,
    day: d.getUTCDate(),
    hh: String(d.getUTCHours()).padStart(2, "0"),
    mm: String(d.getUTCMinutes()).padStart(2, "0"),
  };
}

/** "13:40" in Asia/Seoul. */
export function clock(at: number): string {
  const { hh, mm } = seoul(at);
  return `${hh}:${mm}`;
}

/** The Seoul calendar day of a moment, as a number that grows by one a day. */
function dayNumber(at: number): number {
  return Math.floor((at + SEOUL_OFFSET) / DAY);
}

/**
 * "오늘 13:40", "내일 09:00", "어제 23:10", or "10월 2일 13:40" for any other
 * day, in Asia/Seoul (the days are Seoul's too, like the schedule's).
 */
export function when(at: number, now: number = Date.now()): string {
  const apart = dayNumber(at) - dayNumber(now);
  if (apart === 0) return `오늘 ${clock(at)}`;
  if (apart === 1) return `내일 ${clock(at)}`;
  if (apart === -1) return `어제 ${clock(at)}`;
  const { month, day } = seoul(at);
  return `${month}월 ${day}일 ${clock(at)}`;
}

/** "9월 30일 12:00" in the viewer's time zone. */
export function dateTime(at: number): string {
  const d = new Date(at);
  const hh = String(d.getHours()).padStart(2, "0");
  const mm = String(d.getMinutes()).padStart(2, "0");
  return `${d.getMonth() + 1}월 ${d.getDate()}일 ${hh}:${mm}`;
}
