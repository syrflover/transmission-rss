import { useCallback, useEffect, useRef, useState } from "react";

import { ApiError } from "@/lib/api";
import type { Change, SearchContext, SearchResult, StartSearchBody } from "@/lib/api-types";
import { useChanges } from "@/lib/useChanges";

import { cancelSearch, pollSearch, searchContext, startSearch } from "./api";

/**
 * Where a past episode search is:
 * - `closed`: not opened; nothing was asked of the server.
 * - `loading`: the search words and the range are being read.
 * - `range`: step 1, the words and the range the person confirms. `error` is
 *   why the last search could not start or end.
 * - `searching`: the server is reading the tracker. `sent` of `needed` extra
 *   searches are done once the first page showed it was full.
 * - `preview`: steps 2 and 3, the judged results.
 */
export type SearchPhase =
  | { kind: "closed" }
  | { kind: "loading" }
  | { kind: "range"; context: SearchContext; error: string | null }
  | { kind: "searching"; context: SearchContext; searchId: string; sent: number; needed: number }
  | { kind: "preview"; context: SearchContext; searchId: string; result: SearchResult }
  | { kind: "unavailable"; message: string };

/** How long to wait before reading a search again when the last read got no answer, in milliseconds. */
const RETRY_AFTER_MS = 2100;

function message(error: unknown, fallback: string): string {
  return error instanceof ApiError ? error.message : fallback;
}

/**
 * Drives one rule's past episode search. The server runs and judges it; this
 * only starts it, follows it and hands over the finished preview. It follows
 * the search by reading it when it starts and again whenever the server says
 * the search changed (it got further or ended), not on a timer. A search that
 * is left (the section closed, the rule left) is ended on the server, so
 * nothing of it stays, unless something of that very search was already sent
 * to be received (a later search of the rule is ended as usual).
 */
export function usePastSearch(ruleId: string) {
  const [phase, setPhase] = useState<SearchPhase>({ kind: "closed" });
  const searchId = useRef<string | null>(null);
  /** The search whose results are being received: the server keeps it until its time runs out. */
  const held = useRef<string | null>(null);

  /** Lets go of the current search: ends it on the server unless it was handed off to a receive. */
  const drop = useCallback(() => {
    const id = searchId.current;
    searchId.current = null;
    if (id && id !== held.current) void cancelSearch(id).catch(() => undefined);
  }, []);

  /** Opens the section: reads the search words and the range to start from. */
  const open = useCallback(async () => {
    setPhase({ kind: "loading" });
    try {
      const context = await searchContext(ruleId);
      if (context.running) {
        searchId.current = context.running;
        setPhase({ kind: "searching", context, searchId: context.running, sent: 0, needed: 0 });
      } else {
        setPhase({ kind: "range", context, error: null });
      }
    } catch (e) {
      setPhase({ kind: "unavailable", message: message(e, "검색을 준비하지 못했어요. 잠시 뒤 다시 시도해 주세요.") });
    }
  }, [ruleId]);

  const begin = useCallback(
    async (context: SearchContext, body: StartSearchBody) => {
      try {
        const { search_id } = await startSearch(ruleId, body);
        searchId.current = search_id;
        setPhase({ kind: "searching", context, searchId: search_id, sent: 0, needed: 0 });
      } catch (e) {
        setPhase({ kind: "range", context, error: message(e, "검색을 시작하지 못했어요. 잠시 뒤 다시 시도해 주세요.") });
      }
    },
    [ruleId],
  );

  /** Back to step 1 with the words and range as they were; the running search ends. */
  const back = useCallback(
    (context: SearchContext, error: string | null = null) => {
      drop();
      setPhase({ kind: "range", context, error });
    },
    [drop],
  );

  const close = useCallback(() => {
    drop();
    setPhase({ kind: "closed" });
  }, [drop]);

  /** Tells the current search it has results being received, so leaving does not end it. */
  const hold = useCallback(() => {
    held.current = searchId.current;
  }, []);

  const searching = phase.kind === "searching" ? phase : null;
  const following = useRef(searching);
  following.current = searching;
  const reading = useRef({ busy: false, again: false });
  const retryTimer = useRef<number | undefined>(undefined);

  /** Reads how far the search is, and reads again once if it changed meanwhile. */
  const ask = useCallback(async () => {
    const now = following.current;
    if (!now) return;
    if (reading.current.busy) {
      reading.current.again = true;
      return;
    }
    reading.current.busy = true;
    const mine = () => following.current?.searchId === now.searchId;
    try {
      const poll = await pollSearch(now.searchId);
      if (!mine()) return;
      if (poll.state === "running") {
        setPhase((was) =>
          was.kind === "searching" && was.searchId === now.searchId
            ? { ...was, sent: poll.sent, needed: poll.needed }
            : was,
        );
      } else if (poll.state === "done" && poll.result) {
        setPhase({ kind: "preview", context: now.context, searchId: now.searchId, result: poll.result });
      } else {
        searchId.current = null;
        setPhase({ kind: "range", context: now.context, error: poll.error ?? "검색하지 못했어요." });
      }
    } catch (e) {
      if (!mine()) return;
      if (e instanceof ApiError && e.code === "not_found") {
        searchId.current = null;
        setPhase({
          kind: "range",
          context: now.context,
          error: "검색이 사라졌어요. 서버가 다시 시작됐을 수 있어요. 다시 검색해 주세요.",
        });
      } else {
        // The server cannot be reached for now: ask again soon.
        window.clearTimeout(retryTimer.current);
        retryTimer.current = window.setTimeout(() => void ask(), RETRY_AFTER_MS);
      }
    } finally {
      reading.current.busy = false;
      if (reading.current.again) {
        reading.current.again = false;
        void ask();
      }
    }
  }, []);

  // Listening starts before the first read, so a change during that read is heard.
  const aboutSearch = useCallback(
    (change: Change) => change.kind === "past_search" && change.id === following.current?.searchId,
    [],
  );
  useChanges(aboutSearch, () => void ask(), searching !== null);
  useEffect(() => {
    if (!searching) return;
    void ask();
    return () => window.clearTimeout(retryTimer.current);
  }, [searching?.searchId, searching?.context]); // eslint-disable-line react-hooks/exhaustive-deps

  // Leaving the screen ends a search that nothing was received from.
  useEffect(() => drop, [drop]);

  return { phase, open, begin, back, close, hold, searchId };
}
