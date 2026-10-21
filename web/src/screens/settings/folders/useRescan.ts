import { useCallback, useEffect, useRef, useState } from "react";

import { ApiError } from "@/lib/api";
import { getCommand, isOpen, newCommandId, sendCommand, type Command } from "@/lib/commands";
import { useCommandWatch } from "@/lib/useCommandWatch";

import { RESCAN_KIND } from "./api";

/** How long to wait before reading again a command whose last read failed, in milliseconds. */
const RETRY_AFTER_MS = 2000;

/**
 * Where one `다시 확인` is:
 *
 * - `waiting`: the server has the command; the worker has not ended it.
 * - `unconfirmed`: the answer was lost or the server cannot be reached, so it
 *   is not known whether the server has the command; it is asked for by its ID.
 * - `idle` with a `message`: the command ended `failed`, or the server refused it.
 */
export type RescanPhase =
  | { kind: "idle"; message: string | null }
  | { kind: "sending" }
  | { kind: "waiting" }
  | { kind: "unconfirmed" };

const FAILED = "다시 확인하지 못했어요.";

/**
 * The `다시 확인` flow of one folder, following the web command contract
 * (`src/store/commands`) as `다시 받기` and `보관` do:
 *
 * - One ID per press, kept until the command ends or the server refuses it.
 * - The answer only says accepted; the command is read by its ID, now and
 *   whenever the server says it changed ({@link useCommandWatch}), until the
 *   worker has ended it.
 * - After a lost answer the same ID is asked for, and only a `404` (never
 *   stored) allows sending it again, still with the same ID.
 * - When a command ends, `onEnded` runs (the list is read again) and the phase
 *   goes back to `idle`, with the reason when the command failed.
 */
export function useRescan(folderId: string, onEnded: () => void) {
  const [phase, setPhase] = useState<RescanPhase>({ kind: "idle", message: null });
  const [done, setDone] = useState(false);
  const running = useRef(false);
  const alive = useRef(true);
  const watching = useCommandWatch();
  /** Ends the wait of the press under way, so a press on a screen that is gone stops. */
  const wakeUp = useRef<(() => void) | null>(null);
  const onEndedRef = useRef(onEnded);
  onEndedRef.current = onEnded;

  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
      wakeUp.current?.();
    };
  }, []);

  const press = useCallback(async () => {
    if (running.current) return;
    running.current = true;
    setDone(false);
    setPhase({ kind: "sending" });
    let id = newCommandId();
    let sent = false;
    let ended: Command | null = null;
    let refusal: string | null = null;
    // A change of the command, or a reason to read it again, has come since the last read.
    let changed = false;
    let wake: (() => void) | null = null;
    const heard = () => {
      changed = true;
      const waiting = wake;
      wake = null;
      waiting?.();
    };
    wakeUp.current = heard;
    const nextChange = () => {
      if (changed) {
        changed = false;
        return Promise.resolve();
      }
      return new Promise<void>((resolve) => {
        wake = () => {
          changed = false;
          resolve();
        };
      });
    };
    // Listening starts before the request, so what happens to the command while it is
    // being accepted is heard.
    watching.watch(id, heard);
    while (alive.current && ended === null && refusal === null) {
      // What to wait for before the next round: the server saying the command changed, a
      // few seconds after a failed read, or nothing when there is a request to make now.
      let wait: "change" | "retry" | "none" = "change";
      try {
        const command = sent ? await getCommand(id) : await sendCommand(id, RESCAN_KIND, { folder_id: folderId });
        sent = true;
        if (!isOpen(command)) ended = command;
        else if (alive.current) setPhase({ kind: "waiting" });
      } catch (e) {
        if (e instanceof ApiError && !sent && (e.code === "invalid" || e.code === "not_found")) {
          refusal = e.message;
        } else if (e instanceof ApiError && !sent && e.code === "conflict") {
          // Another tab is already reading this folder: follow that command.
          const open = e.current as Command | undefined;
          if (open && typeof open.id === "string" && isOpen(open)) {
            watching.unwatch(id);
            id = open.id;
            sent = true;
            watching.watch(id, heard);
            wait = "none";
          } else {
            refusal = e.message;
          }
        } else if (e instanceof ApiError && sent && e.code === "not_found") {
          // The server never stored it: sending it again with the same ID is safe.
          sent = false;
          wait = "none";
        } else {
          if (alive.current) setPhase({ kind: "unconfirmed" });
          wait = "retry";
        }
      }
      if (ended === null && refusal === null && alive.current && wait !== "none") {
        if (wait === "retry") watching.retry(id, heard, RETRY_AFTER_MS);
        await nextChange();
      }
    }
    wakeUp.current = null;
    watching.unwatch(id);
    running.current = false;
    if (!alive.current) return;
    if (refusal !== null) {
      setPhase({ kind: "idle", message: refusal });
      return;
    }
    if (ended !== null) {
      onEndedRef.current();
      if (ended.state === "failed") {
        setPhase({ kind: "idle", message: ended.outcome?.reason ?? FAILED });
      } else {
        setDone(true);
        setPhase({ kind: "idle", message: null });
      }
    }
  }, [folderId, watching]);

  return { phase, done, press };
}
