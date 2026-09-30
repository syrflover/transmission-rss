/** Shapes of `/api/import/legacy/*` (see src/web/import_api.rs). No secret value ever appears in them. */

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
}

export interface ChannelView {
  index: number;
  /** Masked: every query value is `***`. */
  url: string;
  directory: string;
  excludes: string[];
  rules: RuleView[];
  existing: ExistingView | null;
}

export interface Preview {
  channels: ChannelView[];
  conflict_count: number;
}

export type Decision = "replace" | "add" | "skip";

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
  }[];
  skipped: { index: number; url: string }[];
  counts: {
    channels_added: number;
    channels_replaced: number;
    channels_skipped: number;
    channels_unchanged: number;
    rules_added: number;
    rules_kept: number;
    rules_removed: number;
  };
}
