import { forget, forgetPrefix, patch } from "@/lib/cached";
import { forgetLibrary, WORK_PREFIX } from "@/screens/library/api";
import { forgetWeek } from "@/screens/schedule/api";
import type { ArchiveSuggestion, ChannelList, RuleList } from "@/lib/api-types";


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
  /** The watch folders with their counts (`/api/library/watch-folders`), shown by the settings list and its panel. */
  watchFolders: "settings:watch-folders",
  /** How the rule list is sorted. */
  ruleSort: "collect:rule-sort",
  /** The subscriptions of this and a coming quarter (`/api/subscriptions`). */
  subscriptions: "collect:subscriptions",
  /** The title candidates (`/api/subscriptions/candidates`). */
  candidates: "collect:candidates",
  /** The archive suggestions (`/api/archive-suggestions`). */
  archiveSuggestions: "collect:archive-suggestions",
  /** The works a save folder's work field offers (`/api/save-folder/works`). */
  folderWorks: "collect:folder-works",
  /** One week of Anissia's schedule (`/api/anissia/schedule/<week>`). */
  schedule: (week: number) => `collect:schedule:${week}`,
} as const;

const SCHEDULE_PREFIX = "collect:schedule:";

const HISTORY_PREFIX = "collect:history:";
/** The last preview of each rule (`usePreview`). */
const PREVIEW_PREFIX = "collect:preview:";

/**
 * A channel was added, edited or deleted. The rule list carries the channels'
 * names and folders (and a deleted channel's rules go with it), and history
 * rows carry the channel's name.
 */
export function channelsChanged(): void {
  // The home screen's checklist ends with the first channel.
  forgetWeek();
  forget(KEYS.rules);
  forgetPrefix(HISTORY_PREFIX);
}

/**
 * The collect folder or the archive folder was set or changed. The rule list
 * and the rule previews carry full save paths, the status board says whether a
 * folder is set, and the two folders are watch folders that follow the setting
 * (so the watch folder list and the works found in them change too).
 */
export function collectFolderChanged(): void {
  forget(KEYS.rules);
  forget(KEYS.status);
  forget(KEYS.watchFolders);
  forgetLibrary();
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
  forgetWeek();
  patch<ChannelList>(KEYS.channels, (list) => ({
    ...list,
    channels: list.channels.map((c) => (c.id === channelId ? { ...c, rule_count: Math.max(0, c.rule_count + delta) } : c)),
  }));
}

/**
 * Rules were deleted by a screen that is not the rule list (a work moved into the archive folder from its page): the
 * lists that show rules or count them, the subscriptions, the archive suggestions and the schedule (which says who
 * follows each anime) read the server again.
 */
export function rulesDeleted(): void {
  forgetWeek();
  forget(KEYS.rules);
  forget(KEYS.channels);
  forget(KEYS.subscriptions);
  forget(KEYS.archiveSuggestions);
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

/**
 * A subscription changed (its creator or its season, `영상 받기`·`자막 받기`):
 * the subscription list and the work pages, which show the creator in their
 * head, read the server again. The caller patches the rule list itself.
 */
export function subscriptionChanged(): void {
  forgetWeek();
  forget(KEYS.subscriptions);
  forget(KEYS.candidates);
  forgetPrefix(WORK_PREFIX);
}

/**
 * A rule was made by subscribing: the lists that show rules, the subscriptions
 * and the schedule (which says who follows each anime) read the server again.
 */
export function subscriptionAdded(channelId: string): void {
  subscriptionChanged();
  forget(KEYS.rules);
  forgetPrefix(SCHEDULE_PREFIX);
  ruleCountChanged(channelId, 1);
}

/**
 * A title candidate was named or rejected: the candidates, the subscription
 * list and the rule list (a waiting rule got its phrase) read the server again,
 * and so do the history rows, which offer a waiting subscription only while one
 * is left.
 */
export function candidatesChanged(): void {
  subscriptionChanged();
  forget(KEYS.rules);
  forgetPrefix(HISTORY_PREFIX);
  forgetPrefix(PREVIEW_PREFIX);
}

/** The rule `ruleId` is no longer suggested (archived, or `수집 유지`): the cached suggestions leave it out. */
export function suggestionGone(ruleId: string): void {
  patch<ArchiveSuggestion[]>(KEYS.archiveSuggestions, (list) => list.filter((s) => s.rule_id !== ruleId));
}

/**
 * Rules were archived, which deletes them and moves their work folders: the
 * lists that show or count rules, the works and their folders, the history
 * rows (which name the rule) and the previews read the server again. The
 * subscription list and the title candidates are left to the caller: the 구독
 * tab shows them at this very moment and reloads them in place, while the
 * rule tab drops them with {@link subscriptionChanged}.
 */
export function ruleArchived(): void {
  forgetLibrary();
  forget(KEYS.rules);
  forget(KEYS.channels);
  forgetPrefix(HISTORY_PREFIX);
  forgetPrefix(PREVIEW_PREFIX);
}
