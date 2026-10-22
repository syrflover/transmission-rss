import { useCallback, useEffect, useRef, useState } from "react";

import { ApiError } from "@/lib/api";
import { getCommand, isOpen, type Command } from "@/lib/commands";
import { useCommandWatch } from "@/lib/useCommandWatch";
import type { ArchiveDirection, ArchiveMove, Rule } from "@/lib/api-types";

import { getRule } from "./api";

/** How long to wait before reading again a command whose last read failed, in milliseconds. */
const RETRY_AFTER_MS = 2000;

/**
 * Where the move of a rule's work folder out of the archive folder is (the
 * `start` or `resume` the server stores when it saves a rule that collects
 * into a work of the archive folder):
 *
 * - `waiting`: the worker has not ended it. The folder may be moving; the rule
 *   collects nothing yet.
 * - `unconfirmed`: the server cannot be reached, so where it is is not known.
 *   That is different from an ended command that failed.
 * - `ended`: the worker ended the command but the rule could not be read
 *   again; the outcome is shown from the command itself.
 */
export type MovePhase =
  | { kind: "idle" }
  | { kind: "waiting"; direction: ArchiveDirection }
  | { kind: "unconfirmed"; direction: ArchiveDirection }
  | { kind: "ended"; move: ArchiveMove };

interface Attempt {
  id: string;
  direction: ArchiveDirection;
}

/**
 * Follows the open move of one rule's work folder (`archive_move` of the rule
 * as it was loaded or read again) as the web command contract asks: the screen
 * reads the command by its ID now and whenever the server says it changed
 * ({@link useCommandWatch}), until the worker has ended it. Then the rule is
 * read again (the worker turned it on) and handed to `onRule`.
 *
 * The rule's state never changes on the screen before the worker changed it.
 */
export function useArchiveMove(rule: Rule | null, onRule: (fresh: Rule) => void) {
  const [phase, setPhase] = useState<MovePhase>({ kind: "idle" });
  const attempt = useRef<Attempt | null>(null);
  const watching = useCommandWatch();
  const alive = useRef(true);
  const onRuleRef = useRef(onRule);
  onRuleRef.current = onRule;
  const ruleId = rule?.id ?? null;

  const ask = useRef<() => Promise<void>>(async () => {});

  /** Reads the attempt's command now and whenever the server says it changed. */
  const watch = () => {
    const id = attempt.current?.id;
    if (id) watching.watch(id, () => ask.current());
  };

  /** Reads it again soon: the last read did not get an answer. */
  const later = () => {
    const id = attempt.current?.id;
    if (id) watching.retry(id, () => ask.current(), RETRY_AFTER_MS);
  };

  const settle = useCallback(
    async (command: Command, direction: ArchiveDirection) => {
      attempt.current = null;
      watching.stop();
      if (ruleId === null) return;
      try {
        const fresh = await getRule(ruleId);
        if (!alive.current) return;
        onRuleRef.current(fresh);
        setPhase({ kind: "idle" });
      } catch {
        // The rule keeps its old look; the outcome still says what happened.
        if (alive.current) setPhase({ kind: "ended", move: { direction, command } });
      }
    },
    [ruleId],
  );

  const follow = useCallback(
    (command: Command, direction: ArchiveDirection) => {
      if (!alive.current) return;
      if (isOpen(command)) {
        setPhase({ kind: "waiting", direction });
        watch();
      } else {
        void settle(command, direction);
      }
    },
    [settle],
  );

  /** Asks for the command by its ID, when the server says it changed. */
  const lookup = useCallback(async () => {
    const current = attempt.current;
    if (!current) return;
    try {
      const command = await getCommand(current.id);
      if (attempt.current?.id === current.id) follow(command, current.direction);
    } catch (e) {
      if (!alive.current || attempt.current?.id !== current.id) return;
      if (e instanceof ApiError && e.code === "not_found") {
        // The server made it and keeps it; one it does not know is nothing to follow.
        attempt.current = null;
        watching.stop();
        setPhase({ kind: "idle" });
      } else {
        // Cannot reach the server, or it failed: unknown, so keep asking.
        setPhase((was) => (was.kind === "waiting" ? was : { kind: "unconfirmed", direction: current.direction }));
        later();
      }
    }
  }, [follow]);
  ask.current = lookup;

  /** Asks again right away. */
  const recheck = useCallback(() => {
    watching.stop();
    void lookup();
  }, [lookup]);

  // A command still open when the rule was loaded (or read again): follow it.
  const open = rule?.archive_move && isOpen(rule.archive_move.command) ? rule.archive_move : null;
  const openId = open?.command.id ?? null;
  useEffect(() => {
    alive.current = true;
    if (open && !attempt.current) {
      attempt.current = { id: open.command.id, direction: open.direction };
      follow(open.command, open.direction);
    } else if (attempt.current) {
      // Mounted again (development double mount): the earlier watch was stopped.
      void lookup();
    }
    return () => {
      alive.current = false;
      watching.stop();
    };
    // Only on mount: later changes come from this hook's own state.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (open && !attempt.current) {
      attempt.current = { id: open.command.id, direction: open.direction };
      follow(open.command, open.direction);
    }
    // Only when an open command appears on the rule; the effect above handles the mount.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [openId]);

  return { phase, recheck };
}
