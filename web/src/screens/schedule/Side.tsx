import { useId, useState } from "react";
import { Link } from "react-router-dom";

import { when } from "@/lib/time";
import { cn } from "@/lib/utils";

import { useBoard, type Load } from "../collect/status/StatusBoard";
import type { Board } from "../collect/status/api";
import type { Week } from "./api";
import { quarterName } from "./format";

/** Where the schedule to add a subscription from opens: Anissia's `신작`. */
export const ADD_FROM_SCHEDULE = "/collect/subs/add?week=8";

/** The time the feeds were last read: the newest read of any channel. */
function lastRead(board: Board): number | null {
  return board.channels.reduce<number | null>(
    (latest, c) => (c.read_at !== null && (latest === null || c.read_at > latest) ? c.read_at : latest),
    null,
  );
}

function Row({ label, children, muted }: { label: string; children: React.ReactNode; muted?: boolean }) {
  return (
    <div className="flex min-w-0 items-baseline justify-between gap-3 text-[13px]">
      <dt className="flex-none text-text-muted">{label}</dt>
      <dd className={cn("m-0 min-w-0 text-right font-semibold", muted && "font-normal text-text-muted")}>{children}</dd>
    </div>
  );
}

/** The collection's state: feeds first, then what was added and what Transmission is doing. */
function CollectStatus({ load }: { load: Load }) {
  const { board, failed, slow } = load;
  return (
    <section aria-label="수집 상태" data-testid="week-collect-status" className="min-w-0 rounded-card border border-hairline-soft bg-surface-1 p-4 shadow-(--card-shadow)">
      <h2 className="mb-2.5 text-[13px] font-bold">수집 상태</h2>
      {!board ? (
        <p className="text-xs text-text-muted">{failed ? "상태를 불러오지 못했어요." : slow ? "상태를 불러오는 중이에요." : ""}</p>
      ) : (
        <>
          {!board.collect_folder_set && (
            <p role="status" className="mb-2.5 text-xs leading-normal font-semibold text-urgent">
              수집 폴더가 아직 없어서 새 항목을 받지 않고 있어요.{" "}
              <Link to="/settings/collection" className="underline underline-offset-2">
                수집 폴더 정하기
              </Link>
            </p>
          )}
          <dl className="m-0 flex flex-col gap-1.5">
            {board.channels.length === 0 ? (
              <Row label="RSS" muted>
                채널 없음
              </Row>
            ) : (
              <>
                <Row label="확인" muted={lastRead(board) === null}>
                  {lastRead(board) === null ? "아직 없음" : when(lastRead(board)!, board.now)}
                </Row>
                <Row label="다음" muted={!board.cycle?.next_at}>
                  {board.cycle?.next_at ? when(board.cycle.next_at, board.now) : "아직 없음"}
                </Row>
              </>
            )}
            <Row label="최근 7일 추가">{board.received.total}개</Row>
            <Row label="받는 중" muted={!board.transmission}>
              {board.transmission ? `${board.transmission.downloading}개` : "아직 없음"}
            </Row>
            <Row label="시딩" muted={!board.transmission}>
              {board.transmission ? `${board.transmission.seeding}개` : "아직 없음"}
            </Row>
            {/* The counts are the worker's last look; when it was shows as on the collect screen's board. */}
            {board.transmission && (
              <Row label="Transmission 확인" muted>
                {when(board.transmission.taken_at, board.now)}
              </Row>
            )}
          </dl>
          {failed && <p className="mt-2 text-xs text-urgent">최신 상태를 불러오지 못해 이전 값을 보여줘요.</p>}
        </>
      )}
    </section>
  );
}

/** The next quarter's subscriptions, with how many wait for a title, and the way to add more. */
function NextQuarter({ week }: { week: Week }) {
  const next = week.next_quarter;
  return (
    <section aria-label="다음 분기 구독" data-testid="week-next-quarter" className="min-w-0 rounded-card border border-hairline-soft bg-surface-1 p-4 shadow-(--card-shadow)">
      <h2 className="mb-0.5 text-[13px] font-bold">다음 분기 구독</h2>
      <p className="mb-2.5 text-xs text-text-muted">{quarterName(next.quarter)}</p>
      <dl className="m-0 mb-3 flex flex-col gap-1.5">
        <Row label="구독">{next.subscriptions}개</Row>
        <Row label="제목 대기" muted={next.title_waiting === 0}>
          {next.title_waiting}개
        </Row>
      </dl>
      <Link
        to={ADD_FROM_SCHEDULE}
        className="inline-flex h-8 items-center justify-center rounded-lg border border-focus px-3 text-[13px] font-medium text-focus hover:bg-surface-2"
      >
        편성표에서 추가
      </Link>
    </section>
  );
}

/** The narrow column beside the schedule on a wide screen. */
export function SideColumn({ week }: { week: Week }) {
  const load = useBoard();
  return (
    <aside aria-label="수집 상태와 다음 분기" className="flex min-w-0 flex-col gap-3 max-[979px]:hidden">
      <CollectStatus load={load} />
      <NextQuarter week={week} />
    </aside>
  );
}

/**
 * One line above the schedule on a phone: the collection status, cut off with an
 * ellipsis where the screen ends, and the next quarter's count, which stays.
 * It opens to the two panels the wide screen shows beside the schedule.
 */
export function SummaryLine({ week }: { week: Week }) {
  // One board, read once, for the line and for the panel it opens.
  const load = useBoard();
  const { board } = load;
  const [open, setOpen] = useState(false);
  const panelId = useId();
  const read = board ? lastRead(board) : null;
  const next = week.next_quarter;
  const item = (key: string, label: string, value: string) => (
    <span key={key} className="mr-2.5 whitespace-nowrap">
      <span className="text-text-muted">{label}</span> <span className="font-semibold">{value}</span>
    </span>
  );
  return (
    <section aria-label="수집 상태" data-testid="week-summary" className="mb-3 min-w-0 min-[980px]:hidden">
      <button
        type="button"
        aria-expanded={open}
        aria-controls={panelId}
        onClick={() => setOpen((v) => !v)}
        className="flex min-h-9 w-full min-w-0 items-center gap-2 rounded-card border border-hairline-soft bg-surface-1 px-2.5 py-1.5 text-left text-[12px] shadow-(--card-shadow)"
      >
        <span className="block min-w-0 flex-1 truncate">
          {board ? (
            <>
              {read !== null && item("read", "확인", when(read, board.now))}
              {board.cycle?.next_at ? item("next", "다음", when(board.cycle.next_at, board.now)) : null}
              {item("seven", "7일", String(board.received.total))}
              {board.transmission ? item("down", "받는 중", String(board.transmission.downloading)) : null}
              {board.transmission ? item("seed", "시딩", String(board.transmission.seeding)) : null}
            </>
          ) : (
            <span className="mr-3 text-text-muted">수집 상태</span>
          )}
        </span>
        <span className="flex-none whitespace-nowrap">
          <span className="text-text-muted">다음 분기</span> <span className="font-semibold">{next.subscriptions}</span>
        </span>
        <span aria-hidden="true" className={cn("flex-none text-text-muted transition-transform", open && "rotate-180")}>
          ▾
        </span>
      </button>
      <div id={panelId} className={cn("mt-2 flex-col gap-3", open ? "flex" : "hidden")}>
        {open && (
          <>
            <CollectStatus load={load} />
            <NextQuarter week={week} />
          </>
        )}
      </div>
    </section>
  );
}
