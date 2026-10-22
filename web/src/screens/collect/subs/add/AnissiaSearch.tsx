import { useEffect, useId, useRef, useState, type FormEvent, type ReactNode } from "react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { ApiError } from "@/lib/api";
import { cn } from "@/lib/utils";
import { WEEKDAYS_LONG } from "@/lib/weekday";
import type { ScheduleEntry } from "@/lib/api-types";

import { btnNeutral, hintClass, inputClass, labelClass } from "../../channels/styles";
import { searchSchedule, type PickedFrom } from "../api";
import { endText, RUN_TEXT, startDate, WEEK_OTHER, WEEK_UPCOMING } from "../format";
import { optionClass } from "./draft";

export const WEEK_NAMES = [...WEEKDAYS_LONG, "기타", "신작"];

/** How long typing has to stop before a search asks Anissia, which is not asked per key. */
const SEARCH_PAUSE_MS = 700;

/** Where an Anissia anime is picked from: the full list, or this quarter's schedule. */
export type AnissiaList = "list" | "schedule";

const LISTS: { id: AnissiaList; label: string }[] = [
  { id: "list", label: "전체 목록" },
  { id: "schedule", label: "편성표" },
];

/** The two places an Anissia anime is picked from, as buttons, the one shown pressed. */
export function AnissiaListTabs({ tab, onTab }: { tab: AnissiaList; onTab: (tab: AnissiaList) => void }) {
  return (
    <div role="group" aria-label="찾는 곳" className="flex gap-1.5">
      {LISTS.map((t) => (
        <button
          key={t.id}
          type="button"
          aria-pressed={tab === t.id}
          onClick={() => onTab(t.id)}
          className={cn(
            "min-h-9 rounded-full border px-4 text-[13px] font-semibold outline-offset-2 focus-visible:outline-2 focus-visible:outline-focus max-[720px]:min-h-10",
            tab === t.id ? "border-focus bg-focus text-primary-foreground" : "border-hairline bg-surface-1 text-text-primary hover:border-text-secondary",
          )}
        >
          {t.label}
        </button>
      ))}
    </div>
  );
}

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
 * Anissia's full list for the text written, every week's anime, a page at a
 * time; the ended ones only with `ended`. It asks once typing pauses, or at
 * once on Enter or `검색`; an empty text asks nothing.
 */
function useSearch(query: string, ended: boolean) {
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
    searchSchedule(q, page, ended, controller.signal)
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
          setError(e instanceof ApiError ? e.message : "Anissia에서 찾지 못했어요. 편성표에서는 고를 수 있어요.");
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
 * `작품 찾기`: a search of Anissia's full list, its field always there, and
 * its results while it holds text, or `children` while it is empty. A pick
 * says the text and the page it was listed on, for the server to check.
 */
export function AnissiaSearch({
  query,
  onQuery,
  ended,
  hint,
  selected,
  onPick,
  children,
}: {
  query: string;
  onQuery: (query: string) => void;
  /** Keep the anime that have ended. */
  ended: boolean;
  /** The line under the field. */
  hint: string;
  selected: ScheduleEntry | null;
  onPick: (entry: ScheduleEntry, from: PickedFrom) => void;
  /** What shows while the field is empty. */
  children?: ReactNode;
}) {
  const uid = useId();
  const search = useSearch(query, ended);
  const results = search.results;
  const searching = query.trim() !== "";

  const submit = (event: FormEvent) => {
    event.preventDefault();
    search.now();
  };

  return (
    <>
      <form onSubmit={submit} className="flex min-w-0 flex-col gap-1.5">
        <Label htmlFor={`${uid}-q`} className={labelClass}>
          작품 찾기
        </Label>
        <div className="flex min-w-0 gap-2">
          <Input
            id={`${uid}-q`}
            type="search"
            value={query}
            onChange={(e) => onQuery(e.target.value)}
            placeholder="한국어 제목"
            autoComplete="off"
            aria-describedby={`${uid}-q-hint`}
            className={cn(inputClass, "min-w-0 flex-1")}
          />
          <Button type="submit" variant="ghost" className={btnNeutral} disabled={!searching}>
            검색
          </Button>
        </div>
        <p id={`${uid}-q-hint`} className={hintClass}>
          {hint}
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
              {ended
                ? `‘${results.q}’로 찾은 Anissia 작품이 없어요. 다른 이름으로 찾아 보세요.`
                : results.hasNext
                  ? "지금까지 찾은 작품은 모두 종영했어요. 결과 더 보기로 더 찾아 보세요."
                  : `‘${results.q}’로 찾은 작품 가운데 종영하지 않은 작품이 없어요.`}
            </p>
          )}
          {results !== null && results.rows.length > 0 && (
            <ul
              className={cn("m-0 flex list-none flex-col gap-2 p-0", results.q !== query.trim() && "opacity-60")}
              aria-label={`‘${results.q}’ 검색 결과`}
            >
              {/* A search's pages are read at different times, so an anime can move into a later page as well. */}
              {results.rows.map(({ entry, page }) => (
                <li key={`${page}:${entry.anime_no}`}>
                  <AnimeOption
                    entry={entry}
                    on={selected?.anime_no === entry.anime_no}
                    withWeek
                    onClick={() => onPick(entry, { q: results.q, page })}
                  />
                </li>
              ))}
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
        children
      )}
    </>
  );
}

/** One anime as a whole-row button: its title, `구독 중`, `결방`·`종영`, its original title and when it airs. */
export function AnimeOption({
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
