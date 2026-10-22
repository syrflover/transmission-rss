import { useCallback, useEffect, useRef, useState } from "react";

import { ApiError } from "@/lib/api";
import { newCommandId } from "@/lib/commands";
import type { Rule, RuleArchived } from "@/lib/api-types";

import { archiveRule } from "./api";

/**
 * Where the rule detail's `보관` is:
 *
 * - `sending`: the request is on its way.
 * - `unconfirmed`: the answer was lost, so it is not known whether the server
 *   deleted the rule. Sending the same request again is safe: the server
 *   answers a stored one with its move.
 */
export type ArchivePhase = { kind: "idle"; error: string | null } | { kind: "sending" } | { kind: "unconfirmed" };

interface Attempt {
  id: string;
  rule: Pick<Rule, "id" | "version">;
  /** Sent before without an answer: a `404` now means the first one deleted the rule. */
  resent: boolean;
}

/** A conflict's `current` that is the rule as stored now (not the move that refused it). */
function isRule(value: unknown): value is Rule {
  return typeof value === "object" && value !== null && "channel_id" in value && "version" in value;
}

/**
 * The `보관` of one rule (`POST /rules/{id}/archive`), with one ID per user
 * action kept until the server answers, as the web command contract asks:
 *
 * - The answer is the deletion done: `onArchived` gets it (the move of the
 *   work folder, or why there is none) and the screen leaves the rule.
 * - A lost answer is sent again with the same ID. A `404` to that resend
 *   means the first request deleted the rule (one with no work folder leaves
 *   no move to find), so it is told as archived, with no more to say.
 * - A rule that changed meanwhile comes back through `onConflict`; a work
 *   folder already moving, or any other refusal, is said in `error`. Nothing
 *   was deleted then.
 */
export function useRuleArchive(
  rule: Rule | null,
  onArchived: (rule: Rule, result: RuleArchived) => void,
  onConflict: (current: Rule) => void,
) {
  const [phase, setPhase] = useState<ArchivePhase>({ kind: "idle", error: null });
  const attempt = useRef<Attempt | null>(null);
  const alive = useRef(true);
  const told = useRef({ onArchived, onConflict });
  told.current = { onArchived, onConflict };
  const ruleRef = useRef(rule);
  ruleRef.current = rule;

  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);

  const send = useCallback(async () => {
    const current = attempt.current;
    const shown = ruleRef.current;
    if (!current || !shown) return;
    setPhase({ kind: "sending" });
    try {
      const result = await archiveRule(current.id, current.rule);
      attempt.current = null;
      if (alive.current) setPhase({ kind: "idle", error: null });
      told.current.onArchived(shown, result);
    } catch (e) {
      if (!alive.current) return;
      if (e instanceof ApiError && e.code === "not_found" && current.resent) {
        attempt.current = null;
        setPhase({ kind: "idle", error: null });
        told.current.onArchived(shown, { command: null, reason: null, work_id: null });
      } else if (e instanceof ApiError && e.code === "conflict") {
        attempt.current = null;
        if (isRule(e.current)) told.current.onConflict(e.current);
        setPhase({ kind: "idle", error: e.message });
      } else if (e instanceof ApiError && (e.code === "invalid" || e.code === "not_found")) {
        // Refused: nothing was deleted, and the ID is not used again.
        attempt.current = null;
        setPhase({ kind: "idle", error: e.message });
      } else {
        // The answer is lost or the server failed: the same request again is safe.
        current.resent = true;
        setPhase({ kind: "unconfirmed" });
      }
    }
  }, []);

  /** `보관` was confirmed: a new user action, so a new ID, with the version the screen shows. */
  const submit = useCallback(() => {
    const shown = ruleRef.current;
    if (attempt.current || !shown) return;
    attempt.current = { id: newCommandId(), rule: { id: shown.id, version: shown.version }, resent: false };
    void send();
  }, [send]);

  /** Sends the same request again after its answer was lost. */
  const resend = useCallback(() => void send(), [send]);

  return { phase, submit, resend };
}
