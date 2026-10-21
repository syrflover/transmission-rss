import { useCallback, useEffect, useRef, useState } from "react";

import { ApiError } from "@/lib/api";
import { getCommand, isOpen, newCommandId, type Command } from "@/lib/commands";
import { useCommandWatch } from "@/lib/useCommandWatch";
import type { EpisodeUndo, Rule } from "@/lib/api-types";

import { getRule, sendEpisodeUndo } from "./api";

/** How long to wait before reading again what a failed read left open, in milliseconds. */
const RETRY_AFTER_MS = 2000;

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
 * command ends or the server refuses it; the screen reads the command by that
 * ID now and whenever the server says it changed ({@link useCommandWatch}; the
 * undo says so as it records each file) and, while it is open, reads the rule
 * for the files renamed so far. When it ends, the rule is read again and
 * handed to `onRule`; the rule's own `episode_undo` then tells the outcome.
 */
export function useEpisodeUndo(rule: Rule, onRule: (fresh: Rule) => void) {
  const [phase, setPhase] = useState<UndoPhase>({ kind: "idle", error: null });
  const attempt = useRef<Attempt | null>(null);
  const watching = useCommandWatch();
  const alive = useRef(true);
  const onRuleRef = useRef(onRule);
  onRuleRef.current = onRule;
  const ruleId = rule.id;

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
    async (command: Command) => {
      attempt.current = null;
      watching.stop();
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

  /**
   * Shows `command`, a command the server has, and follows it. The reading
   * itself, the command and the progress together, is the watched one
   * ({@link lookup}), so reads of it never overlap and a failed one is read
   * again.
   */
  const follow = useCallback(
    (command: Command) => {
      if (!alive.current) return;
      if (!isOpen(command)) {
        void settle(command);
        return;
      }
      setPhase((was) => ({ kind: "waiting", progress: was.kind === "waiting" ? was.progress : null }));
      watch();
    },
    [settle],
  );

  /** The watched read: the command, and while it is open the files renamed so far. */
  const lookup = useCallback(async () => {
    const current = attempt.current;
    if (!current) return;
    try {
      const command = await getCommand(current.id);
      if (!alive.current || attempt.current?.id !== current.id) return;
      if (!isOpen(command)) {
        await settle(command);
        return;
      }
      setPhase((was) => ({ kind: "waiting", progress: was.kind === "waiting" ? was.progress : null }));
      try {
        const fresh = await getRule(ruleId);
        const undo = fresh.episode_undo;
        if (alive.current && undo && undo.command.id === attempt.current?.id) {
          setPhase((was) => (was.kind === "waiting" ? { kind: "waiting", progress: undo } : was));
        }
      } catch {
        // The command is known; only the progress is missing, so look again soon.
        if (alive.current && attempt.current?.id === current.id) later();
      }
    } catch (e) {
      if (!alive.current || attempt.current?.id !== current.id) return;
      if (e instanceof ApiError && e.code === "not_found") {
        watching.stop();
        setPhase({ kind: "unconfirmed", lost: true });
      } else {
        setPhase((was) => (was.kind === "waiting" ? was : { kind: "unconfirmed", lost: false }));
        later();
      }
    }
  }, [settle, ruleId]);
  ask.current = lookup;

  const send = useCallback(async () => {
    const current = attempt.current;
    if (!current?.mine) return;
    setPhase({ kind: "sending" });
    try {
      follow(await sendEpisodeUndo(current.id, ruleId, current.episode));
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
          follow(open);
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
      follow(open.command);
    } else if (attempt.current) {
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
      attempt.current = { id: open.command.id, episode: rule.episode, mine: false };
      follow(open.command);
    }
    // Only when an open undo appears on the rule; the effect above handles the mount.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [openId]);

  return { phase, submit, resend };
}
