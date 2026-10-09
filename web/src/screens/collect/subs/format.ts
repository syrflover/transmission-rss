import { WEEKDAYS_LONG, WEEKDAYS_SHORT } from "@/lib/weekday";

import type { Anime, SubscriptionBrief } from "./api";

/** The tabs of the schedule: Sunday to Saturday, `기타` and `신작`, as Anissia groups them. */
export const WEEK_TABS: readonly string[] = [...WEEKDAYS_SHORT, "기타", "신작"];

export const WEEK_OTHER = 7;
export const WEEK_UPCOMING = 8;

/** The weekday (0 is Sunday) it is in Seoul now, which the schedule is in. */
export function todayWeek(now: Date = new Date()): number {
  const name = new Intl.DateTimeFormat("en-US", { timeZone: "Asia/Seoul", weekday: "short" }).format(now);
  const index = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"].indexOf(name);
  return index < 0 ? 0 : index;
}

/** `10월 7일`, `10월` when only the month is known, with the year when it is not this year's. */
export function startDate(date: string | null, now: Date = new Date()): string | null {
  if (date === null) return null;
  const match = /^(\d{4})-(\d{2})(?:-(\d{2}))?$/.exec(date);
  if (!match) return null;
  const year = Number(match[1]);
  const month = Number(match[2]);
  const day = match[3] === undefined ? null : Number(match[3]);
  const prefix = year === now.getFullYear() ? "" : `${year}년 `;
  return day === null ? `${prefix}${month}월` : `${prefix}${month}월 ${day}일`;
}

/** Where the anime sits in the week: `수요일 22:30`, `기타`, `신작`. */
export function airing(anime: Pick<Anime, "week" | "air_time">): string {
  if (anime.week >= 0 && anime.week <= 6) {
    return anime.air_time ? `${WEEKDAYS_LONG[anime.week]} ${anime.air_time}` : WEEKDAYS_LONG[anime.week];
  }
  return anime.week === WEEK_OTHER ? "기타" : "신작";
}

/** The subject particle after a name: `이` after a Hangul syllable with a final consonant, otherwise `가`. */
function subjectParticle(name: string): string {
  const last = name.trim().codePointAt(name.trim().length - 1) ?? 0;
  const isHangul = last >= 0xac00 && last <= 0xd7a3;
  return isHangul && (last - 0xac00) % 28 !== 0 ? "이" : "가";
}

/** The sentence a subtitle choice carries. */
export const SUBTITLE_NOTES = {
  follow: (creator: string) => `${creator}${subjectParticle(creator)} 새 회차 자막을 올리면 자동으로 받아요.`,
  undecided: "자막을 자동으로 받지 않아요. 자막 후보가 처음 생기면 할 일로 알려요.",
  none: "영상만 받아요. 자막 때문에 할 일을 만들지 않아요.",
} as const;

/** The creator's name, `제작자 미정` or `받지 않음`; the label beside it says it is the creator. */
export function subtitleChoice(subscription: Pick<SubscriptionBrief, "subtitles" | "creator">): string {
  switch (subscription.subtitles) {
    case "follow":
      return subscription.creator ?? "";
    case "undecided":
      return "제작자 미정";
    case "none":
      return "받지 않음";
  }
}
