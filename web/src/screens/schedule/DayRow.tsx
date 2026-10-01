import { cn } from "@/lib/utils";

import type { Day } from "./api";
import { monthDay, weekdayName } from "./format";
import { WeekCard } from "./WeekCard";

/** The id of today's row: `오늘로 이동` scrolls to it and moves focus to it. */
export const TODAY_ID = "schedule-today";

function Today() {
  return (
    <span className="rounded-full border border-focus px-1.5 py-px text-[11px] leading-tight font-bold text-focus">오늘</span>
  );
}

/**
 * One day of the week: the date and weekday in a column, then the cards of what
 * airs, wrapping across the width. A day with nothing airing is one thin line.
 */
export function DayRow({ day }: { day: Day }) {
  const empty = day.cards.length === 0;
  const name = weekdayName(day.weekday);
  return (
    <li
      id={day.today ? TODAY_ID : undefined}
      tabIndex={day.today ? -1 : undefined}
      data-date={day.date}
      className={cn(
        "grid scroll-mt-[calc(var(--topbar-h)+8px)] grid-cols-[84px_minmax(0,1fr)] gap-x-3 border-t border-hairline-soft px-2 outline-offset-[-2px] first:border-t-0 min-[721px]:grid-cols-[104px_minmax(0,1fr)] min-[721px]:px-3",
        empty ? "items-center py-2" : "items-start py-3",
        day.today && "rounded-card border-t-0 bg-[color-mix(in_srgb,var(--focus-ring)_7%,transparent)] ring-1 ring-focus/60",
      )}
    >
      <div className={cn("flex min-w-0 flex-col gap-0.5", empty && "justify-center")}>
        <span className="flex flex-wrap items-center gap-x-1.5 gap-y-0.5">
          <span className={cn("text-sm font-bold", empty && !day.today && "font-semibold text-text-secondary")}>{name}</span>
          {day.today && <Today />}
        </span>
        <span className="text-xs text-text-muted">{monthDay(day.date)}</span>
      </div>
      {empty ? (
        <p className="min-w-0 text-[13px] text-text-muted">방영 없음</p>
      ) : (
        <ul className="m-0 grid min-w-0 list-none grid-cols-[repeat(auto-fill,minmax(min(100%,264px),1fr))] gap-2 p-0">
          {day.cards.map((card) => (
            <WeekCard key={card.rule_id} card={card} />
          ))}
        </ul>
      )}
    </li>
  );
}
