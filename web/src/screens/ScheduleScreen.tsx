import { useEffect, useState } from "react";

import { useCached } from "@/lib/cached";
import { quarterName } from "@/lib/quarter";
import { clock } from "@/lib/time";

import { EmptyState, ScreenFrame } from "./ScreenFrame";
import { Checklist } from "./schedule/Checklist";
import { DayRow, TODAY_ID } from "./schedule/DayRow";
import { monthDay } from "./schedule/format";
import { SideColumn, SummaryLine } from "./schedule/Side";
import { BTN } from "./settings/parts";
import { loadHome, setSkipped, WEEK_KEY, type FirstRun, type Home, type Step, type Week } from "./schedule/api";

const POLL_MS = 30_000;
/** From this width the collection status sits beside the schedule; below it, a line above. */
const WIDE = "(min-width: 980px)";

const STEP_NAME: Record<Step, string> = { folder: "감시 폴더 등록", import: "기존 설정 가져오기" };

/** Reads the screen again every {@link POLL_MS} and when the page becomes visible. */
function usePolling(reload: () => void) {
  useEffect(() => {
    const timer = window.setInterval(reload, POLL_MS);
    const onVisible = () => document.visibilityState === "visible" && reload();
    document.addEventListener("visibilitychange", onVisible);
    return () => {
      window.clearInterval(timer);
      document.removeEventListener("visibilitychange", onVisible);
    };
  }, [reload]);
}

function useWide(): boolean {
  const [wide, setWide] = useState(() => window.matchMedia(WIDE).matches);
  useEffect(() => {
    const query = window.matchMedia(WIDE);
    const onChange = () => setWide(query.matches);
    query.addEventListener("change", onChange);
    return () => query.removeEventListener("change", onChange);
  }, []);
  return wide;
}

/** Scrolls to today's row and moves focus to it. */
function goToToday() {
  const row = document.getElementById(TODAY_ID);
  if (!row) return;
  row.scrollIntoView({ block: "start" });
  row.focus({ preventScroll: true });
}

function Meta({ week, now }: { week: Week; now: number }) {
  return (
    <p className="m-0 flex flex-wrap items-baseline gap-x-4 gap-y-0.5 text-[13px] text-text-secondary">
      <span>
        {monthDay(week.start)} ~ {monthDay(week.end)}
      </span>
      <span>{quarterName(week.quarter)}</span>
      <span className="text-text-muted">기준 오늘 {clock(now)}</span>
    </p>
  );
}

/** A step skipped just now, kept in view for taking it back after the checklist is gone. */
function SkippedNotice({ step, onUndo, busy }: { step: Step; onUndo: () => void; busy: boolean }) {
  return (
    <p
      role="status"
      className="mb-3 flex flex-wrap items-center gap-x-3 gap-y-1.5 rounded-card border border-hairline-soft bg-surface-1 px-3.5 py-2 text-[13px] text-text-secondary"
    >
      <span className="min-w-0 flex-1 basis-48">{STEP_NAME[step]} 단계를 건너뛰어서 처음 설정을 마쳤어요.</span>
      <button type="button" className={BTN.plain} disabled={busy} onClick={onUndo}>
        건너뛰기 취소
      </button>
    </p>
  );
}

/**
 * The app's first screen: this week's episodes of the subscribed anime by day,
 * with the collection status beside them (a line above on a phone). On a new
 * install the `처음 설정` checklist takes their place.
 */
export function ScheduleScreen() {
  const { data, error, slow, reload, update } = useCached(WEEK_KEY, loadHome, "이번 주 편성을 불러오지 못했어요.");
  usePolling(reload);
  const wide = useWide();
  const [skipped, setSkippedStep] = useState<Step | null>(null);
  const [undoing, setUndoing] = useState(false);

  /** The checklist as the server has it after a skip or its undo. */
  const changed = (next: FirstRun, step: Step, wasSkipped: boolean) => {
    if (next.active) {
      update((home: Home) => ({ ...home, first_run: next, week: null }));
      setSkippedStep(null);
      return;
    }
    // The checklist is over: the schedule is read, and the skip stays undoable here.
    update((home: Home) => ({ ...home, first_run: null, week: null }));
    setSkippedStep(wasSkipped ? step : null);
    reload();
  };

  const undo = async () => {
    if (!skipped) return;
    setUndoing(true);
    try {
      const next = await setSkipped(skipped, false);
      update((home: Home) => ({ ...home, first_run: next.active ? next : null, week: next.active ? null : home.week }));
      setSkippedStep(null);
    } catch {
      // The notice stays with its button for another try.
    } finally {
      setUndoing(false);
    }
  };

  if (!data) {
    return (
      <ScreenFrame title="이번 주 편성">
        {error ? <EmptyState>{error}</EmptyState> : slow ? <EmptyState>편성을 불러오는 중이에요.</EmptyState> : null}
      </ScreenFrame>
    );
  }

  if (data.first_run) {
    return (
      <ScreenFrame title="이번 주 편성">
        <Checklist firstRun={data.first_run} onChange={changed} />
      </ScreenFrame>
    );
  }

  const week = data.week;
  if (!week) {
    // The checklist just ended and the schedule is on its way.
    return (
      <ScreenFrame title="이번 주 편성">
        {skipped && <SkippedNotice step={skipped} onUndo={undo} busy={undoing} />}
        {error ? <EmptyState>{error}</EmptyState> : slow ? <EmptyState>편성을 불러오는 중이에요.</EmptyState> : null}
      </ScreenFrame>
    );
  }

  const none = week.days.every((day) => day.cards.length === 0);
  return (
    <ScreenFrame
      title="이번 주 편성"
      actions={
        <button type="button" className={BTN.plain} onClick={goToToday}>
          오늘로 이동
        </button>
      }
      meta={<Meta week={week} now={data.now} />}
    >
      {skipped && <SkippedNotice step={skipped} onUndo={undo} busy={undoing} />}
      {!wide && <SummaryLine week={week} />}
      <div className="grid min-w-0 grid-cols-1 items-start gap-5 min-[980px]:grid-cols-[minmax(0,1fr)_288px]">
        <div className="min-w-0">
          {none && (
            <p className="mb-3 text-[13px] leading-relaxed text-text-muted">
              이번 주에 방영하는 구독이 없어요. 수집 화면에서 방영작을 구독하면 여기에 나타나요.
            </p>
          )}
          <ol
            aria-label={`${monthDay(week.start)}부터 ${monthDay(week.end)}까지 요일별 편성`}
            className="m-0 flex list-none flex-col gap-1.5 p-0"
            data-testid="week-days"
          >
            {week.days.map((day) => (
              <DayRow key={day.date} day={day} />
            ))}
          </ol>
        </div>
        {wide && <SideColumn week={week} />}
      </div>
    </ScreenFrame>
  );
}
