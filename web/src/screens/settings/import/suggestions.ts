import type { ChannelView, Decision, PickRequest, RuleView, SuggestionKind } from "./types";

/**
 * Where a subscription suggestion stands in the review:
 * - `pickable`: the user can check it;
 * - `none`: there is nothing to check (no comment, or one that could not be read);
 * - `skipped`: the channel is skipped, so its suggestions go with it;
 * - `not_imported`: the channel is not imported;
 * - `blocked`: the comment was read but the rule cannot become a subscription;
 * - `kept`: replacing keeps a rule that is a subscription already;
 * - `unlisted`: Anissia answered and does not list the anime, so only the rule is imported.
 */
export type Standing = "pickable" | "none" | "skipped" | "not_imported" | "blocked" | "kept" | "unlisted";

/** Why a suggestion for an anime Anissia does not list cannot be checked. */
export const UNLISTED_NOTE = "Anissia 편성표에 없는 작품이라서 규칙만 가져와요.";

export function standing(
  channel: ChannelView,
  rule: RuleView,
  decision: Decision | undefined,
  unlisted: ReadonlySet<number>,
): Standing {
  const suggestion = rule.suggestion;
  if (suggestion.anime_no === null) return "none";
  if (channel.not_imported !== null) return "not_imported";
  if (decision === "skip") return "skipped";
  if (suggestion.blocked !== null) return "blocked";
  if (decision === "replace" && suggestion.keeps_subscription) return "kept";
  if (unlisted.has(suggestion.anime_no)) return "unlisted";
  return "pickable";
}

export const pickKey = (channel: number, rule: number) => `${channel}:${rule}`;

/** What the user changed from the suggestions' first state, by `pickKey`. */
export type Picks = Record<string, boolean>;

/** Whether the suggestion is checked now: the user's mark, else how the review starts. */
export function isPicked(
  channel: ChannelView,
  rule: RuleView,
  index: number,
  picks: Picks,
  decision: Decision | undefined,
  unlisted: ReadonlySet<number>,
): boolean {
  if (standing(channel, rule, decision, unlisted) !== "pickable") return false;
  return picks[pickKey(channel.index, index)] ?? rule.suggestion.checked;
}

/** The checked suggestions of the channels that are imported, as the apply takes them. */
export function pickedRequests(
  channels: ChannelView[],
  picks: Picks,
  choices: Record<number, Decision>,
  unlisted: ReadonlySet<number>,
): PickRequest[] {
  return channels.flatMap((channel) =>
    channel.rules.flatMap((rule, index) =>
      isPicked(channel, rule, index, picks, choices[channel.index], unlisted)
        ? [{ channel: channel.index, rule: index }]
        : [],
    ),
  );
}

/** How many rules of a channel fall in each of the spec's four cases. */
export function caseCounts(channel: ChannelView): Record<SuggestionKind, number> {
  const counts: Record<SuggestionKind, number> = { with_creator: 0, address_only: 0, unreadable: 0, none: 0 };
  for (const rule of channel.rules) counts[rule.suggestion.kind] += 1;
  return counts;
}

const WEEKDAYS = ["일", "월", "화", "수", "목", "금", "토"];

/** Where an anime sits in the week: `수 22:30`, `기타`, `신작`. */
export function weekLabel(week: number, airTime: string | null): string {
  if (week === 7) return "기타";
  if (week === 8) return "신작";
  const day = `${WEEKDAYS[week] ?? "?"}요일`;
  return airTime ? `${day} ${airTime}` : day;
}
