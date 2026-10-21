import { useCallback, useEffect, useRef, useState } from "react";

import { ApiError } from "@/lib/api";
import { getCommand, isOpen, newCommandId, type Command } from "@/lib/commands";
import { useCommandWatch } from "@/lib/useCommandWatch";
import type { ArchiveDirection, ArchiveMove, Rule } from "@/lib/api-types";

import { getRule, sendArchive } from "./api";

/** How long to wait before reading again a command whose last read failed, in milliseconds. */
const RETRY_AFTER_MS = 2000;

/**
 * Where one `보관`·`복원` is:
 *
 * - `waiting`: the server has the command (accepted), the worker has not
 *   ended it. The folder may be moving; nothing is done yet.
 * - `unconfirmed`: the answer to the request was lost, or the server cannot be
 *   reached, so it is not known whether the server has the command. That is
 *   different from an ended command that failed.
 * - `ended`: the worker ended the command but the rule could not be read
 *   again; the outcome is shown from the command itself.
 */
export type MovePhase =
  | { kind: "idle"; error: string | null }
  | { kind: "sending"; direction: ArchiveDirection }
  | { kind: "waiting"; direction: ArchiveDirection }
  /** `lost`: the server answered that it has no such command, so sending it again with the same ID is safe. */
  | { kind: "unconfirmed"; direction: ArchiveDirection; lost: boolean }
  | { kind: "ended"; move: ArchiveMove };

interface Attempt {
  id: string;
  direction: ArchiveDirection;
  /** `false` for a command found on the rule at load: it is only followed, never resent. */
  mine: boolean;
}

/**
 * The `보관`·`복원` flow of one rule, following the web command contract
 * (`src/store/commands`) as `다시 받기` does:
 *
 * - One ID per user action, made when the button is pressed and kept until the
 *   command ends or the server refuses the request. A resend is the same request.
 * - The answer only says accepted. The screen then reads the command by its ID,
 *   now and whenever the server says it changed ({@link useCommandWatch}),
 *   until the worker has ended it.
 * - When the answer is lost, the screen asks for the command by the same ID.
 *   Only a `404` (the server never stored it) allows sending it again, still
 *   with the same ID.
 * - When the command ends, the rule is read again (its state and version
 *   changed on the server) and handed to `onRule`.
 *
 * The rule's state never changes on the screen before the worker changed it.
 */
export function useArchiveMove(rule: Rule | null, onRule: (fresh: Rule) => void) {
  const [phase, setPhase] = useState<MovePhase>({ kind: "idle", error: null });
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
        setPhase({ kind: "idle", error: null });
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

  /** Asks for the command by its ID: after a lost answer, and when the server says it changed. */
  const lookup = useCallback(async () => {
    const current = attempt.current;
    if (!current) return;
    try {
      const command = await getCommand(current.id);
      if (attempt.current?.id === current.id) follow(command, current.direction);
    } catch (e) {
      if (!alive.current || attempt.current?.id !== current.id) return;
      if (e instanceof ApiError && e.code === "not_found") {
        watching.stop();
        setPhase({ kind: "unconfirmed", direction: current.direction, lost: true });
      } else {
        // Cannot reach the server, or it failed: unknown, so keep asking.
        setPhase((was) =>
          was.kind === "waiting" ? was : { kind: "unconfirmed", direction: current.direction, lost: false },
        );
        later();
      }
    }
  }, [follow]);
  ask.current = lookup;

  const send = useCallback(async () => {
    const current = attempt.current;
    if (!current?.mine || ruleId === null) return;
    setPhase({ kind: "sending", direction: current.direction });
    try {
      follow(await sendArchive(current.id, ruleId, current.direction), current.direction);
    } catch (e) {
      if (!alive.current) return;
      if (e instanceof ApiError && (e.code === "invalid" || e.code === "not_found")) {
        // Refused: nothing was stored, and the ID is not used again.
        attempt.current = null;
        setPhase({ kind: "idle", error: e.message });
      } else if (e instanceof ApiError && e.code === "conflict") {
        const open = e.current as Command | undefined;
        if (open && typeof open.id === "string" && open.id !== current.id && isOpen(open)) {
          // The rule already has a command in progress (another tab): follow that one.
          attempt.current = { id: open.id, direction: current.direction, mine: false };
          follow(open, current.direction);
        } else {
          attempt.current = null;
          setPhase({ kind: "idle", error: e.message });
        }
      } else {
        // The answer is lost or the server failed: look it up by the same ID.
        setPhase({ kind: "unconfirmed", direction: current.direction, lost: false });
        later();
      }
    }
  }, [follow, ruleId]);

  /** A button was pressed: a new user action, so a new ID. */
  const submit = useCallback(
    (direction: ArchiveDirection) => {
      if (attempt.current || ruleId === null) return;
      attempt.current = { id: newCommandId(), direction, mine: true };
      void send();
    },
    [ruleId, send],
  );

  /** Sends the same request again after the server said it never stored it. */
  const resend = useCallback(() => void send(), [send]);

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
      attempt.current = { id: open.command.id, direction: open.direction, mine: false };
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
      attempt.current = { id: open.command.id, direction: open.direction, mine: false };
      follow(open.command, open.direction);
    }
    // Only when an open command appears on the rule; the effect above handles the mount.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [openId]);

  return { phase, submit, resend, recheck };
}
