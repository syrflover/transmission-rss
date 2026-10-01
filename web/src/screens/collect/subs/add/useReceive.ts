import { useCallback, useEffect, useRef, useState } from "react";

import { ApiError } from "@/lib/api";
import { getCommand, isOpen, newCommandId, type Command } from "@/lib/commands";

import { receiveWithRule } from "../api";

/** How often the open commands are asked for their state, in milliseconds. */
const POLL_MS = 2000;

/**
 * Where one past item's `receive_once` is:
 * - `sending`: the request is on its way.
 * - `waiting`: the server has the command and the worker has not ended it; this
 *   is not `추가함` yet.
 * - `added`: the item is in Transmission (a `duplicate` counts, as in the history).
 * - `failed`: the worker ended the command without adding it, or the request
 *   was refused, with the sentence to show.
 */
export type ReceivePhase =
  | { kind: "sending" }
  | { kind: "waiting" }
  | { kind: "added" }
  /** `lost`: the answer never came, so the server may have the command. */
  | { kind: "failed"; message: string; lost: boolean };

interface Entry {
  itemId: number;
  /** One per user action; a resend after a lost answer reuses it, so it cannot add twice. */
  commandId: string;
  phase: ReceivePhase;
}

function ended(command: Command): ReceivePhase {
  const result = command.outcome?.result;
  if (result === "received" || result === "duplicate") return { kind: "added" };
  return {
    kind: "failed",
    message: command.outcome?.reason ?? "추가하지 못했어요. 까닭은 알 수 없어요.",
    lost: false,
  };
}

/**
 * Receives the ticked past items with the new rule, one `receive_once` each,
 * and follows every command until the worker ended it. Nothing here decides
 * which items are received: the worker checks that the rule would pick each one.
 */
export function useReceive(ruleId: string | null) {
  const [entries, setEntries] = useState<Entry[]>([]);
  const alive = useRef(true);
  const open = useRef<Entry[]>([]);
  open.current = entries;

  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);

  const setPhase = useCallback((itemId: number, phase: ReceivePhase) => {
    setEntries((all) => all.map((e) => (e.itemId === itemId ? { ...e, phase } : e)));
  }, []);

  const send = useCallback(
    async (rule: string, itemId: number, commandId: string) => {
      try {
        const command = await receiveWithRule(commandId, itemId, rule);
        if (alive.current) setPhase(itemId, isOpen(command) ? { kind: "waiting" } : ended(command));
      } catch (e) {
        if (!alive.current) return;
        const refused = e instanceof ApiError && e.code !== "network";
        setPhase(itemId, {
          kind: "failed",
          message: refused ? e.message : "서버에 닿지 못했어요. 요청이 갔는지 알 수 없어요.",
          lost: !refused,
        });
      }
    },
    [setPhase],
  );

  /** Starts receiving `itemIds` with the rule. */
  const start = useCallback(
    (rule: string, itemIds: number[]) => {
      const next = itemIds.map<Entry>((itemId) => ({ itemId, commandId: newCommandId(), phase: { kind: "sending" } }));
      setEntries(next);
      for (const e of next) void send(rule, e.itemId, e.commandId);
    },
    [send],
  );

  /** Sends an item that failed again: a new action after the worker ended it, the same request after a lost answer. */
  const retry = useCallback(
    (itemId: number) => {
      if (!ruleId) return;
      const entry = open.current.find((e) => e.itemId === itemId);
      if (!entry) return;
      // A command the worker ended is spent; a request that never got an answer is sent again as it was.
      const commandId = entry.phase.kind === "failed" && entry.phase.lost ? entry.commandId : newCommandId();
      setEntries((all) => all.map((e) => (e.itemId === itemId ? { ...e, commandId, phase: { kind: "sending" } } : e)));
      void send(ruleId, itemId, commandId);
    },
    [ruleId, send],
  );

  const waiting = entries.some((e) => e.phase.kind === "waiting");
  useEffect(() => {
    if (!waiting) return;
    const timer = window.setInterval(() => {
      for (const e of open.current) {
        if (e.phase.kind !== "waiting") continue;
        getCommand(e.commandId).then(
          (command) => {
            if (alive.current && !isOpen(command)) setPhase(e.itemId, ended(command));
          },
          () => {
            // Cannot reach the server for now: the next poll asks again.
          },
        );
      }
    }, POLL_MS);
    return () => window.clearInterval(timer);
  }, [waiting, setPhase]);

  return { entries, start, retry };
}
