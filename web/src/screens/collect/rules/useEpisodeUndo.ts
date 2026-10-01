import { useCallback, useEffect, useRef, useState } from "react";

import { ApiError } from "@/lib/api";
import { getCommand, isOpen, newCommandId, type Command } from "@/lib/commands";

import { getRule, sendEpisodeUndo, type EpisodeUndo, type Rule } from "./api";
import { POLL_MS } from "./useArchiveMove";

/**
 * Where one `되돌리기` is:
 *
 * - `waiting`: the server has the command, the worker has not ended it.
 *   `progress` is the undo as the rule last showed it (the files renamed so far).
 * - `unconfirmed`: it is not known whether the server has the command; `lost`
 *   when the server answered it has none, so sending it again is safe.
 * - `ended`: the command ended but the rule could not be read again.
 */
export type UndoPhase =
  | { kind: "idle"; error: string | null }
  | { kind: "sending" }
  | { kind: "waiting"; progress: EpisodeUndo | null }
  | { kind: "unconfirmed"; lost: boolean }
  | { kind: "ended"; command: Command };

interface Attempt {
  id: string;
  episode: number;
  /** `false` for a command found on the rule at load: it is only followed, never resent. */
  mine: boolean;
}

/**
 * The `되돌리기` flow of a rule's automatic offset, following the web command
 * contract as {@link useArchiveMove} does: one ID per press, kept until the
 * command ends or the server refuses it; the screen asks for the command by
 * that ID every {@link POLL_MS} and, while it is open, reads the rule for the
 * files renamed so far. When it ends, the rule is read again and handed to
 * `onRule`; the rule's own `episode_undo` then tells the outcome.
 */
export function useEpisodeUndo(rule: Rule, onRule: (fresh: Rule) => void) {
  const [phase, setPhase] = useState<UndoPhase>({ kind: "idle", error: null });
  const attempt = useRef<Attempt | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const alive = useRef(true);
  const onRuleRef = useRef(onRule);
  onRuleRef.current = onRule;
  const ruleId = rule.id;

  const clearTimer = () => {
    if (timer.current !== null) clearTimeout(timer.current);
    timer.current = null;
  };
  const ask = useRef<() => void>(() => {});
  const later = () => {
    clearTimer();
    timer.current = setTimeout(() => ask.current(), POLL_MS);
  };

  const settle = useCallback(
    async (command: Command) => {
      attempt.current = null;
      clearTimer();
      try {
        const fresh = await getRule(ruleId);
        if (!alive.current) return;
        onRuleRef.current(fresh);
        setPhase({ kind: "idle", error: null });
      } catch {
        if (alive.current) setPhase({ kind: "ended", command });
      }
    },
    [ruleId],
  );

  const follow = useCallback(
    async (command: Command) => {
      if (!alive.current) return;
      if (!isOpen(command)) {
        void settle(command);
        return;
      }
      setPhase((was) => ({ kind: "waiting", progress: was.kind === "waiting" ? was.progress : null }));
      later();
      try {
        const fresh = await getRule(ruleId);
        const undo = fresh.episode_undo;
        if (alive.current && undo && undo.command.id === attempt.current?.id) {
          setPhase((was) => (was.kind === "waiting" ? { kind: "waiting", progress: undo } : was));
        }
      } catch {
        // The progress waits for the next look.
      }
    },
    [settle, ruleId],
  );

  const lookup = useCallback(async () => {
    const current = attempt.current;
    if (!current) return;
    try {
      const command = await getCommand(current.id);
      if (attempt.current?.id === current.id) void follow(command);
    } catch (e) {
      if (!alive.current || attempt.current?.id !== current.id) return;
      if (e instanceof ApiError && e.code === "not_found") {
        clearTimer();
        setPhase({ kind: "unconfirmed", lost: true });
      } else {
        setPhase((was) => (was.kind === "waiting" ? was : { kind: "unconfirmed", lost: false }));
        later();
      }
    }
  }, [follow]);
  ask.current = () => void lookup();

  const send = useCallback(async () => {
    const current = attempt.current;
    if (!current?.mine) return;
    setPhase({ kind: "sending" });
    try {
      void follow(await sendEpisodeUndo(current.id, ruleId, current.episode));
    } catch (e) {
      if (!alive.current) return;
      if (e instanceof ApiError && (e.code === "invalid" || e.code === "not_found")) {
        // Refused: nothing was stored, and the ID is not used again.
        attempt.current = null;
        setPhase({ kind: "idle", error: e.message });
      } else if (e instanceof ApiError && e.code === "conflict") {
        const open = e.current as Command | undefined;
        if (open && typeof open.id === "string" && open.id !== current.id && isOpen(open)) {
          // Another tab is undoing it already: follow that one.
          attempt.current = { id: open.id, episode: current.episode, mine: false };
          void follow(open);
        } else {
          attempt.current = null;
          setPhase({ kind: "idle", error: e.message });
        }
      } else {
        setPhase({ kind: "unconfirmed", lost: false });
        later();
      }
    }
  }, [follow, ruleId]);

  /**
   * `되돌리기` was pressed: a new user action, so a new ID. `episode` is the
   * automatic value to undo: the rule's own, or for `이어서 되돌리기` the `from`
   * of an undo that stopped half done.
   */
  const submit = useCallback(
    (episode: number = rule.episode) => {
      if (attempt.current) return;
      attempt.current = { id: newCommandId(), episode, mine: true };
      void send();
    },
    [rule.episode, send],
  );

  /** Sends the same request again after the server said it never stored it. */
  const resend = useCallback(() => void send(), [send]);

  // An undo still open when the rule was loaded (or read again): follow it.
  const open = rule.episode_undo && isOpen(rule.episode_undo.command) ? rule.episode_undo : null;
  const openId = open?.command.id ?? null;
  useEffect(() => {
    alive.current = true;
    if (open && !attempt.current) {
      attempt.current = { id: open.command.id, episode: rule.episode, mine: false };
      void follow(open.command);
    } else if (attempt.current) {
      void lookup();
    }
    return () => {
      alive.current = false;
      clearTimer();
    };
    // Only on mount: later changes come from this hook's own state.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (open && !attempt.current) {
      attempt.current = { id: open.command.id, episode: rule.episode, mine: false };
      void follow(open.command);
    }
    // Only when an open undo appears on the rule; the effect above handles the mount.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [openId]);

  return { phase, submit, resend };
}
