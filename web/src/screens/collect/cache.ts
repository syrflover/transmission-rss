import { forget, forgetPrefix, patch } from "@/lib/cached";

import type { Channel } from "./channels/api";
import type { RuleList } from "./rules/api";

/**
 * The cache keys of the collection screens, and what a write on one screen
 * does to the copies other screens keep (see `lib/cached.ts`). The screens
 * keep their own copy true with `update`; what they cannot patch is dropped
 * here, so a screen opened later reads the server before it shows anything.
 */
export const KEYS = {
  status: "collect:status",
  /** The channel list (`/api/channels`): the 채널 tab, and the 기록 tab's channel filter. */
  channels: "collect:channels",
  /** Every rule with its channels (`/api/rules`). */
  rules: "collect:rules",
  /** The first page of the history under one filter (`filter` is `writeFilter`'s query string). */
  history: (filter: string) => `collect:history:${filter}`,
  /** The query string a tab had when it was left (the open rule, the history filter), for coming back to it. */
  search: (tab: string) => `collect:search:${tab}`,
  /** The collect and archive folders (`/api/settings/collection`), shown by the settings list and its panel. */
  collection: "settings:collection",
  /** How the rule list is sorted. */
  ruleSort: "collect:rule-sort",
} as const;

const HISTORY_PREFIX = "collect:history:";
/** The last preview of each rule (`usePreview`). */
const PREVIEW_PREFIX = "collect:preview:";

/**
 * A channel was added, edited or deleted. The rule list carries the channels'
 * names and folders (and a deleted channel's rules go with it), and history
 * rows carry the channel's name.
 */
export function channelsChanged(): void {
  forget(KEYS.rules);
  forgetPrefix(HISTORY_PREFIX);
}

/**
 * The collect folder was set or changed. The rule list and the rule previews
 * carry full save paths, and the status board says whether a folder is set.
 */
export function collectFolderChanged(): void {
  forget(KEYS.rules);
  forget(KEYS.status);
  forgetPrefix(PREVIEW_PREFIX);
}

/** Channels were added or changed by a path that cannot say how (the legacy import). */
export function everythingChanged(): void {
  forget(KEYS.channels);
  collectFolderChanged();
  channelsChanged();
}

/** A rule was created (`+1`) or deleted (`-1`): the channel list shows how many rules a channel has. */
export function ruleCountChanged(channelId: string, delta: number): void {
  patch<Channel[]>(KEYS.channels, (channels) =>
    channels.map((c) => (c.id === channelId ? { ...c, rule_count: Math.max(0, c.rule_count + delta) } : c)),
  );
}

/** The rule list as it is after the rule `ruleId` was deleted. */
export function withoutRule(list: RuleList, ruleId: string): RuleList {
  const gone = list.rules.find((r) => r.id === ruleId);
  return {
    ...list,
    rules: list.rules.filter((r) => r.id !== ruleId),
    channels: list.channels.map((c) =>
      gone && c.id === gone.channel_id ? { ...c, rule_count: Math.max(0, c.rule_count - 1) } : c,
    ),
  };
}
