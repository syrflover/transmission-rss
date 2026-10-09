/**
 * The date a stored copy was received, as the 파일 card and the 자막 card show it. The two cards group copies
 * differently (the 파일 card by episode, the 자막 card by creator, season, episode and format), so each hands its own
 * grouping to `carriesTime`; the day and the clock are the viewer's, which the server cannot know. Imports nothing, so it
 * runs under `node --test`.
 */

const pad = (n: number) => String(n).padStart(2, "0");

/** The viewer's calendar day of a moment, as a key that is the same for every moment of that day. */
function dayKey(at: number): string {
  const d = new Date(at);
  return `${d.getFullYear()}-${d.getMonth()}-${d.getDate()}`;
}

/** Whether two moments are on the same calendar day of the viewer. */
export function sameDay(a: number, b: number): boolean {
  return dayKey(a) === dayKey(b);
}

/**
 * Whether the date of `entry` carries its time: another of the entries shown with it (`shown`, which may include
 * `entry` itself) is the same copy by the card's grouping (`sameCopy`) and was received the same day
 * (docs/specs/subtitles.md, 지난 수정본).
 */
export function carriesTime<T extends { id: string; stored_at: number }>(
  entry: T,
  shown: readonly T[],
  sameCopy: (a: T, b: T) => boolean,
): boolean {
  return shown.some((other) => other.id !== entry.id && sameCopy(other, entry) && sameDay(other.stored_at, entry.stored_at));
}

/** `9월 7일 받음`; `9월 7일 13:05 받음` when `withTime`, so two copies received on one day can be told apart. */
export function receivedAt(storedAt: number, withTime: boolean): string {
  const d = new Date(storedAt);
  const day = `${d.getMonth() + 1}월 ${d.getDate()}일`;
  return withTime ? `${day} ${pad(d.getHours())}:${pad(d.getMinutes())} 받음` : `${day} 받음`;
}
