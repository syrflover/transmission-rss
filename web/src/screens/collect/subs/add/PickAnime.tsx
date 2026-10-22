import { useId, useState } from "react";

import { Button } from "@/components/ui/button";
import { useCached } from "@/lib/cached";
import { ago } from "@/lib/time";
import { cn } from "@/lib/utils";
import type { Schedule, ScheduleEntry } from "@/lib/api-types";

import { KEYS } from "../../cache";
import { btnNeutral, hintClass } from "../../channels/styles";
import { fetchSchedule, type PickedFrom } from "../api";
import { WEEK_TABS } from "../format";
import { AnimeOption, AnissiaSearch, WEEK_NAMES } from "./AnissiaSearch";

/**
 * Step 1: an anime of Anissia's schedule, by weekday, or found by `작품 찾기`
 * in every week, the ended ones left out. The pick says where it was made, for
 * the server to check.
 */
export function PickAnime({
  week,
  selected,
  onWeek,
  onPick,
  heading = "구독할 작품을 골라요",
}: {
  week: number;
  selected: ScheduleEntry | null;
  onWeek: (week: number) => void;
  onPick: (entry: ScheduleEntry, from: PickedFrom) => void;
  /** The step's title; the subscribe flow's by default. */
  heading?: string;
}) {
  const uid = useId();
  const [query, setQuery] = useState("");
  const searching = query.trim() !== "";
  const schedule = useCached<Schedule>(
    KEYS.schedule(week),
    (signal) => fetchSchedule(week, signal),
    "편성표를 불러오지 못했어요.",
  );
  const entries = schedule.data?.entries;

  return (
    <section aria-labelledby={`${uid}-h`} className="flex min-w-0 flex-col gap-3.5">
      <h3 id={`${uid}-h`} className="text-[15px] font-bold">
        {heading}
      </h3>

      <div role="group" aria-label="요일" className="flex flex-wrap gap-1.5">
        {WEEK_TABS.map((name, index) => (
          <button
            key={name}
            type="button"
            aria-pressed={!searching && index === week}
            aria-label={WEEK_NAMES[index]}
            onClick={() => {
              // A weekday is looked at instead of the search.
              setQuery("");
              onWeek(index);
            }}
            className={cn(
              "min-h-9 min-w-10 rounded-full border px-3 text-[13px] font-semibold outline-offset-2 focus-visible:outline-2 focus-visible:outline-focus max-[720px]:min-h-10",
              !searching && index === week
                ? "border-focus bg-focus text-primary-foreground"
                : "border-hairline bg-surface-1 text-text-primary hover:border-text-secondary",
            )}
          >
            {name}
          </button>
        ))}
      </div>

      <AnissiaSearch
        query={query}
        onQuery={setQuery}
        ended={false}
        hint="모든 요일에서 찾고, 종영한 작품은 빼요. Anissia는 한국어 제목으로 찾아져서 일본어나 영어 제목은 거의 맞지 않아요."
        selected={selected}
        onPick={onPick}
      >
        {entries === undefined && schedule.error === null && schedule.slow && (
          <p className="text-[13px] text-text-muted">편성표를 불러오는 중이에요.</p>
        )}

        {entries === undefined && schedule.error !== null && (
          <div className="flex flex-col items-start gap-2.5">
            <p role="alert" className="text-[13px] leading-normal font-semibold text-urgent">
              {schedule.error}
            </p>
            <Button type="button" variant="ghost" className={btnNeutral} onClick={schedule.reload}>
              재시도
            </Button>
          </div>
        )}

        {entries !== undefined && schedule.error !== null && (
          <div className="flex flex-wrap items-center gap-2.5">
            <p role="alert" className="text-[13px] leading-normal font-semibold text-urgent">
              {schedule.error}
            </p>
            <Button type="button" variant="ghost" className={btnNeutral} onClick={schedule.reload}>
              재시도
            </Button>
          </div>
        )}

        {entries !== undefined && entries.length === 0 && (
          <p className="text-[13px] text-text-muted">{WEEK_NAMES[week]} 편성이 없어요.</p>
        )}

        {entries !== undefined && entries.length > 0 && (
          <ul className="m-0 flex list-none flex-col gap-2 p-0" aria-label={`${WEEK_NAMES[week]} 편성`}>
            {entries.map((entry) => (
              <li key={entry.anime_no}>
                <AnimeOption
                  entry={entry}
                  on={selected?.anime_no === entry.anime_no}
                  withWeek={false}
                  onClick={() => onPick(entry, { week })}
                />
              </li>
            ))}
          </ul>
        )}

        {schedule.data && (
          <p className={hintClass}>
            Anissia 편성표예요. {schedule.data.cached ? `${ago(schedule.data.fetched_at)}에 읽은 내용이에요.` : "방금 읽었어요."}
          </p>
        )}
      </AnissiaSearch>
    </section>
  );
}
