import { useCallback, useState } from "react";

import { DEFAULT_SORT, FILTERS, SORTS, type FilterKey, type SortKey } from "./model";

/**
 * What the library list remembers.
 *
 * - The view (cover grid or list) and the sort stay in this browser
 *   (`localStorage`), so the next visit opens the same way. Storage can be
 *   blocked or full; every access is guarded and the screen works without it.
 * - The search text and the filter are kept for the life of the page: going to
 *   a work and back (or to another menu and back) finds them as they were. A
 *   reload starts without them.
 */

export type ViewKey = "grid" | "list";

const STORAGE_KEY = "trss-library-v1";

interface Saved {
  view: ViewKey;
  sort: SortKey;
}

const DEFAULT_SAVED: Saved = { view: "grid", sort: DEFAULT_SORT };

function readSaved(): Saved {
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    if (raw) {
      const parsed = JSON.parse(raw) as Partial<Record<keyof Saved, unknown>>;
      return {
        view: parsed.view === "grid" || parsed.view === "list" ? parsed.view : DEFAULT_SAVED.view,
        sort: SORTS.some((s) => s.key === parsed.sort) ? (parsed.sort as SortKey) : DEFAULT_SAVED.sort,
      };
    }
  } catch {
    // Blocked storage or an unreadable value: the defaults.
  }
  return DEFAULT_SAVED;
}

function writeSaved(saved: Saved) {
  try {
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify(saved));
  } catch {
    // Not remembered; the choice still applies to this visit.
  }
}

interface Session {
  search: string;
  filter: FilterKey;
}

let session: Session = { search: "", filter: "all" };

export interface LibraryPrefs extends Saved, Session {
  setView: (view: ViewKey) => void;
  setSort: (sort: SortKey) => void;
  setSearch: (search: string) => void;
  setFilter: (filter: FilterKey) => void;
}

export function useLibraryPrefs(): LibraryPrefs {
  const [saved, setSaved] = useState<Saved>(readSaved);
  const [current, setCurrent] = useState<Session>(session);

  const change = useCallback((next: Partial<Saved>) => {
    setSaved((was) => {
      const merged = { ...was, ...next };
      writeSaved(merged);
      return merged;
    });
  }, []);
  const remember = useCallback((next: Partial<Session>) => {
    session = { ...session, ...next };
    setCurrent(session);
  }, []);

  return {
    ...saved,
    ...current,
    setView: (view) => change({ view }),
    setSort: (sort) => change({ sort }),
    setSearch: (search) => remember({ search }),
    setFilter: (filter) => remember({ filter: FILTERS.some((f) => f.key === filter) ? filter : "all" }),
  };
}
