import { useCallback, useEffect, useRef, useState } from "react";

import { ApiError } from "@/lib/api";
import { getCommand, isOpen, newCommandId, type Command } from "@/lib/commands";
import { useCommandWatch } from "@/lib/useCommandWatch";

import { rulesDeleted } from "../../collect/cache";
import { sendWorkArchive, type WorkArchiveConflict, type WorkArchiveRule, type WorkDetail } from "../api";
import type { WorkArchivePhase } from "./archiveNotice";

/** How long to wait before reading again a command whose last read failed, in milliseconds. */
const RETRY_AFTER_MS = 2000;

interface Attempt {
  id: string;
  rules: WorkArchiveRule[];
  /** `false` for a command found on the work at load: it is only followed, never resent. */
  mine: boolean;
}

/**
 * The `보관 폴더로 옮기기` flow of one work, following the web command contract as a rule's `보관` does
 * (`collect/rules/useArchiveMove.ts`):
 *
 * - One ID per user action, made when the move is confirmed and kept until the command ends or the server refuses
 *   the request. A resend is the same request, rules included.
 * - The answer only says accepted (the rules are deleted by then). The screen then reads the command by its ID, now
 *   and whenever the server says it changed, until the worker has ended it.
 * - When the answer is lost, the screen asks for the command by the same ID. Only a `404` allows sending it again.
 * - When the command ends, the work is read again through `onRefresh` (it may be in the archive folder now).
 */
export function useWorkArchive(work: WorkDetail, onRefresh: () => Promise<void>) {
  const [phase, setPhase] = useState<WorkArchivePhase>({ kind: "idle", error: null });
  const attempt = useRef<Attempt | null>(null);
  const watching = useCommandWatch();
  const alive = useRef(true);
  const refresh = useRef(onRefresh);
  refresh.current = onRefresh;
  const workId = work.id;
  /** The move whose deleted rules the screens were told of. */
  const told = useRef<string | null>(null);

  const ask = useRef<() => Promise<void>>(async () => {});

  const watch = () => {
    const id = attempt.current?.id;
    if (id) watching.watch(id, () => ask.current());
  };

  const later = () => {
    const id = attempt.current?.id;
    if (id) watching.retry(id, () => ask.current(), RETRY_AFTER_MS);
  };

  /** Reads the work again; a failed read leaves the page as it is. */
  const reread = () => void refresh.current().catch(() => {});

  const follow = useCallback((command: Command) => {
    if (!alive.current) return;
    if (told.current !== command.id) {
      // The move's rules were deleted with it, however it was found (its answer, a lookup after a lost answer, a move
      // open on the work): the screens that list rules read the server again.
      told.current = command.id;
      rulesDeleted();
    }
    if (isOpen(command)) {
      setPhase({ kind: "waiting" });
      watch();
      return;
    }
    attempt.current = null;
    watching.stop();
    setPhase({ kind: "ended", command });
    reread();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  /** Asks for the command by its ID: after a lost answer, and when the server says it changed. */
  const lookup = useCallback(async () => {
    const current = attempt.current;
    if (!current) return;
    try {
      const command = await getCommand(current.id);
      if (attempt.current?.id === current.id) follow(command);
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
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [follow]);
  ask.current = lookup;

  const send = useCallback(async () => {
    const current = attempt.current;
    if (!current?.mine) return;
    setPhase({ kind: "sending" });
    try {
      const command = await sendWorkArchive(workId, current.id, current.rules);
      follow(command);
      // The work's rules and its move as the server has them now.
      if (isOpen(command)) reread();
    } catch (e) {
      if (!alive.current) return;
      if (e instanceof ApiError && e.code === "conflict") {
        const now = e.current as WorkArchiveConflict | undefined;
        const open = now?.command;
        if (open && open.id !== current.id && isOpen(open)) {
          // The work is moving already (another tab): follow that move.
          attempt.current = { id: open.id, rules: [], mine: false };
          follow(open);
        } else {
          attempt.current = null;
          setPhase({ kind: "idle", error: e.message });
        }
        // The rules or the folders changed: the page shows them as they are.
        reread();
      } else if (e instanceof ApiError && (e.code === "invalid" || e.code === "not_found")) {
        // Refused: nothing was deleted or stored, and the ID is not used again.
        attempt.current = null;
        setPhase({ kind: "idle", error: e.message });
      } else {
        // The answer is lost or the server failed: look it up by the same ID.
        setPhase({ kind: "unconfirmed", lost: false });
        later();
      }
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [follow, workId]);

  /**
   * The move was confirmed with `rules`, the rules the confirmation showed (none for `다시 옮기기`): a new user action,
   * so a new ID.
   */
  const submit = useCallback(
    (rules: WorkArchiveRule[]) => {
      if (attempt.current) return;
      attempt.current = { id: newCommandId(), rules, mine: true };
      void send();
    },
    [send],
  );

  /** Sends the same request again after the server said it never stored it. */
  const resend = useCallback(() => void send(), [send]);

  /** Asks again right away. */
  const recheck = useCallback(() => {
    watching.stop();
    void lookup();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [lookup]);

  // A move still open when the work was loaded (or read again): follow it.
  const open = work.archive.command && isOpen(work.archive.command) ? work.archive.command : null;
  const openId = open?.id ?? null;
  useEffect(() => {
    alive.current = true;
    if (open && !attempt.current) {
      attempt.current = { id: open.id, rules: [], mine: false };
      follow(open);
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
      attempt.current = { id: open.id, rules: [], mine: false };
      follow(open);
    }
    // Only when an open move appears on the work; the effect above handles the mount.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [openId]);

  /** Whether a move is under way from this page's point of view: no new one is offered. */
  const busy = phase.kind === "sending" || phase.kind === "waiting" || phase.kind === "unconfirmed";

  return { phase, busy, submit, resend, recheck };
}
