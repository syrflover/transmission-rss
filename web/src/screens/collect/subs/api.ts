import { api } from "@/lib/api";
import { sendCommand, type Command } from "@/lib/commands";
import type {
  CandidateRejected,
  ChangeCreatorBody,
  Creators,
  LinkRuleBody,
  NameTitleBody,
  NewSubscription,
  RejectCandidateBody,
  Rule,
  Schedule,
  Subscribed,
  SubscriptionList,
  TitleCandidate,
  TitleCandidateList,
  Titles,
  WaitingSub,
} from "@/lib/api-types";

/**
 * The subscription API (`src/web/subscriptions_api.rs`): Anissia's schedule,
 * the subscribed rules, and subscribing. An Anissia call that fails throws an
 * `ApiError` with code `unavailable` and a sentence naming the reason.
 */

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

export async function subscribe(body: NewSubscription): Promise<Rule> {
  const { rule } = await api<Subscribed>("/subscriptions", { method: "POST", body });
  return rule;
}

/** Changes the creator a subscription follows; `null` is `제작자 미정`. */
export function changeCreator(rule: Pick<Rule, "id" | "version">, creator: string | null): Promise<Rule> {
  const body: ChangeCreatorBody = { version: rule.version, creator };
  return api<Rule>(`/rules/${encodeURIComponent(rule.id)}/creator`, { method: "PUT", body });
}

/** What `편성표와 연결` sends besides the rule's version. */
export type ScheduleLink = Omit<LinkRuleBody, "version">;

/** `편성표와 연결`: turns an existing rule into a subscription, keeping its phrase, folder and order. */
export function linkToSchedule(rule: Pick<Rule, "id" | "version">, link: ScheduleLink): Promise<Rule> {
  const body: LinkRuleBody = { version: rule.version, ...link };
  return api<Rule>(`/rules/${encodeURIComponent(rule.id)}/subscription`, { method: "POST", body });
}

/**
 * Asks the worker to receive a past item with the new rule: the item is one the
 * rule would pick that no rule has received.
 */
export function receiveWithRule(id: string, itemId: number, ruleId: string): Promise<Command> {
  return sendCommand(id, "receive_once", { item_id: itemId, rule_id: ruleId });
}

export function fetchCandidates(signal?: AbortSignal): Promise<TitleCandidate[]> {
  return api<TitleCandidateList>("/subscriptions/candidates", { signal }).then((r) => r.candidates);
}

/** `거절`: the work is not offered again in this channel. */
export async function rejectCandidate(channelId: string, work: string): Promise<void> {
  const body: RejectCandidateBody = { channel_id: channelId, work };
  await api<CandidateRejected>("/subscriptions/candidates/reject", { method: "POST", body });
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
  const body: NameTitleBody = { version: rule.rule_version, work, ...(directory === null ? {} : { directory }) };
  return api<Rule>(`/rules/${encodeURIComponent(rule.rule_id)}/title`, { method: "POST", body });
}
