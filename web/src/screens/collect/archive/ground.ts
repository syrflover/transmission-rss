import { dateTime } from "@/lib/time";

import { startDate } from "../subs/format";
import type { ArchiveSuggestion, Ground } from "./api";

/** The work a suggestion is for: the anime's name, else the rule's phrase. */
export function suggestionTitle(s: Pick<ArchiveSuggestion, "anime" | "title">): string {
  return s.anime?.subject ?? s.title;
}

/** Why a rule is suggested, as a sentence. */
export function groundText(ground: Ground, s: Pick<ArchiveSuggestion, "last_received_at">): string {
  switch (ground.kind) {
    case "ended": {
      const date = startDate(ground.end_date);
      return date ? `Anissia에서 방영이 끝났어요. 종영일은 ${date}이에요.` : "Anissia에서 방영이 끝났어요.";
    }
    case "unlisted":
      return "Anissia 편성표에서 빠졌어요. 방영이 끝난 것으로 봐요.";
    case "quiet": {
      const since = ground.since;
      if (since === null) return "4주 넘게 규칙에 맞는 새 항목이 없어요.";
      return s.last_received_at === since
        ? `${dateTime(since)}에 마지막으로 받은 뒤 4주 넘게 규칙에 맞는 새 항목이 없어요.`
        : `${dateTime(since)}부터 4주 넘게 규칙에 맞는 새 항목이 없어요.`;
    }
  }
}
