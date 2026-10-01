import { useCallback, useEffect, useRef, useState } from "react";

import { ApiError } from "@/lib/api";

import {
  cancelSearch,
  pollSearch,
  searchContext,
  startSearch,
  type SearchContext,
  type SearchResult,
} from "./api";

/** How often a running search is asked how far it is, in milliseconds. */
const POLL_MS = 700;

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

function message(error: unknown, fallback: string): string {
  return error instanceof ApiError ? error.message : fallback;
}

/**
 * Drives one rule's past episode search. The server runs and judges it; this
 * only starts it, follows it and hands over the finished preview. A search that
 * is left (the section closed, the rule left) is ended on the server, so
 * nothing of it stays, unless something was already sent to be received.
 */
export function usePastSearch(ruleId: string) {
  const [phase, setPhase] = useState<SearchPhase>({ kind: "closed" });
  const searchId = useRef<string | null>(null);
  const keep = useRef(false);

  const drop = useCallback(() => {
    const id = searchId.current;
    searchId.current = null;
    if (id) void cancelSearch(id).catch(() => undefined);
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
    async (context: SearchContext, body: { query: string; from: number; to: number }) => {
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
      if (!keep.current) drop();
      setPhase({ kind: "range", context, error });
    },
    [drop],
  );

  const close = useCallback(() => {
    if (!keep.current) drop();
    setPhase({ kind: "closed" });
  }, [drop]);

  /** Tells the search it has results being received, so leaving does not end it. */
  const hold = useCallback(() => {
    keep.current = true;
  }, []);

  const searching = phase.kind === "searching" ? phase : null;
  useEffect(() => {
    if (!searching) return;
    let cancelled = false;
    let timer: number | undefined;
    const ask = async () => {
      try {
        const poll = await pollSearch(searching.searchId);
        if (cancelled) return;
        if (poll.state === "running") {
          setPhase((now) =>
            now.kind === "searching" && now.searchId === searching.searchId
              ? { ...now, sent: poll.sent, needed: poll.needed }
              : now,
          );
          timer = window.setTimeout(ask, POLL_MS);
        } else if (poll.state === "done" && poll.result) {
          setPhase({ kind: "preview", context: searching.context, searchId: searching.searchId, result: poll.result });
        } else {
          searchId.current = null;
          setPhase({ kind: "range", context: searching.context, error: poll.error ?? "검색하지 못했어요." });
        }
      } catch (e) {
        if (cancelled) return;
        if (e instanceof ApiError && e.code === "not_found") {
          searchId.current = null;
          setPhase({
            kind: "range",
            context: searching.context,
            error: "검색이 사라졌어요. 서버가 다시 시작됐을 수 있어요. 다시 검색해 주세요.",
          });
        } else {
          // The server cannot be reached for now: ask again.
          timer = window.setTimeout(ask, POLL_MS * 3);
        }
      }
    };
    timer = window.setTimeout(ask, POLL_MS);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [searching?.searchId, searching?.context]); // eslint-disable-line react-hooks/exhaustive-deps

  // Leaving the screen ends a search that nothing was received from.
  useEffect(
    () => () => {
      if (!keep.current) {
        const id = searchId.current;
        if (id) void cancelSearch(id).catch(() => undefined);
      }
    },
    [],
  );

  return { phase, open, begin, back, close, hold, searchId };
}
