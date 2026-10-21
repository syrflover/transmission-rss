import { api, timedRead } from "@/lib/api";
import { sendCommand, type Command } from "@/lib/commands";
import type {
  ArchiveDirection,
  ArchivedWorkAnswer,
  ChannelBrief,
  Preview,
  Rule,
  RuleCreateBody,
  RuleEpisodeBody,
  RuleFieldsBody,
  RuleGroup,
  RuleList,
  RuleOrderBody,
  RuleOrderEntry,
  RulePreviewBody,
  RuleRemoved,
  RuleState,
  RuleSwitchBody,
  RuleUpdateBody,
} from "@/lib/api-types";

import { archivedWorkPath, type ArchivedWork } from "./archivedWork";

/**
 * The rules API (`src/web/rules_api.rs`). Every write carries the version the
 * screen saw; a stale one answers `conflict` with the server's current value.
 */

// The answer's shape lives with the notice it feeds, which `node --test` runs without `@/` imports.
export type { ArchivedWork };

/**
 * The work a rule saving to `directory` (as typed) would bring out of the
 * archive folder; `null` when its work folder is not a work there, or, with
 * `from` (the folder a stored rule has now), when the rule stays in its work
 * folder. Asks nothing while no folder is typed.
 */
export async function getArchivedWork(
  directory: string,
  from: string | null,
  signal?: AbortSignal,
): Promise<ArchivedWork | null> {
  const path = archivedWorkPath(directory, from);
  if (path === null) return null;
  const { archived } = await api<ArchivedWorkAnswer>(path, { signal });
  return archived;
}

/** The fields the detail edits. */
export interface RuleFields {
  match: string;
  regex: boolean;
  case_insensitive: boolean;
  directory: string;
  episode: number;
  state: RuleState;
}

/** What a channel is called on screen: its name, or the host when it has none. */
export function channelName(channel: Pick<ChannelBrief, "name" | "host">): string {
  return channel.name ?? channel.host;
}

/** What a rule is called on screen. */
export function ruleTitle(rule: Pick<Rule, "match">): string {
  return rule.match === null || rule.match === "" ? "제목 대기" : rule.match;
}

const body = (fields: RuleFields): RuleFieldsBody => ({
  match: fields.match === "" ? null : fields.match,
  regex: fields.regex,
  case_insensitive: fields.case_insensitive,
  directory: fields.directory,
  episode: fields.episode,
  state: fields.state,
});

export function listRules(): Promise<RuleList> {
  return api<RuleList>("/rules");
}

export function getRule(id: string): Promise<Rule> {
  return api<Rule>(`/rules/${encodeURIComponent(id)}`, timedRead());
}

/**
 * Asks the worker to archive or restore the rule. The rule's state is not
 * saved with the other fields: the worker turns it off before the folder moves
 * and on after it moved back.
 */
export function sendArchive(id: string, ruleId: string, direction: ArchiveDirection): Promise<Command> {
  return sendCommand(id, "rule_archive", { rule_id: ruleId, direction });
}

/** `되돌리기` of the app's offset `episode`, as the screen showed it. */
export function sendEpisodeUndo(id: string, ruleId: string, episode: number): Promise<Command> {
  return sendCommand(id, "episode_undo", { rule_id: ruleId, episode });
}

export function createRule(channelId: string, fields: RuleFields): Promise<Rule> {
  const create: RuleCreateBody = { channel_id: channelId, ...body(fields) };
  return api<Rule>("/rules", { method: "POST", body: create });
}

export function saveRule(rule: Rule, fields: RuleFields): Promise<Rule> {
  const update: RuleUpdateBody = { version: rule.version, channel_id: rule.channel_id, ...body(fields) };
  return api<Rule>(`/rules/${encodeURIComponent(rule.id)}`, { method: "PUT", body: update });
}

/**
 * The rule detail's switches, applied at once: `video` is `영상 받기` (off pauses
 * the rule) and `subtitles` is `자막 받기` of a subscription. Send one.
 */
export function switchRule(rule: Rule, change: { video: boolean } | { subtitles: boolean }): Promise<Rule> {
  const switched: RuleSwitchBody = { version: rule.version, ...change };
  return api<Rule>(`/rules/${encodeURIComponent(rule.id)}/switch`, { method: "PUT", body: switched });
}

/** `적용` of the app's episode suggestion: the offset becomes the user's own, at once. */
export function applyEpisode(rule: Rule, episode: number): Promise<Rule> {
  const applied: RuleEpisodeBody = { version: rule.version, episode };
  return api<Rule>(`/rules/${encodeURIComponent(rule.id)}/episode`, { method: "PUT", body: applied });
}

export async function deleteRule(rule: Rule): Promise<void> {
  await api<RuleRemoved>(`/rules/${encodeURIComponent(rule.id)}?version=${rule.version}`, {
    method: "DELETE",
  });
}

/** `order` lists every rule of the channel once, in the wanted order, with the version seen. */
export async function reorderRules(
  channelId: string,
  order: RuleOrderEntry[],
): Promise<Rule[]> {
  const sent: RuleOrderBody = { channel_id: channelId, order };
  const { rules } = await api<RuleGroup>("/rules/order", { method: "PUT", body: sent });
  return rules;
}

/**
 * `position` is the edited rule's place among the channel's rules, from 0. `subscribing` asks, for a rule not saved
 * yet, what the app offers its offset before anything is received.
 */
export function previewRule(
  channelId: string,
  ruleId: string | null,
  fields: RuleFields,
  position: number,
  subscribing: boolean,
  signal?: AbortSignal,
): Promise<Preview> {
  const asked: RulePreviewBody = {
    channel_id: channelId,
    rule_id: ruleId,
    rule: body(fields),
    position,
    subscribing,
  };
  return api<Preview>("/rules/preview", { method: "POST", signal, body: asked });
}
