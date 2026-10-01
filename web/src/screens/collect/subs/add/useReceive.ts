import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { ApiError } from "@/lib/api";
import { getCommand, isOpen, newCommandId, type Command } from "@/lib/commands";

import { receiveWithRule } from "../api";

/** How often the open commands are asked for their state, in milliseconds. */
const POLL_MS = 2000;

/**
 * Where one item's receive command is:
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

/** One receive command, for the item `key` (a history item's ID, or a search result's key). */
export interface CommandRow<K> {
  key: K;
  /** One per user action; a resend after a lost answer reuses it, so it cannot add twice. */
  commandId: string;
  phase: ReceivePhase;
}

/** What each phase says on a row. */
export const PHASE_TEXT: Record<ReceivePhase["kind"], string> = {
  sending: "요청하는 중",
  waiting: "추가하는 중",
  added: "추가함",
  failed: "추가하지 못함",
};

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
 * Sends one receive command per item and follows every command until the
 * worker ended it. `send(commandId, key, scope)` makes the request; `scope` is
 * the rule it is for. Nothing here decides which items are received: the
 * worker checks that the rule would pick each one.
 *
 * Commands are sent one after another in the order given, so the worker, which
 * runs the oldest first, adds them in that order. Every ticked item is sent
 * even when the screen is left meanwhile (what the person asked for is not
 * dropped half way); only the state of a screen that is gone is not updated.
 */
export function useCommandRows<K extends string | number>(
  scope: string | null,
  send: (commandId: string, key: K, scope: string) => Promise<Command>,
) {
  const [rows, setRows] = useState<CommandRow<K>[]>([]);
  const alive = useRef(true);
  const open = useRef<CommandRow<K>[]>([]);
  open.current = rows;
  const sender = useRef(send);
  sender.current = send;

  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);

  const setPhase = useCallback((key: K, phase: ReceivePhase) => {
    setRows((all) => all.map((r) => (r.key === key ? { ...r, phase } : r)));
  }, []);

  const post = useCallback(
    async (forScope: string, key: K, commandId: string) => {
      try {
        const command = await sender.current(commandId, key, forScope);
        if (alive.current) setPhase(key, isOpen(command) ? { kind: "waiting" } : ended(command));
      } catch (e) {
        if (!alive.current) return;
        const refused = e instanceof ApiError && e.code !== "network";
        setPhase(key, {
          kind: "failed",
          message: refused ? e.message : "서버에 닿지 못했어요. 요청이 갔는지 알 수 없어요.",
          lost: !refused,
        });
      }
    },
    [setPhase],
  );

  /** Starts receiving `keys`, in order, for `forScope`. */
  const start = useCallback(
    (forScope: string, keys: K[]) => {
      const next = keys.map<CommandRow<K>>((key) => ({ key, commandId: newCommandId(), phase: { kind: "sending" } }));
      setRows(next);
      void (async () => {
        for (const row of next) {
          await post(forScope, row.key, row.commandId);
        }
      })();
    },
    [post],
  );

  /** Sends an item that failed again: a new action after the worker ended it, the same request after a lost answer. */
  const retry = useCallback(
    (key: K) => {
      if (!scope) return;
      const row = open.current.find((r) => r.key === key);
      if (!row) return;
      // A command the worker ended is spent; a request that never got an answer is sent again as it was.
      const commandId = row.phase.kind === "failed" && row.phase.lost ? row.commandId : newCommandId();
      setRows((all) => all.map((r) => (r.key === key ? { ...r, commandId, phase: { kind: "sending" } } : r)));
      void post(scope, key, commandId);
    },
    [scope, post],
  );

  /**
   * Receives one item, from a row of its own: a new command per press, or the
   * same one again after a lost answer. Does nothing while the item's command
   * is under way or has added it.
   */
  const one = useCallback(
    (key: K) => {
      if (!scope) return;
      const row = open.current.find((r) => r.key === key);
      if (row && row.phase.kind !== "failed") return;
      const commandId = row?.phase.kind === "failed" && row.phase.lost ? row.commandId : newCommandId();
      const next: CommandRow<K> = { key, commandId, phase: { kind: "sending" } };
      setRows((all) => [...all.filter((r) => r.key !== key), next]);
      void post(scope, key, commandId);
    },
    [scope, post],
  );

  const waiting = rows.some((r) => r.phase.kind === "waiting");
  useEffect(() => {
    if (!waiting) return;
    const timer = window.setInterval(() => {
      for (const r of open.current) {
        if (r.phase.kind !== "waiting") continue;
        getCommand(r.commandId).then(
          (command) => {
            if (alive.current && !isOpen(command)) setPhase(r.key, ended(command));
          },
          () => {
            // Cannot reach the server for now: the next poll asks again.
          },
        );
      }
    }, POLL_MS);
    return () => window.clearInterval(timer);
  }, [waiting, setPhase]);

  return { rows, start, retry, one };
}

/** One past item's `receive_once`, as the subscribe flow and the rule detail show it. */
interface Entry {
  itemId: number;
  commandId: string;
  phase: ReceivePhase;
}

/**
 * Receives the ticked past items with the rule, one `receive_once` each, and
 * follows every command until the worker ended it.
 */
export function useReceive(ruleId: string | null) {
  const { rows, start, retry, one } = useCommandRows<number>(ruleId, (commandId, itemId, rule) =>
    receiveWithRule(commandId, itemId, rule),
  );
  const entries = useMemo<Entry[]>(
    () => rows.map((r) => ({ itemId: r.key, commandId: r.commandId, phase: r.phase })),
    [rows],
  );
  return { entries, start, retry, one };
}
