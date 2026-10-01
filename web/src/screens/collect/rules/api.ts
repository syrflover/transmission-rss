import { api } from "@/lib/api";
import { sendCommand, type Command } from "@/lib/commands";

import type { SubscriptionBrief } from "../subs/api";

/**
 * The rules API (`src/web/rules_api.rs`). Every write carries the version the
 * screen saw; a stale one answers `conflict` with the server's current value.
 */

export interface RegexProblem {
  /** A full sentence for the screen. */
  message: string;
  /** The regex library's own reason. */
  detail: string;
}

export type RuleState = "active" | "archived";

/** `보관` (`archive`) or `복원` (`restore`) of a rule: the `rule_archive` command. */
export type ArchiveDirection = "archive" | "restore";

/**
 * The last archive or restore of a rule and where it is. The outcome's
 * `result` is `moved` (the work folder is where the command puts it), `kept`
 * (the rule changed state and the folder stayed, for the `reason`) or `failed`.
 */
export interface ArchiveMove {
  direction: ArchiveDirection;
  command: Command;
}

export interface Rule {
  id: string;
  channel_id: string;
  version: number;
  /** Place among the channel's rules from 1: the order the worker checks in. */
  order: number;
  /** `null` while the rule waits for its title. */
  match: string | null;
  regex: boolean;
  case_insensitive: boolean;
  /** Relative to the app's collect folder. */
  directory: string;
  episode: number;
  episode_auto: boolean;
  state: RuleState;
  /** An earlier rule takes an item this rule also matches. */
  overlap: boolean;
  error: RegexProblem | null;
  last_received_at: number | null;
  /** The last `보관`·`복원` of the rule, open or ended; `null` when it never had one. */
  archive_move: ArchiveMove | null;
  /** Set when the rule follows an anime of Anissia's schedule. */
  subscription: SubscriptionBrief | null;
}

export interface ChannelBrief {
  id: string;
  position: number;
  name: string | null;
  host: string;
  rule_count: number;
}

export interface RuleList {
  rules: Rule[];
  channels: ChannelBrief[];
  /** The app's collect folder; `null` until one is chosen in the settings. */
  collect_folder: string | null;
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

export type PreviewKind = "mine" | "earlier" | "excluded";

export interface PreviewItem {
  id: number;
  title: string;
  first_seen_at: number;
  /** The stored title contains a masked secret value, so the worker may judge it differently. */
  masked: boolean;
  kind: PreviewKind;
  save_path: string | null;
  taken_by: { rule_id: string | null; match: string | null } | null;
  excluded_by: string | null;
  stored_result: string;
}

export interface Preview {
  error: RegexProblem | null;
  counts: { total: number; mine: number; earlier: number; excluded: number; unmatched: number };
  masked_total: number;
  items: PreviewItem[];
  truncated: boolean;
}

/** What a channel is called on screen: its name, or the host when it has none. */
export function channelName(channel: Pick<ChannelBrief, "name" | "host">): string {
  return channel.name ?? channel.host;
}

/** What a rule is called on screen. */
export function ruleTitle(rule: Pick<Rule, "match">): string {
  return rule.match === null || rule.match === "" ? "제목 대기" : rule.match;
}

const body = (fields: RuleFields) => ({
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
  return api<Rule>(`/rules/${encodeURIComponent(id)}`);
}

/**
 * Asks the worker to archive or restore the rule. The rule's state is not
 * saved with the other fields: the worker turns it off before the folder moves
 * and on after it moved back.
 */
export function sendArchive(id: string, ruleId: string, direction: ArchiveDirection): Promise<Command> {
  return sendCommand(id, "rule_archive", { rule_id: ruleId, direction });
}

export function createRule(channelId: string, fields: RuleFields): Promise<Rule> {
  return api<Rule>("/rules", { method: "POST", body: { channel_id: channelId, ...body(fields) } });
}

export function saveRule(rule: Rule, fields: RuleFields): Promise<Rule> {
  return api<Rule>(`/rules/${encodeURIComponent(rule.id)}`, {
    method: "PUT",
    body: { version: rule.version, channel_id: rule.channel_id, ...body(fields) },
  });
}

export async function deleteRule(rule: Rule): Promise<void> {
  await api<{ removed: boolean }>(`/rules/${encodeURIComponent(rule.id)}?version=${rule.version}`, {
    method: "DELETE",
  });
}

/** `order` lists every rule of the channel once, in the wanted order, with the version seen. */
export async function reorderRules(
  channelId: string,
  order: { id: string; version: number }[],
): Promise<Rule[]> {
  const { rules } = await api<{ rules: Rule[] }>("/rules/order", {
    method: "PUT",
    body: { channel_id: channelId, order },
  });
  return rules;
}

/** `position` is the edited rule's place among the channel's rules, from 0. */
export function previewRule(
  channelId: string,
  ruleId: string | null,
  fields: RuleFields,
  position: number,
  signal?: AbortSignal,
): Promise<Preview> {
  return api<Preview>("/rules/preview", {
    method: "POST",
    signal,
    body: { channel_id: channelId, rule_id: ruleId ?? undefined, rule: body(fields), position },
  });
}
