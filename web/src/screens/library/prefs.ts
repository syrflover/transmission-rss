import { useCallback, useState } from "react";

import { readStoredChoices, writeStoredChoices, type Choices } from "@/lib/stored-choices";
import { browserStore } from "@/lib/theme";

import type { FilterKey, PlaceKey, SortKey } from "./api";
import { DEFAULT_SORT, FILTERS, PLACES, SORTS } from "./model";

/**
 * What the library list remembers: the view (cover grid or list), the sort,
 * the filter and the folder in this browser, so a reload or a new tab starts
 * from the last ones (src/lib/stored-choices.ts), and the search text for the
 * life of the page (user decision, 2026-10-23). Going to a work and back (or to
 * another menu and back) finds all of them as they were. A kept value the list
 * no longer offers, or storage that cannot be used, starts that item from the
 * cover grid, the default sort, every work or every folder.
 */

export type ViewKey = "grid" | "list";

interface Kept {
  view: ViewKey;
  sort: SortKey;
  search: string;
  filter: FilterKey;
  place: PlaceKey;
}

type Stored = Omit<Kept, "search">;

const STORAGE_KEY = "trss-library-list-v1";

const DEFAULTS: Stored = { view: "grid", sort: DEFAULT_SORT, filter: "all", place: "all" };

const CHOICES: Choices<Stored> = {
  view: ["grid", "list"],
  sort: SORTS.map((s) => s.key),
  filter: FILTERS.map((f) => f.key),
  place: PLACES.map((p) => p.key),
};

let kept: Kept = { ...readStoredChoices(browserStore(), STORAGE_KEY, DEFAULTS, CHOICES), search: "" };

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
    if (next.search === undefined) {
      const { search: _search, ...stored } = kept;
      writeStoredChoices(browserStore(), STORAGE_KEY, stored);
    }
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
