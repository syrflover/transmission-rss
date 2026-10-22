import { useEffect, useId, useRef, useState, type FormEvent } from "react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { ApiError } from "@/lib/api";
import { useCached } from "@/lib/cached";
import { ago } from "@/lib/time";
import { cn } from "@/lib/utils";
import { WEEKDAYS_LONG } from "@/lib/weekday";
import type { Schedule, ScheduleEntry } from "@/lib/api-types";

import { KEYS } from "../../cache";
import { btnNeutral, hintClass, inputClass, labelClass } from "../../channels/styles";
import { fetchSchedule, searchSchedule, type PickedFrom } from "../api";
import { endText, RUN_TEXT, startDate, WEEK_OTHER, WEEK_TABS, WEEK_UPCOMING } from "../format";
import { optionClass } from "./draft";

const WEEK_NAMES = [...WEEKDAYS_LONG, "기타", "신작"];

/** How long typing has to stop before `작품 찾기` asks Anissia, which is not asked per key. */
const SEARCH_PAUSE_MS = 700;

/**
 * The time of day (the weeks that have no weekday have none), then the start
 * and end dates: `22:30 · 10월 7일 시작 · 12월 23일 종영 예정`. A search's line
 * leads with the week, as it lists every week: `수요일 22:30 · …`, `신작 · …`.
 */
function when(entry: ScheduleEntry, withWeek: boolean): string {
  const start = startDate(entry.start_date);
  const noWeekday = entry.week === WEEK_OTHER || entry.week === WEEK_UPCOMING;
  const time = noWeekday ? null : (entry.air_time ?? "시각 미정");
  const week = withWeek ? (WEEK_NAMES[entry.week] ?? "기타") : null;
  const starts = start ? `${start} 시작` : noWeekday ? "시작일 미정" : null;
  const airs = [week, time].filter((part) => part !== null).join(" ");
  return [airs === "" ? null : airs, starts, endText(entry.end_date, entry.run)]
    .filter((part) => part !== null)
    .join(" · ");
}

/** An anime of a search with the page it was listed on, which the server finds it on again. */
interface Found {
  entry: ScheduleEntry;
  page: number;
}

interface Results {
  q: string;
  rows: Found[];
  page: number;
  hasNext: boolean;
  maxPage: number;
}

/**
 * `작품 찾기`: Anissia's full list for the text written, every week's anime that
 * have not ended, a page at a time. It asks once typing pauses, or at once on
 * Enter or `검색`; an empty text asks nothing and leaves the week's schedule.
 */
function useSearch(query: string) {
  const [results, setResults] = useState<Results | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const asking = useRef<AbortController | null>(null);
  const pause = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const last = useRef<{ q: string; page: number } | null>(null);

  const run = (q: string, page: number) => {
    asking.current?.abort();
    const controller = new AbortController();
    asking.current = controller;
    last.current = { q, page };
    setLoading(true);
    setError(null);
    searchSchedule(q, page, controller.signal)
      .then(
        (answer) => {
          if (controller.signal.aborted) return;
          const rows = answer.entries.map((entry) => ({ entry, page: answer.page }));
          setResults((prev) => ({
            q: answer.q,
            rows: page > 1 && prev?.q === answer.q ? [...prev.rows, ...rows] : rows,
            page: answer.page,
            hasNext: answer.has_next,
            maxPage: answer.max_page,
          }));
        },
        (e: unknown) => {
          if (controller.signal.aborted) return;
          // Another text's results would read as this one's answer, and their next page as its next page.
          setResults((prev) => (prev?.q === q ? prev : null));
          setError(e instanceof ApiError ? e.message : "Anissia에서 찾지 못했어요. 요일 탭에서는 고를 수 있어요.");
        },
      )
      .finally(() => {
        if (!controller.signal.aborted) setLoading(false);
      });
  };

  const q = query.trim();
  useEffect(() => {
    if (q === "") {
      asking.current?.abort();
      last.current = null;
      setResults(null);
      setError(null);
      setLoading(false);
      return;
    }
    pause.current = setTimeout(() => run(q, 1), SEARCH_PAUSE_MS);
    return () => clearTimeout(pause.current);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [q]);
  useEffect(() => () => asking.current?.abort(), []);

  return {
    results,
    loading,
    error,
    /** Enter or `검색`: the text as it is now, unless its first page is what is shown or asked. */
    now: () => {
      clearTimeout(pause.current);
      if (q === "" || (last.current?.q === q && (loading || error === null))) return;
      run(q, 1);
    },
    /** The next page, of the text's own results only. */
    more: () => results?.q === q && run(q, results.page + 1),
    retry: () => last.current && run(last.current.q, last.current.page),
  };
}

/**
 * Step 1: an anime of Anissia's schedule, by weekday, or found by `작품 찾기`
 * in every week. The pick says where it was made, for the server to check.
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
  const search = useSearch(query);
  const schedule = useCached<Schedule>(
    KEYS.schedule(week),
    (signal) => fetchSchedule(week, signal),
    "편성표를 불러오지 못했어요.",
  );
  const entries = schedule.data?.entries;
  const results = search.results;

  const submit = (event: FormEvent) => {
    event.preventDefault();
    search.now();
  };

  // A search's pages are read at different times, so an anime can move into a later page as well.
  const row = (entry: ScheduleEntry, from: PickedFrom) => (
    <li key={"q" in from ? `${from.page}:${entry.anime_no}` : entry.anime_no}>
      <AnimeOption
        entry={entry}
        on={selected?.anime_no === entry.anime_no}
        withWeek={"q" in from}
        onClick={() => onPick(entry, from)}
      />
    </li>
  );

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

      <form onSubmit={submit} className="flex min-w-0 flex-col gap-1.5">
        <Label htmlFor={`${uid}-filter`} className={labelClass}>
          작품 찾기
        </Label>
        <div className="flex min-w-0 gap-2">
          <Input
            id={`${uid}-filter`}
            type="search"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="한국어 제목"
            autoComplete="off"
            aria-describedby={`${uid}-filter-hint`}
            className={cn(inputClass, "min-w-0 flex-1")}
          />
          <Button type="submit" variant="ghost" className={btnNeutral} disabled={!searching}>
            검색
          </Button>
        </div>
        <p id={`${uid}-filter-hint`} className={hintClass}>
          모든 요일에서 찾고, 종영한 작품은 빼요. Anissia는 한국어 제목으로 찾아져서 일본어나 영어 제목은 거의 맞지 않아요.
        </p>
      </form>

      {searching ? (
        <>
          {search.error !== null && (
            <div className="flex flex-wrap items-center gap-2.5">
              <p role="alert" className="text-[13px] leading-normal font-semibold text-urgent">
                {search.error}
              </p>
              <Button type="button" variant="ghost" className={btnNeutral} onClick={search.retry}>
                재시도
              </Button>
            </div>
          )}
          {results === null && search.loading && <p className={hintClass}>Anissia에서 찾는 중이에요.</p>}
          {results !== null && results.rows.length === 0 && !search.loading && search.error === null && (
            <p className="text-[13px] text-text-muted">
              {results.hasNext
                ? "지금까지 찾은 작품은 모두 종영했어요. 결과 더 보기로 더 찾아 보세요."
                : `‘${results.q}’로 찾은 작품 가운데 종영하지 않은 작품이 없어요.`}
            </p>
          )}
          {results !== null && results.rows.length > 0 && (
            <ul
              className={cn("m-0 flex list-none flex-col gap-2 p-0", results.q !== query.trim() && "opacity-60")}
              aria-label={`‘${results.q}’ 검색 결과`}
            >
              {results.rows.map(({ entry, page }) => row(entry, { q: results.q, page }))}
            </ul>
          )}
          {results !== null && results.rows.length > 0 && search.loading && (
            <p className={hintClass}>Anissia에서 찾는 중이에요.</p>
          )}
          {results?.q === query.trim() && results.hasNext && results.page < results.maxPage && !search.loading && (
            <Button type="button" variant="ghost" className={cn(btnNeutral, "self-start")} onClick={search.more}>
              결과 더 보기
            </Button>
          )}
        </>
      ) : (
        <>
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
              {entries.map((entry) => row(entry, { week }))}
            </ul>
          )}

          {schedule.data && (
            <p className={hintClass}>
              Anissia 편성표예요. {schedule.data.cached ? `${ago(schedule.data.fetched_at)}에 읽은 내용이에요.` : "방금 읽었어요."}
            </p>
          )}
        </>
      )}
    </section>
  );
}

/** One anime as a whole-row button: its title, `구독 중`, `결방`·`종영`, its original title and when it airs. */
function AnimeOption({
  entry,
  on,
  withWeek,
  onClick,
}: {
  entry: ScheduleEntry;
  on: boolean;
  withWeek: boolean;
  onClick: () => void;
}) {
  const following = entry.subscribed_rules.length > 0;
  return (
    <button
      type="button"
      aria-pressed={on}
      onClick={onClick}
      className={cn(optionClass, on ? "border-focus" : "border-hairline-soft")}
    >
      <span className="flex min-w-0 flex-wrap items-baseline gap-x-2 gap-y-1">
        <span className="min-w-0 text-[14.5px] leading-snug font-semibold break-words">{entry.subject}</span>
        {following && (
          <span className="flex-none rounded-full border border-ok px-2 py-px text-[11.5px] font-semibold text-ok">
            구독 중
          </span>
        )}
        {(entry.run === "off" || entry.run === "end") && (
          <span className="flex-none rounded-full border border-hairline px-2 py-px text-[11.5px] font-semibold text-text-secondary">
            {RUN_TEXT[entry.run]}
          </span>
        )}
      </span>
      {entry.original_subject && (
        <span className="min-w-0 text-xs leading-snug break-words text-text-muted">{entry.original_subject}</span>
      )}
      <span className="text-xs text-text-secondary">{when(entry, withWeek)}</span>
    </button>
  );
}
