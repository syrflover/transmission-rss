import { api } from "@/lib/api";
import { sendCommand, type Command } from "@/lib/commands";

import type { Rule } from "../rules/api";

/**
 * The subscription API (`src/web/subscriptions_api.rs`): Anissia's schedule,
 * the subscribed rules, and subscribing. An Anissia call that fails throws an
 * `ApiError` with code `unavailable` and a sentence naming the reason.
 */

/** How a subscription gets subtitles: follow a creator, creator undecided, or none. */
export type SubtitleMode = "follow" | "undecided" | "none";

/** The schedule snapshot of an anime, as Anissia last listed it. */
export interface Anime {
  anime_no: number;
  subject: string;
  original_subject: string | null;
  /** 0 (Sunday) to 6 (Saturday), 7 (`기타`) or 8 (`신작`). */
  week: number;
  /** `HH:MM`, Asia/Seoul. */
  air_time: string | null;
  /** `YYYY-MM-DD`, or `YYYY-MM` when only the month is known. */
  start_date: string | null;
  end_date: string | null;
  status: string;
  fetched_at: number;
}

export interface SubscriptionBrief {
  anissia_anime_no: number;
  subtitles: SubtitleMode;
  /** The creator followed; none for `undecided`. */
  creator: string | null;
  season_id: string | null;
  subscribed_at: number;
  anime: Anime | null;
  /** The quarter the anime started in (or the subscription began in). */
  quarter: Quarter;
}

export interface SubscribedRule {
  rule_id: string;
  channel_id: string;
}

export interface ScheduleEntry {
  anime_no: number;
  week: number;
  subject: string;
  original_subject: string | null;
  air_time: string | null;
  start_date: string | null;
  end_date: string | null;
  status: string;
  genres: string[];
  caption_count: number;
  /** The rules that already follow this anime, in any channel. */
  subscribed_rules: SubscribedRule[];
}

export interface Schedule {
  week: number;
  entries: ScheduleEntry[];
  fetched_at: number;
  cached: boolean;
}

export interface Creator {
  name: string;
  captions: number;
  last_updated_at: string | null;
}

export interface Creators {
  anime_no: number;
  creators: Creator[];
  fetched_at: number;
  cached: boolean;
}

export interface Quarter {
  year: number;
  number: number;
}

export interface SubscriptionItem {
  rule_id: string;
  /** `paused` while `영상 받기` is off. */
  state: "active" | "paused";
  rule_version: number;
  channel_id: string;
  channel_name: string | null;
  channel_host: string;
  title: string | null;
  directory: string;
  subscription: SubscriptionBrief;
  quarter: Quarter;
  /** The anime's quarter has not begun yet. */
  upcoming: boolean;
}

export interface SubscriptionList {
  quarter: Quarter;
  subscriptions: SubscriptionItem[];
}

export interface TitleGroup {
  /** The work, as the newest item writes it: the rule's match phrase. */
  work: string;
  latest_title: string;
  /** How many recorded items the work has. */
  items: number;
  latest_seen_at: number;
  /** The save folder suggested for it. */
  folder: string | null;
}

export interface Titles {
  /** How many items the channel's history holds. */
  recorded_items: number;
  /** How many works match the filter. */
  total: number;
  truncated: boolean;
  titles: TitleGroup[];
}

export function fetchSchedule(week: number, signal?: AbortSignal): Promise<Schedule> {
  return api<Schedule>(`/anissia/schedule/${week}`, { signal });
}

export function fetchCreators(animeNo: number, signal?: AbortSignal): Promise<Creators> {
  return api<Creators>(`/anissia/anime/${animeNo}/creators`, { signal });
}

export function listSubscriptions(signal?: AbortSignal): Promise<SubscriptionList> {
  return api<SubscriptionList>("/subscriptions", { signal });
}

export function fetchTitles(channelId: string, query: string, signal?: AbortSignal): Promise<Titles> {
  const params = new URLSearchParams({ channel_id: channelId, q: query });
  return api<Titles>(`/subscriptions/titles?${params}`, { signal });
}

export interface NewSubscription {
  channel_id: string;
  anissia_anime_no: number;
  /** The schedule week the anime was picked from. */
  week: number;
  /** The release title's work; `null` subscribes before the first episode, waiting for its title. */
  work: string | null;
  subtitles: SubtitleMode;
  creator: string | null;
  directory: string;
}

export async function subscribe(body: NewSubscription): Promise<Rule> {
  const { rule } = await api<{ rule: Rule }>("/subscriptions", { method: "POST", body });
  return rule;
}

/** Changes the creator a subscription follows; `null` is `제작자 미정`. */
export function changeCreator(rule: Pick<Rule, "id" | "version">, creator: string | null): Promise<Rule> {
  return api<Rule>(`/rules/${encodeURIComponent(rule.id)}/creator`, {
    method: "PUT",
    body: { version: rule.version, creator },
  });
}

export interface ScheduleLink {
  anissia_anime_no: number;
  /** The schedule week the anime was picked from. */
  week: number;
  subtitles: SubtitleMode;
  creator: string | null;
}

/** `편성표와 연결`: turns an existing rule into a subscription, keeping its phrase, folder and order. */
export function linkToSchedule(rule: Pick<Rule, "id" | "version">, link: ScheduleLink): Promise<Rule> {
  return api<Rule>(`/rules/${encodeURIComponent(rule.id)}/subscription`, {
    method: "POST",
    body: { version: rule.version, ...link },
  });
}

/**
 * Asks the worker to receive a past item with the new rule: the item is one the
 * rule would pick that no rule has received.
 */
export function receiveWithRule(id: string, itemId: number, ruleId: string): Promise<Command> {
  return sendCommand(id, "receive_once", { item_id: itemId, rule_id: ruleId });
}

/** A subscription that waits for its title, as a work is offered to it. */
export interface WaitingSub {
  rule_id: string;
  rule_version: number;
  /** The anime it follows; only a hint for which work belongs to it. */
  anime: Anime | null;
  /** The save folder it has now. */
  directory: string;
}

/** A work that appeared in a channel with a subscription waiting for a title. */
export interface TitleCandidate {
  channel_id: string;
  channel_name: string | null;
  channel_host: string;
  /** What identifies the candidate within its channel. */
  key: string;
  /** The work, as the newest item writes it: the match phrase it would give. */
  work: string;
  latest_title: string;
  items: number;
  first_seen_at: number;
  latest_seen_at: number;
  /** The save folder made from the work, to offer in place of the folder the subscription has. */
  folder: string | null;
  waiting: WaitingSub[];
}

export function fetchCandidates(signal?: AbortSignal): Promise<TitleCandidate[]> {
  return api<{ candidates: TitleCandidate[] }>("/subscriptions/candidates", { signal }).then((r) => r.candidates);
}

/** `거절`: the work is not offered again in this channel. */
export async function rejectCandidate(channelId: string, work: string): Promise<void> {
  await api<{ rejected: boolean }>("/subscriptions/candidates/reject", {
    method: "POST",
    body: { channel_id: channelId, work },
  });
}

/**
 * Names the title of a waiting subscription: the work becomes its match phrase,
 * and `directory` (when given) replaces its save folder. Receives nothing.
 */
export function nameTitle(
  rule: Pick<WaitingSub, "rule_id" | "rule_version">,
  work: string,
  directory: string | null,
): Promise<Rule> {
  return api<Rule>(`/rules/${encodeURIComponent(rule.rule_id)}/title`, {
    method: "POST",
    body: { version: rule.rule_version, work, ...(directory === null ? {} : { directory }) },
  });
}
