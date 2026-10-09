import { useCallback, useEffect, useRef, useState } from "react";
import { useNavigationType } from "react-router-dom";

import { ApiError } from "@/lib/api";
import { forgetPrefix, peek, store, useAfterDelay, useStored } from "@/lib/cached";

import { fetchTodos } from "../todo/api";
import { withKinds } from "../todo/kinds";
import { LIST_PREFIX, loadWorkPage, type FilterKey, type LibraryWorkPage, type SortKey } from "./api";
import { prepare, type Work } from "./model";

/**
 * The library list, one page at a time.
 *
 * The server sorts, filters and searches; this reads the first page of a query
 * and, when the screen reaches the end of what it has, the page after it (the
 * `next` cursor of the last one). The pages loaded so far are kept in the cache
 * (`lib/cached.ts`) under the query, so going to a work and coming back with the
 * browser's back step shows all of them in the first render, as tall as they
 * were, and the page scroll goes back to where it was.
 *
 * - Only the pages of the last query are kept: when another query's first page
 *   arrives, the earlier query's pages are dropped.
 * - Opening the library from a menu (not back or forward) shows what is kept and
 *   reads the first page again behind it; the answer replaces the kept pages,
 *   which starts the list over at its top, where the page is.
 * - The pages found as they were still have the to-do badges of when they were read, and a to-do may have been
 *   handled since (a work's 교체 승인 decided from its detail): the to-do list is read once behind them and each
 *   kept work takes its badges from it, the pages and the scroll staying where they are.
 * - Another sort, filter or search starts at the first page and the top of the page.
 * - An answer that is not for the current query any more (typing went on,
 *   another sort was picked) is dropped: reads of an old query are cancelled and
 *   an answer is stored only if nothing else changed the pages meanwhile.
 */

export interface LoadedPages {
  works: Work[];
  /** Where the page after the last loaded one starts; `null` when it was the last. */
  next: string | null;
  /** How many works the filter and the search match, over every page. */
  total: number;
  /** How many works the library has, whatever the filter and the search. */
  libraryCount: number;
}

export interface WorkPagesQuery {
  sort: SortKey;
  filter: FilterKey;
  search: string;
}

const LOAD_FAILED = "작품 목록을 불러오지 못했어요.";

/** The search as it is asked: trimmed and in NFC, as the server compares it. */
const needleOf = (search: string) => search.trim().normalize("NFC");

const keyOf = (query: WorkPagesQuery) =>
  `${LIST_PREFIX}${query.sort}:${query.filter}:${needleOf(query.search).toLocaleLowerCase()}`;

function firstPage(page: LibraryWorkPage): LoadedPages {
  return { works: prepare(page.items), next: page.next, total: page.total, libraryCount: page.library_count };
}

/** The loaded pages and the page after them. A work that is on both (it moved) stays where it was first shown. */
function appended(loaded: LoadedPages, page: LibraryWorkPage): LoadedPages {
  const have = new Set(loaded.works.map((work) => work.id));
  const fresh = prepare(page.items.filter((work) => !have.has(work.id)));
  return {
    works: [...loaded.works, ...fresh],
    next: page.next,
    total: page.total,
    libraryCount: page.library_count,
  };
}

const messageOf = (e: unknown) => (e instanceof ApiError ? e.message : LOAD_FAILED);

export interface WorkPages {
  /**
   * The pages of the current query, or the last ones shown while the current
   * query's first page is on its way (`stale`); `undefined` until the first answer.
   */
  data: LoadedPages | undefined;
  /** `data` is of an earlier query: the current one has not answered yet. */
  stale: boolean;
  /** The first page of the current query could not be read and nothing is kept for it. */
  error: string | null;
  /** Nothing is shown and the first load has taken a while. */
  slow: boolean;
  /** The page after the loaded ones is being read for longer than a moment. */
  loadingMore: boolean;
  /** The page after the loaded ones could not be read. */
  moreFailed: boolean;
  /** Reads the page after the loaded ones, unless it is being read or there is none. */
  loadMore: () => void;
  /** Reads the first page again (after an error). */
  reload: () => void;
}

export function useWorkPages(query: WorkPagesQuery): WorkPages {
  const key = keyOf(query);
  const needle = needleOf(query.search);
  const entry = useStored<LoadedPages>(key);
  const [failure, setFailure] = useState<{ key: string; message: string } | null>(null);
  const [more, setMore] = useState<"idle" | "loading" | "failed">("idle");
  const [round, setRound] = useState(0);

  // The newest query, for reads that start from an event.
  const latest = useRef({ key, sort: query.sort, filter: query.filter, needle });
  latest.current = { key, sort: query.sort, filter: query.filter, needle };
  const moreRead = useRef<AbortController | null>(null);

  const type = useNavigationType();
  const opened = useRef({ type, mounted: false });

  // What was last shown, while the next query's first page is on its way.
  const shown = useRef<LoadedPages | undefined>(undefined);
  if (entry !== undefined) shown.current = entry;

  useEffect(() => {
    const controller = new AbortController();
    const kept = peek<LoadedPages>(key);
    const first = !opened.current.mounted;
    opened.current.mounted = true;
    // Back or forward finds the pages as they were; the first read of anything else is behind what is kept.
    const restore = kept !== undefined && round === 0 && (!first || opened.current.type === "POP");
    if (restore) {
      fetchTodos(controller.signal).then(
        (todos) => {
          const now = peek<LoadedPages>(key);
          if (controller.signal.aborted || now === undefined) return;
          const works = withKinds(now.works, new Map(Object.entries(todos.badges)));
          if (works !== now.works) store(key, { ...now, works });
        },
        // The badges stay as they were read; the next read of the list has them again.
        () => {},
      );
    } else {
      loadWorkPage({ sort: latest.current.sort, filter: latest.current.filter, q: latest.current.needle }, controller.signal).then(
        (page) => {
          if (controller.signal.aborted) return;
          forgetPrefix(LIST_PREFIX);
          store(key, firstPage(page));
          setFailure(null);
          // Another sort, filter or search starts the list over, at its top.
          if (!first && round === 0) window.scrollTo({ top: 0, left: 0, behavior: "instant" });
        },
        (e: unknown) => {
          if (controller.signal.aborted) return;
          setFailure({ key, message: messageOf(e) });
        },
      );
    }
    return () => {
      controller.abort();
      moreRead.current?.abort();
      moreRead.current = null;
      setMore("idle");
    };
  }, [key, round]);

  const loadMore = useCallback(() => {
    const { key: at, sort, filter, needle: q } = latest.current;
    const loaded = peek<LoadedPages>(at);
    if (loaded === undefined || loaded.next === null || moreRead.current !== null) return;
    const controller = new AbortController();
    moreRead.current = controller;
    setMore("loading");
    loadWorkPage({ sort, filter, q, after: loaded.next }, controller.signal).then(
      (page) => {
        if (controller.signal.aborted) return;
        moreRead.current = null;
        setMore("idle");
        // The pages were replaced while this was on its way (a read from the top): the answer no longer continues them.
        if (peek<LoadedPages>(at) === loaded) store(at, appended(loaded, page));
      },
      () => {
        if (controller.signal.aborted) return;
        moreRead.current = null;
        setMore("failed");
      },
    );
  }, []);

  const reload = useCallback(() => {
    setFailure(null);
    setRound((n) => n + 1);
  }, []);

  const data = entry ?? shown.current;
  const error = entry === undefined && failure?.key === key ? failure.message : null;
  const slow = useAfterDelay(data === undefined && error === null);
  const loadingMore = useAfterDelay(more === "loading");
  return {
    data,
    stale: entry === undefined && data !== undefined,
    error,
    slow,
    loadingMore,
    moreFailed: more === "failed",
    loadMore,
    reload,
  };
}

/** `value` once it has stayed the same for `ms`. The first value is taken at once. */
export function useDebounced<T>(value: T, ms: number): T {
  const [settled, setSettled] = useState(value);
  useEffect(() => {
    const timer = setTimeout(() => setSettled(value), ms);
    return () => clearTimeout(timer);
  }, [value, ms]);
  return settled;
}
