import { useId, useState } from "react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { useCached } from "@/lib/cached";
import { ago } from "@/lib/time";
import { cn } from "@/lib/utils";
import { WEEKDAYS_LONG } from "@/lib/weekday";

import { KEYS } from "../../cache";
import { btnNeutral, hintClass, inputClass, labelClass } from "../../channels/styles";
import { fetchSchedule, type Schedule, type ScheduleEntry } from "../api";
import { startDate, WEEK_OTHER, WEEK_TABS, WEEK_UPCOMING } from "../format";
import { optionClass } from "./draft";

const WEEK_NAMES = [...WEEKDAYS_LONG, "기타", "신작"];

/** The time of day, or the start date for the weeks that have no weekday. */
function when(entry: ScheduleEntry): string {
  if (entry.week === WEEK_OTHER || entry.week === WEEK_UPCOMING) {
    const start = startDate(entry.start_date);
    return start ? `${start} 시작` : "시작일 미정";
  }
  return entry.air_time ?? "시각 미정";
}

/** Step 1: an anime of Anissia's schedule, by weekday. */
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
  onPick: (entry: ScheduleEntry) => void;
  /** The step's title; the subscribe flow's by default. */
  heading?: string;
}) {
  const uid = useId();
  const [filter, setFilter] = useState("");
  const schedule = useCached<Schedule>(
    KEYS.schedule(week),
    (signal) => fetchSchedule(week, signal),
    "편성표를 불러오지 못했어요.",
  );
  const entries = schedule.data?.entries;
  const needle = filter.trim().toLowerCase();
  const shown = entries?.filter(
    (e) => needle === "" || `${e.subject} ${e.original_subject ?? ""}`.toLowerCase().includes(needle),
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
            aria-pressed={index === week}
            aria-label={WEEK_NAMES[index]}
            onClick={() => onWeek(index)}
            className={cn(
              "min-h-9 min-w-10 rounded-full border px-3 text-[13px] font-semibold outline-offset-2 focus-visible:outline-2 focus-visible:outline-focus max-[720px]:min-h-10",
              index === week
                ? "border-focus bg-focus text-primary-foreground"
                : "border-hairline bg-surface-1 text-text-primary hover:border-text-secondary",
            )}
          >
            {name}
          </button>
        ))}
      </div>

      {entries !== undefined && entries.length > 0 && (
        <div className="flex min-w-0 flex-col gap-1.5">
          <Label htmlFor={`${uid}-filter`} className={labelClass}>
            작품 찾기
          </Label>
          <Input
            id={`${uid}-filter`}
            type="search"
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
            placeholder="제목 일부"
            autoComplete="off"
            className={inputClass}
          />
        </div>
      )}

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

      {shown !== undefined && entries !== undefined && entries.length > 0 && shown.length === 0 && (
        <p className="text-[13px] text-text-muted">‘{filter.trim()}’에 맞는 작품이 없어요.</p>
      )}

      {shown !== undefined && shown.length > 0 && (
        <ul className="m-0 flex list-none flex-col gap-2 p-0" aria-label={`${WEEK_NAMES[week]} 편성`}>
          {shown.map((entry) => {
            const following = entry.subscribed_rules.length > 0;
            const on = selected?.anime_no === entry.anime_no;
            return (
              <li key={entry.anime_no}>
                <button
                  type="button"
                  aria-pressed={on}
                  onClick={() => onPick(entry)}
                  className={cn(optionClass, on ? "border-focus" : "border-hairline-soft")}
                >
                  <span className="flex min-w-0 flex-wrap items-baseline gap-x-2 gap-y-1">
                    <span className="min-w-0 text-[14.5px] leading-snug font-semibold break-words">
                      {entry.subject}
                    </span>
                    {following && (
                      <span className="flex-none rounded-full border border-ok px-2 py-px text-[11.5px] font-semibold text-ok">
                        구독 중
                      </span>
                    )}
                  </span>
                  {entry.original_subject && (
                    <span className="min-w-0 text-xs leading-snug break-words text-text-muted">
                      {entry.original_subject}
                    </span>
                  )}
                  <span className="text-xs text-text-secondary">{when(entry)}</span>
                </button>
              </li>
            );
          })}
        </ul>
      )}

      {schedule.data && (
        <p className={hintClass}>
          Anissia 편성표예요. {schedule.data.cached ? `${ago(schedule.data.fetched_at)}에 읽은 내용이에요.` : "방금 읽었어요."}
        </p>
      )}
    </section>
  );
}
