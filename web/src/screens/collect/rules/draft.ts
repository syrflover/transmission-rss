import type { Rule, RuleFields } from "./api";

/** What the detail form holds: the fields as typed, before they are checked. */
export interface Draft {
  match: string;
  regex: boolean;
  case_insensitive: boolean;
  directory: string;
  /** Typed text; only a whole number can be saved. */
  episode: string;
  state: RuleFields["state"];
}

/** A rule that is not saved yet starts from these; the episode offset default is the store's. */
export const BLANK_DRAFT: Draft = {
  match: "",
  regex: false,
  case_insensitive: false,
  directory: "",
  episode: "1",
  state: "active",
};

export function draftOf(rule: Rule): Draft {
  return {
    match: rule.match ?? "",
    regex: rule.regex,
    case_insensitive: rule.case_insensitive,
    directory: rule.directory,
    episode: String(rule.episode),
    state: rule.state,
  };
}

/** The whole number typed for the episode offset, or `null` when it is not one. */
export function parseEpisode(text: string): number | null {
  const trimmed = text.trim();
  if (!/^-?\d{1,9}$/.test(trimmed)) return null;
  return Number(trimmed);
}

export function sameDraft(a: Draft, b: Draft): boolean {
  return (
    a.match === b.match &&
    a.regex === b.regex &&
    a.case_insensitive === b.case_insensitive &&
    a.directory.trim() === b.directory.trim() &&
    parseEpisode(a.episode) === parseEpisode(b.episode) &&
    a.episode.trim() === b.episode.trim()
    // The state is not typed: the switches and the worker change it, and the draft follows.
  );
}

/** The fields to send. An episode text that is not a number falls back to `fallbackEpisode`. */
export function fieldsOf(draft: Draft, fallbackEpisode: number): RuleFields {
  return {
    match: draft.match,
    regex: draft.regex,
    case_insensitive: draft.case_insensitive,
    directory: draft.directory,
    episode: parseEpisode(draft.episode) ?? fallbackEpisode,
    state: draft.state,
  };
}
