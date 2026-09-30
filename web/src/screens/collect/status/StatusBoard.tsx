import { useEffect, useId, useState, type ReactNode } from "react";
import { Link } from "react-router-dom";

import { useCached } from "@/lib/cached";
import { ago } from "@/lib/time";
import { cn } from "@/lib/utils";

import { KEYS } from "../cache";
import { channelName } from "../rules/api";
import { loadBoard, type Board } from "./api";

const POLL_MS = 30_000;
const WEEKDAY = ["일", "월", "화", "수", "목", "금", "토"];
/** The history filter the `실패·중복` link opens. */
export const PROBLEMS_HREF = "/collect/history?result=add_failed,duplicate";

interface Load {
  board: Board | undefined;
  /** The last read failed. With a board shown, it is the previous one. */
  failed: boolean;
  /** Nothing is cached and the first read is slow. */
  slow: boolean;
}

/**
 * The board: the copy from the last visit shows at once and is read again
 * behind it, then every {@link POLL_MS} and when the page becomes visible.
 */
function useBoard(): Load {
  const { data, error, slow, reload } = useCached(KEYS.status, loadBoard, "상태를 불러오지 못했어요.");
  useEffect(() => {
    const timer = window.setInterval(reload, POLL_MS);
    const onVisible = () => document.visibilityState === "visible" && reload();
    document.addEventListener("visibilitychange", onVisible);
    return () => {
      window.clearInterval(timer);
      document.removeEventListener("visibilitychange", onVisible);
    };
  }, [reload]);
  return { board: data, failed: error !== null, slow };
}

function weekday(date: string): string {
  const [y, m, d] = date.split("-").map(Number);
  return WEEKDAY[new Date(y, m - 1, d).getDay()];
}

function Cell({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex min-w-0 flex-col gap-1">
      <h2 className="text-xs font-semibold text-text-muted">{label}</h2>
      {children}
    </div>
  );
}

function Feeds({ board }: { board: Board }) {
  const { channels } = board;
  if (channels.length === 0) {
    return (
      <>
        <p className="text-[15px] font-bold">채널 없음</p>
        <p className="text-xs text-text-muted">채널 탭에서 RSS 채널을 추가해요.</p>
      </>
    );
  }
  const read = channels.filter((c) => c.ok !== null);
  const good = channels.filter((c) => c.ok === true).length;
  const failing = channels.filter((c) => c.ok === false);
  const latest = read.reduce<number | null>((m, c) => (c.read_at !== null && (m === null || c.read_at > m) ? c.read_at : m), null);
  return (
    <>
      <p className="text-[15px] font-bold">
        {read.length === 0 ? "아직 읽지 않았어요" : `${channels.length}개 중 ${good}개 정상`}
      </p>
      <p className={cn("text-xs break-all", failing.length > 0 ? "font-semibold text-urgent" : "text-text-muted")}>
        {failing.length > 0
          ? `읽지 못함: ${failing.map(channelName).join(", ")}`
          : latest !== null
            ? `마지막 확인 ${ago(latest, board.now)}`
            : "worker가 확인하면 여기에 보여요."}
      </p>
    </>
  );
}

function Bars({ board }: { board: Board }) {
  const { days, total } = board.received;
  const max = Math.max(1, ...days.map((d) => d.count));
  return (
    <>
      <p className="text-[15px] font-bold">{total}개 받음</p>
      <div
        role="img"
        aria-label={`최근 7일 받은 개수: ${days.map((d) => `${weekday(d.date)}요일 ${d.count}개`).join(", ")}`}
        className="mt-0.5 flex h-[38px] items-end gap-1"
        data-testid="day-bars"
      >
        {days.map((day, i) => (
          <div key={day.date} className="flex h-full min-w-0 flex-1 flex-col justify-end gap-0.5" title={`${day.date} ${day.count}개`}>
            <div
              className={cn("w-full rounded-sm", day.count > 0 ? "bg-focus" : "bg-surface-3")}
              style={{ height: day.count > 0 ? `${Math.max(12, (day.count / max) * 100)}%` : "2px" }}
            />
            <span
              aria-hidden="true"
              className={cn("text-center text-[10px] leading-none text-text-muted", i === days.length - 1 && "font-bold text-text-secondary")}
            >
              {weekday(day.date)}
            </span>
          </div>
        ))}
      </div>
    </>
  );
}

function Transmission({ board }: { board: Board }) {
  const t = board.transmission;
  if (t === null) {
    return (
      <>
        <p className="text-[15px] font-bold">아직 없음</p>
        <p className="text-xs text-text-muted">worker가 확인하면 여기에 보여요.</p>
      </>
    );
  }
  return (
    <>
      <p className="text-[15px] font-bold">
        받는 중 {t.downloading}개 · 시딩 {t.seeding}개
      </p>
      <p className="text-xs text-text-muted">확인 {ago(t.taken_at, board.now)}</p>
    </>
  );
}

function ProblemsLink({ count, className }: { count: number; className?: string }) {
  if (count === 0) return null;
  return (
    <Link
      to={PROBLEMS_HREF}
      className={cn(
        "inline-flex min-h-8 flex-none items-center rounded-full border border-[color-mix(in_srgb,var(--accent-urgent)_50%,transparent)] px-3 text-[13px] font-semibold whitespace-nowrap text-urgent hover:border-urgent",
        className,
      )}
    >
      실패·중복 {count}개
    </Link>
  );
}

/** One line for phones: the numbers that matter, in the order of the board. */
function summaryOf(board: Board): string {
  const feeds = board.channels.length === 0 ? "채널 없음" : `RSS ${board.channels.filter((c) => c.ok === true).length}/${board.channels.length}`;
  const t = board.transmission;
  return [
    feeds,
    `7일 ${board.received.total}개`,
    t ? `받는 중 ${t.downloading} · 시딩 ${t.seeding}` : "Transmission 아직 없음",
  ].join(" · ");
}

/**
 * The strip above the collection tabs, the same for every tab. Everything on
 * it is read from the server's stored snapshots; nothing here reads a feed or
 * Transmission. On phones it folds to one line that opens on tap.
 */
export function StatusBoard() {
  const { board, failed, slow } = useBoard();
  const [open, setOpen] = useState(false);
  const panelId = useId();

  const body = board ? (
    <div className="grid min-w-0 grid-cols-[minmax(0,1fr)_minmax(0,1.1fr)_minmax(0,1.2fr)] gap-x-6 gap-y-3 max-[720px]:grid-cols-1">
      <Cell label="RSS 채널">
        <Feeds board={board} />
      </Cell>
      <Cell label="최근 7일 받은 항목">
        <Bars board={board} />
      </Cell>
      <Cell label="Transmission">
        <Transmission board={board} />
      </Cell>
    </div>
  ) : failed || slow ? (
    <p className="text-[13px] text-text-muted">{failed ? "상태를 불러오지 못했어요." : "상태를 불러오는 중이에요."}</p>
  ) : null;

  return (
    <section
      aria-label="수집 상태"
      data-testid="status-board"
      className="mb-3 min-w-0 rounded-card border border-hairline-soft bg-surface-1 shadow-(--card-shadow) min-[721px]:min-h-[112px]"
    >
      {/* Phones: one line that opens the board. */}
      <div className="flex min-w-0 items-center gap-2 px-3.5 py-2 min-[721px]:hidden">
        <button
          type="button"
          aria-expanded={open}
          aria-controls={panelId}
          onClick={() => setOpen((v) => !v)}
          className="flex min-h-9 min-w-0 flex-1 items-center gap-2 text-left text-[13px] font-semibold"
        >
          <span className="min-w-0 flex-1 truncate">{board ? summaryOf(board) : failed ? "상태를 불러오지 못했어요." : slow ? "상태를 불러오는 중" : ""}</span>
          <span aria-hidden="true" className={cn("flex-none text-text-muted transition-transform", open && "rotate-180")}>
            ▾
          </span>
        </button>
        {board && <ProblemsLink count={board.problems} />}
      </div>

      <div id={panelId} className={cn("px-4 py-3.5", "max-[720px]:border-t max-[720px]:border-hairline-soft", !open && "max-[720px]:hidden")}>
        {body}
        {failed && board && <p className="mt-2 text-xs text-urgent">최신 상태를 불러오지 못해 이전 값을 보여줘요.</p>}
      </div>

      {/* PC and tablet: the link sits under the board so the rows do not move. */}
      {board && board.problems > 0 && (
        <div className="border-t border-hairline-soft px-4 py-2 max-[720px]:hidden">
          <ProblemsLink count={board.problems} />
        </div>
      )}
    </section>
  );
}
