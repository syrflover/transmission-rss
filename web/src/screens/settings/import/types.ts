/** Shapes of `/api/import/legacy/*` (see src/web/import_api.rs). No secret value ever appears in them. */

/** What the comment above a rule offers (`src/web/import_api/suggestions.rs`). */
export type SuggestionKind = "with_creator" | "address_only" | "unreadable" | "none";

export interface SuggestionView {
  kind: SuggestionKind;
  /** Anissia's `animeNo` of the address that was read. */
  anime_no: number | null;
  /** The creator the comment names; null is `제작자 미정`. */
  creator: string | null;
  /**
   * The weekday (0 is Sunday) and `HH:MM` the comment gives. Shown only while
   * Anissia has not answered: Anissia's own values win.
   */
  comment_airs: { week: number; time: string } | null;
  /** Why an unreadable comment could not be read. */
  reason: string | null;
  /** Why a suggestion that was read cannot become a subscription. */
  blocked: string | null;
  /** Whether the review starts with it checked. */
  checked: boolean;
  /** Replacing keeps a rule that is a subscription already; it stays as it is. */
  keeps_subscription: boolean;
}

export interface RuleView {
  /** The match phrase; null is a rule waiting for its title. */
  match: string | null;
  regex: boolean;
  case_insensitive: boolean;
  directory: string;
  episode: number;
  /** A regex that cannot be compiled: it is imported but matches no title. */
  invalid_regex: boolean;
  /** Replacing keeps the ID of an existing rule for this one. */
  keeps_existing_rule: boolean;
  /**
   * The save folder that stays when the channel is replaced: the rule is a subscription and
   * the file's folder for it is no work folder. Null when the file's folder is used.
   */
  folder_kept: string | null;
  suggestion: SuggestionView;
}

export interface RemovedRule {
  match: string | null;
  directory: string;
}

export interface ExistingView {
  id: string;
  version: number;
  /** Masked. */
  url: string;
  rule_count: number;
  /** What a replacement would delete. */
  removed_rules: RemovedRule[];
  /** Title-waiting subscriptions of the channel, which a replacement leaves as they are. */
  title_waiting_kept: number;
}

export interface ChannelView {
  index: number;
  /** Masked: every query value is `***`. */
  url: string;
  /** The folder the file names for the channel, as written. */
  directory: string;
  /** Why the channel is not imported (its folder is outside the collect folder); null when it is. */
  not_imported: string | null;
  excludes: string[];
  rules: RuleView[];
  existing: ExistingView | null;
}

export interface CollectFolderView {
  /** The collect folder now; null when none is set. Sent back with the apply. */
  current: string | null;
  /** The folder this import sets because none is set yet. */
  will_set: string | null;
}

export interface Preview {
  channels: ChannelView[];
  conflict_count: number;
  collect_folder: CollectFolderView;
}

export type Decision = "replace" | "add" | "skip";

/** A suggestion the user kept checked: the channel's place in the file and the rule's in the channel. */
export interface PickRequest {
  channel: number;
  rule: number;
}

export interface ChoiceRequest {
  index: number;
  existing_id: string;
  existing_version: number;
  decision: Decision;
}

export interface ApplyResult {
  added: { index: number; id: string; url: string; rule_count: number }[];
  replaced: {
    index: number;
    id: string;
    url: string;
    rule_count: number;
    kept_rules: number;
    added_rules: number;
    removed_rules: RemovedRule[];
    /** Title-waiting subscriptions the replacement left as they were. */
    title_waiting_kept: number;
    /** Subscriptions whose save folder stayed because the file's folder is no work folder. */
    folders_kept: { rule: number; match: string | null; directory: string }[];
  }[];
  skipped: { index: number; url: string }[];
  not_imported: { index: number; url: string; reason: string }[];
  /** The collect folder this import set; null when it changed none. */
  collect_folder_set: string | null;
  subscriptions: {
    created: {
      channel: number;
      rule: number;
      anime_no: number;
      /** Anissia's title; null while the schedule is unknown. */
      subject: string | null;
      creator: string | null;
      /** False leaves the weekday and time for the app's daily re-read. */
      schedule_known: boolean;
      /** While the schedule is unknown: the subscription sits on the weekday and time the comment gave (`기타` when it gave none). */
      schedule_from_comment: boolean;
    }[];
    /** Checked suggestions that did not become subscriptions, with the reason. */
    not_created: { channel: number; rule: number; reason: string }[];
    /** Why Anissia could not be asked, if it could not. */
    unavailable: string | null;
  };
  counts: {
    channels_added: number;
    channels_replaced: number;
    channels_skipped: number;
    channels_not_imported: number;
    channels_unchanged: number;
    rules_added: number;
    rules_kept: number;
    rules_removed: number;
    title_waiting_kept: number;
    folders_kept: number;
    subscriptions_created: number;
  };
}
