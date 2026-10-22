import { useCallback, useState } from "react";

import type { FilterKey, PlaceKey, SortKey } from "./api";
import { DEFAULT_SORT, FILTERS, PLACES } from "./model";

/**
 * What the library list remembers: the view (cover grid or list), the sort,
 * the search text, the filter and the folder, for the life of the page. Going
 * to a work and back (or to another menu and back) finds them as they were; a
 * reload or a new page starts from the cover grid, the default sort, no search,
 * every work (user decision, 2026-10-04) and every folder.
 */

export type ViewKey = "grid" | "list";

interface Kept {
  view: ViewKey;
  sort: SortKey;
  search: string;
  filter: FilterKey;
  place: PlaceKey;
}

let kept: Kept = { view: "grid", sort: DEFAULT_SORT, search: "", filter: "all", place: "all" };

export interface LibraryPrefs extends Kept {
  setView: (view: ViewKey) => void;
  setSort: (sort: SortKey) => void;
  setSearch: (search: string) => void;
  setFilter: (filter: FilterKey) => void;
  setPlace: (place: PlaceKey) => void;
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
    setPlace: (place) => remember({ place: PLACES.some((p) => p.key === place) ? place : "all" }),
  };
}
