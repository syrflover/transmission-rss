import { useCallback, useState } from "react";

import type { FilterKey, SortKey } from "./api";
import { DEFAULT_SORT, FILTERS } from "./model";

/**
 * What the library list remembers: the view (cover grid or list), the sort,
 * the search text and the filter, for the life of the page. Going to a work
 * and back (or to another menu and back) finds them as they were; a reload or
 * a new page starts from the cover grid, the default sort, no search and every
 * work (user decision, 2026-10-04).
 */

export type ViewKey = "grid" | "list";

interface Kept {
  view: ViewKey;
  sort: SortKey;
  search: string;
  filter: FilterKey;
}

let kept: Kept = { view: "grid", sort: DEFAULT_SORT, search: "", filter: "all" };

export interface LibraryPrefs extends Kept {
  setView: (view: ViewKey) => void;
  setSort: (sort: SortKey) => void;
  setSearch: (search: string) => void;
  setFilter: (filter: FilterKey) => void;
}

export function useLibraryPrefs(): LibraryPrefs {
  const [current, setCurrent] = useState<Kept>(kept);

  const remember = useCallback((next: Partial<Kept>) => {
    kept = { ...kept, ...next };
    setCurrent(kept);
  }, []);

  return {
    ...current,
    setView: (view) => remember({ view }),
    setSort: (sort) => remember({ sort }),
    setSearch: (search) => remember({ search }),
    setFilter: (filter) => remember({ filter: FILTERS.some((f) => f.key === filter) ? filter : "all" }),
  };
}
