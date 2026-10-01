import { useEffect, useState } from "react";

import { api, ApiError } from "@/lib/api";

import type { Preview } from "./types";

/** What Anissia's schedule says about an anime. */
export interface ScheduledAnime {
  subject: string;
  /** 0 (Sunday) to 6 (Saturday), 7 (`기타`) or 8 (`신작`). */
  week: number;
  air_time: string | null;
}

export interface SuggestionLookup {
  /** `loading` while Anissia is still being asked; the review is usable all the while. */
  status: "idle" | "loading" | "done";
  /** The schedule of each anime found, by Anissia's anime number. */
  schedules: Record<number, ScheduledAnime>;
  /** The creators Anissia lists for an anime; null when they could not be read. */
  creators: Record<number, string[] | null>;
  /**
   * The anime Anissia answered without, though every week of its schedule was
   * read: nothing is known of them, so a subscription cannot be made. Empty when
   * Anissia stopped answering, since then nothing is known either way.
   */
  unlisted: ReadonlySet<number>;
  /** Why Anissia stopped answering, if it did. */
  problem: string | null;
}

const NOTHING: SuggestionLookup = {
  status: "idle",
  schedules: {},
  creators: {},
  unlisted: new Set(),
  problem: null,
};

interface ScheduleAnswer {
  entries: { anime_no: number; week: number; subject: string; air_time: string | null }[];
}

interface CreatorsAnswer {
  creators: { name: string }[];
}

/** The last week of Anissia's schedule: `신작`. */
const LAST_WEEK = 8;

/**
 * Asks Anissia, in the background, for what the review shows beside each
 * suggestion: the anime's weekday and time, and whether the creator the comment
 * names is one of the anime's. The preview itself never waits for Anissia, and a
 * failure only leaves those values undecided (`미정`): it never blocks the import.
 * An anime that Anissia answered without is reported in `unlisted`.
 *
 * Anissia has no request for one anime, so the schedule is read week by week
 * (each answer is cached by the server for a few minutes) until every anime of
 * the suggestions has been found.
 */
export function useSuggestionLookup(preview: Preview | null): SuggestionLookup {
  const [lookup, setLookup] = useState<SuggestionLookup>(NOTHING);

  useEffect(() => {
    if (!preview) {
      setLookup(NOTHING);
      return;
    }
    const animes = new Set<number>();
    const withCreator = new Set<number>();
    for (const channel of preview.channels) {
      if (channel.not_imported !== null) continue;
      for (const rule of channel.rules) {
        const { anime_no, creator } = rule.suggestion;
        if (anime_no === null) continue;
        animes.add(anime_no);
        if (creator !== null) withCreator.add(anime_no);
      }
    }
    if (animes.size === 0) {
      setLookup({ ...NOTHING, status: "done" });
      return;
    }

    let cancelled = false;
    let current: SuggestionLookup = { ...NOTHING, status: "loading" };
    const publish = (next: Partial<SuggestionLookup>) => {
      current = { ...current, ...next };
      if (!cancelled) setLookup(current);
    };
    const stop = (error: unknown) =>
      publish({
        status: "done",
        problem: error instanceof ApiError ? error.message : "Anissia에 연결하지 못했어요.",
      });

    void (async () => {
      publish({});
      const missing = new Set(animes);
      for (let week = 0; week <= LAST_WEEK && missing.size > 0; week += 1) {
        let answer: ScheduleAnswer;
        try {
          answer = await api<ScheduleAnswer>(`/anissia/schedule/${week}`);
        } catch (error) {
          stop(error);
          return;
        }
        if (cancelled) return;
        const found: Record<number, ScheduledAnime> = {};
        for (const entry of answer.entries) {
          if (missing.delete(entry.anime_no)) {
            found[entry.anime_no] = { subject: entry.subject, week: entry.week, air_time: entry.air_time };
          }
        }
        publish({ schedules: { ...current.schedules, ...found } });
      }
      // The loop ends early only by failing, so what is still missing was not listed.
      if (missing.size > 0) publish({ unlisted: new Set(missing) });

      for (const animeNo of withCreator) {
        let names: string[] | null;
        try {
          const answer = await api<CreatorsAnswer>(`/anissia/anime/${animeNo}/creators`);
          names = answer.creators.map((creator) => creator.name);
        } catch (error) {
          publish({ creators: { ...current.creators, [animeNo]: null } });
          stop(error);
          return;
        }
        if (cancelled) return;
        publish({ creators: { ...current.creators, [animeNo]: names } });
      }
      publish({ status: "done" });
    })();

    return () => {
      cancelled = true;
    };
  }, [preview]);

  return lookup;
}
