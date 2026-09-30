import { useCallback, useEffect, useRef, useState } from "react";

import { ApiError } from "@/lib/api";

import {
  getCommand,
  getHistoryItem,
  newCommandId,
  sendReceiveOnce,
  type Command,
  type HistoryItem,
  type ReceiveOncePayload,
} from "./api";

/** How often a running command is asked for its state, in milliseconds. */
export const POLL_MS = 2000;

/**
 * Where one `한 번 받기` is. The parts that matter for the wording:
 *
 * - `waiting`: the server has the command (accepted), the worker has not
 *   finished it. This is not "received".
 * - `unconfirmed`: the answer to the request was lost, or the server cannot be
 *   reached, so it is not known whether the server has the command. That is
 *   different from `ended` with `failed`, where the worker tried and could not.
 * - `ended`: the worker wrote the outcome.
 */
export type ReceivePhase =
  | { kind: "idle"; error: string | null }
  | { kind: "sending" }
  | { kind: "waiting" }
  /** `lost`: the server answered that it has no such command, so sending it again with the same ID is safe. */
  | { kind: "unconfirmed"; lost: boolean }
  | { kind: "ended"; failed: boolean; message: string };

interface Attempt {
  id: string;
  /** `null` for a command found on the item at load: it is only followed, never resent. */
  payload: ReceiveOncePayload | null;
}

/** A finished command with nothing to explain: the row then shows what the item itself says. */
const FALLBACK_DONE = "";
const FALLBACK_FAILED = "받지 못했어요. 까닭은 알 수 없어요.";

/**
 * How an ended command reads. An item that ended `received` or `duplicate` is
 * in Transmission, even when this command's own add failed (a rule got it after
 * the request was accepted): that is never a failure. The worker ends such a
 * command as `done`; one it ended as `failed` before it did so carries the
 * add's error, which the row does not show next to `받음`.
 */
function endedMessage(command: Command): { failed: boolean; message: string } {
  const result = command.outcome?.result;
  const reason = command.outcome?.reason;
  if (result === "received" || result === "duplicate") {
    return { failed: false, message: command.state === "done" && reason ? reason : FALLBACK_DONE };
  }
  const failed = command.state === "failed" || result === "add_failed";
  if (reason) return { failed, message: reason };
  return { failed, message: failed ? FALLBACK_FAILED : FALLBACK_DONE };
}

/**
 * The `한 번 받기` flow of one history item, following the web command
 * contract (`src/store/commands`):
 *
 * - One ID per user action, made when the button is pressed and kept until the
 *   command ends or the server refuses the request. It is sent with the
 *   folder; the folder is fixed with the ID, so a resend is the same request.
 * - The answer to the request only says accepted. The screen then asks for the
 *   command by its ID every {@link POLL_MS} until the worker has ended it.
 * - When the answer is lost, the screen asks for the command by the same ID.
 *   Only a `404` (the server never stored it) allows sending it again, still
 *   with the same ID.
 * - When the command ends, the item is read again so the row shows what the
 *   worker wrote.
 */
export function useReceiveOnce(item: HistoryItem, onItem: (item: HistoryItem) => void) {
  const [phase, setPhase] = useState<ReceivePhase>({ kind: "idle", error: null });
  const attempt = useRef<Attempt | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const alive = useRef(true);
  const onItemRef = useRef(onItem);
  onItemRef.current = onItem;
  const itemId = item.id;

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
      const { failed, message } = endedMessage(command);
      try {
        const fresh = await getHistoryItem(itemId);
        if (alive.current) onItemRef.current(fresh);
      } catch {
        // The row keeps its old look; the outcome below still says what happened.
      }
      if (alive.current) setPhase({ kind: "ended", failed, message });
    },
    [itemId],
  );

  const follow = useCallback(
    (command: Command) => {
      if (!alive.current) return;
      if (command.state === "pending" || command.state === "running") {
        setPhase({ kind: "waiting" });
        later();
      } else {
        void settle(command);
      }
    },
    [settle],
  );

  /** Asks for the command by its ID: after a lost answer, and on every poll. */
  const lookup = useCallback(async () => {
    const current = attempt.current;
    if (!current) return;
    try {
      const command = await getCommand(current.id);
      if (attempt.current?.id === current.id) follow(command);
    } catch (e) {
      if (!alive.current || attempt.current?.id !== current.id) return;
      if (e instanceof ApiError && e.code === "not_found") {
        clearTimer();
        setPhase({ kind: "unconfirmed", lost: true });
      } else {
        // Cannot reach the server, or it failed: unknown, so keep asking.
        setPhase((was) => (was.kind === "waiting" ? was : { kind: "unconfirmed", lost: false }));
        later();
      }
    }
  }, [follow]);
  ask.current = () => void lookup();

  const send = useCallback(async () => {
    const current = attempt.current;
    if (!current?.payload) return;
    setPhase({ kind: "sending" });
    try {
      follow(await sendReceiveOnce(current.id, current.payload));
    } catch (e) {
      if (!alive.current) return;
      if (e instanceof ApiError && (e.code === "invalid" || e.code === "not_found")) {
        // Refused: nothing was stored, and the ID is not used again.
        attempt.current = null;
        setPhase({ kind: "idle", error: e.message });
      } else if (e instanceof ApiError && e.code === "conflict") {
        const open = e.current as Command | undefined;
        if (open && typeof open.id === "string" && open.id !== current.id && (open.state === "pending" || open.state === "running")) {
          // The item already has a command in progress: follow that one.
          attempt.current = { id: open.id, payload: null };
          follow(open);
        } else {
          attempt.current = null;
          setPhase({ kind: "idle", error: e.message });
        }
      } else {
        // The answer is lost or the server failed: look it up by the same ID.
        setPhase({ kind: "unconfirmed", lost: false });
        later();
      }
    }
  }, [follow]);

  /** The button was pressed: a new user action, so a new ID. */
  const submit = useCallback(
    (folder: string) => {
      if (attempt.current) return;
      attempt.current = { id: newCommandId(), payload: { item_id: itemId, folder } };
      void send();
    },
    [itemId, send],
  );

  /** Sends the same request again after the server said it never stored it. */
  const resend = useCallback(() => void send(), [send]);

  /** Asks again right away. */
  const recheck = useCallback(() => {
    clearTimer();
    void lookup();
  }, [lookup]);

  const dismiss = useCallback(() => setPhase({ kind: "idle", error: null }), []);

  // A command already in progress when the list loaded (or reloaded).
  const openCommand = item.command;
  useEffect(() => {
    alive.current = true;
    if (openCommand && !attempt.current) {
      attempt.current = { id: openCommand.id, payload: null };
      follow(openCommand);
    } else if (attempt.current) {
      // Mounted again (development double mount): the earlier timer was cleared.
      void lookup();
    }
    return () => {
      alive.current = false;
      clearTimer();
    };
    // Only on mount: later changes come from this hook's own state.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return { phase, submit, resend, recheck, dismiss };
}
