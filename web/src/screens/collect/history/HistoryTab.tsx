import { Fragment, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useNavigate, useSearchParams } from "react-router-dom";

import { Button } from "@/components/ui/button";
import { patch, useCached } from "@/lib/cached";

import { EmptyState } from "../../ScreenFrame";
import { KEYS } from "../cache";
import { listChannels, channelTitle, type Channel } from "../channels/api";
import { btnNeutral, inputClass } from "../channels/styles";
import {
  listHistory,
  type HistoryCounts,
  type HistoryFilter,
  type HistoryItem,
  type HistoryPage,
} from "./api";
import {
  RESULT_CHIPS,
  chipCount,
  customLabel,
  readFilter,
  sameResults,
  writeFilter,
} from "./filters";
import { dayHeading, dayKey } from "./format";
import { HistoryRow } from "./HistoryRow";

/** Rows fetched per request. */
const PAGE_SIZE = 50;

type More = "idle" | "loading" | "failed";

/**
 * What the list shows: the first page (kept in the cache per filter, so coming
 * back to the tab shows it at once) and the pages scrolled in after it.
 */
interface View {
  items: HistoryItem[];
  /** The cursor for the next page; `null` on the last one. */
  next: string | null;
  counts: HistoryCounts;
}

/** The pages after the first, which are only kept while the tab is open. */
interface Extra {
  /** The filter these rows were fetched for. */
  key: string;
  /** The first page's cursor they continue from; they are dropped when a newer first page ends elsewhere. */
  anchor: string;
  items: HistoryItem[];
  next: string | null;
}

const NO_COUNTS: HistoryCounts = {
  total: 0,
  received: 0,
  no_match: 0,
  excluded: 0,
  duplicate: 0,
  add_failed: 0,
};

/**
 * The 기록 tab: what the worker did with every RSS item, newest first, with
 * filters by result and channel kept in the URL (`?result=add_failed,duplicate&channel=<id>`).
 * The list pages in by cursor as it is scrolled, so rows already shown keep
 * their place while more are added below.
 */
export function HistoryTab() {
  const [params] = useSearchParams();
  const navigate = useNavigate();
  const filter = useMemo(() => readFilter(params), [params]);
  const key = writeFilter(filter);
  const cacheKey = KEYS.history(key);

  const first = useCached<HistoryPage>(
    cacheKey,
    () => listHistory(filter, { limit: PAGE_SIZE }),
    "기록을 불러오지 못했어요.",
  );
  const channelList = useCached<Channel[]>(KEYS.channels, listChannels, "");
  // Without the list the channel filter only offers the chosen channel; the list itself still works.
  const channels = channelList.data ?? [];
  const [extra, setExtra] = useState<Extra | null>(null);
  const [moreState, setMoreState] = useState<{ key: string; state: More }>({ key, state: "idle" });
  const more = moreState.key === key ? moreState.state : "idle";
  // Answers for a filter that is no longer chosen are dropped.
  const latest = useRef(key);
  latest.current = key;

  const page = first.data;
  const ready = page !== undefined;
  const view = useMemo<View | null>(() => {
    if (!page) return null;
    if (!extra || extra.key !== key || extra.anchor !== page.next) return page;
    // Rows already shown are kept as they are; a repeated ID is not added twice.
    const seen = new Set(page.items.map((i) => i.id));
    return {
      items: [...page.items, ...extra.items.filter((i) => !seen.has(i.id))],
      next: extra.next,
      counts: page.counts,
    };
  }, [page, extra, key]);
  // While another filter loads, the list of the last one stays, dimmed.
  const lastView = useRef<View | null>(null);
  if (view) lastView.current = view;
  const shown = view ?? lastView.current;
  const next = view ? view.next : null;

  const loadMore = useCallback(() => {
    if (!page || !view || view.next === null) return;
    const chosenKey = key;
    const anchor = page.next!;
    setMoreState({ key: chosenKey, state: "loading" });
    listHistory(filter, { after: view.next, limit: PAGE_SIZE }).then(
      (answer) => {
        if (latest.current !== chosenKey) return;
        setExtra((was) => {
          const before = was && was.key === chosenKey && was.anchor === anchor ? was.items : [];
          return { key: chosenKey, anchor, items: [...before, ...answer.items], next: answer.next };
        });
        setMoreState({ key: chosenKey, state: "idle" });
      },
      () => {
        if (latest.current === chosenKey) setMoreState({ key: chosenKey, state: "failed" });
      },
    );
  }, [page, view, key, filter]);

  // Load the next page when the end of the list comes near.
  const sentinel = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const node = sentinel.current;
    if (!node || next === null || more !== "idle") return;
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((entry) => entry.isIntersecting)) loadMore();
      },
      { rootMargin: "600px 0px" },
    );
    observer.observe(node);
    return () => observer.disconnect();
  }, [next, more, loadMore]);

  const refreshCounts = useCallback(() => {
    const chosenKey = latest.current;
    listHistory({ results: [], channel: readFilter(new URLSearchParams(chosenKey)).channel }, { limit: 1 }).then(
      (answer) => {
        if (latest.current !== chosenKey) return;
        patch<HistoryPage>(KEYS.history(chosenKey), (was) => ({ ...was, counts: answer.counts }));
      },
      () => {},
    );
  }, []);

  /** A row changed (the worker finished a command): the row is replaced in place, so nothing moves. */
  const replaceItem = useCallback(
    (item: HistoryItem) => {
      const swap = (items: HistoryItem[]) => items.map((old) => (old.id === item.id ? item : old));
      // The cached first page keeps the new row too, for the next visit.
      patch<HistoryPage>(KEYS.history(latest.current), (was) => ({ ...was, items: swap(was.items) }));
      setExtra((was) => (was ? { ...was, items: swap(was.items) } : was));
      refreshCounts();
    },
    [refreshCounts],
  );

  const choose = (next: HistoryFilter) => navigate({ search: writeFilter(next) }, { replace: true });

  const counts = page ? page.counts : NO_COUNTS;
  const knownChip = RESULT_CHIPS.find((chip) => sameResults(chip.results, filter.results));
  const channelKnown = filter.channel === null || channels.some((c) => c.id === filter.channel);

  let body;
  if (first.error !== null && !ready) {
    body = (
      <div className="flex flex-col items-start gap-2.5">
        <p role="alert" className="text-[13px] font-semibold text-urgent">
          {first.error}
        </p>
        <Button type="button" variant="ghost" className={btnNeutral} onClick={first.reload}>
          재시도
        </Button>
      </div>
    );
  } else if (!shown) {
    // A quick answer never shows the loading line.
    body = first.slow ? <p className="text-[13px] text-text-muted">기록을 불러오는 중이에요.</p> : null;
  } else if (shown.items.length === 0 && ready) {
    body =
      filter.results.length === 0 && filter.channel === null ? (
        <EmptyState>
          아직 수집 기록이 없어요. 채널의 새 항목을 확인하면 받은 항목과 받지 않은 항목이 시간순으로 쌓여요.
        </EmptyState>
      ) : (
        <div className="flex flex-col items-start gap-2.5">
          <EmptyState>조건에 맞는 기록이 없어요.</EmptyState>
          <Button type="button" variant="ghost" className={btnNeutral} onClick={() => choose({ results: [], channel: null })}>
            필터 지우기
          </Button>
        </div>
      );
  } else {
    body = (
      <>
        <ul className={ready ? "" : "opacity-60"} aria-busy={!ready}>
          {shown.items.map((item, index) => {
            const previous = shown.items[index - 1];
            const newDay = !previous || dayKey(previous.first_seen_at) !== dayKey(item.first_seen_at);
            return (
              <Fragment key={item.id}>
                {newDay && (
                  <li className="mt-5 mb-2 first:mt-1" data-day-heading>
                    <h3 className="text-[13px] font-bold text-text-secondary">{dayHeading(item.first_seen_at)}</h3>
                  </li>
                )}
                <HistoryRow item={item} onItem={replaceItem} />
              </Fragment>
            );
          })}
        </ul>
        <div ref={sentinel} className="flex min-h-12 items-center justify-center py-3 text-[13px] text-text-muted">
          {more === "loading" && "기록을 더 불러오는 중이에요."}
          {more === "failed" && (
            <span className="flex flex-wrap items-center justify-center gap-2.5">
              <span role="alert" className="font-semibold text-urgent">
                기록을 더 불러오지 못했어요.
              </span>
              <Button type="button" variant="ghost" className={btnNeutral} onClick={loadMore}>
                다시 불러오기
              </Button>
            </span>
          )}
          {more === "idle" && next === null && shown.items.length > 0 && ready && "가장 오래된 기록까지 봤어요."}
        </div>
      </>
    );
  }

  return (
    <div className="flex flex-col gap-3.5">
      <div className="flex flex-col gap-3">
        <div className="flex flex-wrap items-center gap-2.5">
          <select
            aria-label="채널로 거르기"
            value={filter.channel ?? ""}
            onChange={(e) => choose({ ...filter, channel: e.target.value || null })}
            className={`${inputClass} w-[190px] max-w-full rounded-[10px] border py-1`}
          >
            <option value="">모든 채널</option>
            {!channelKnown && <option value={filter.channel ?? ""}>삭제됐거나 알 수 없는 채널</option>}
            {channels.map((channel) => (
              <option key={channel.id} value={channel.id}>
                {channelTitle(channel)}
              </option>
            ))}
          </select>
        </div>

        <div role="group" aria-label="결과로 거르기" className="flex flex-wrap gap-2">
          {RESULT_CHIPS.map((chip) => (
            <FilterChip
              key={chip.id}
              label={chip.label}
              count={ready ? chipCount(chip, counts) : null}
              pressed={knownChip?.id === chip.id}
              onClick={() => choose({ ...filter, results: chip.results })}
            />
          ))}
          {!knownChip && (
            <FilterChip
              label={customLabel(filter.results)}
              count={null}
              pressed
              onClick={() => choose({ ...filter, results: [] })}
            />
          )}
        </div>
      </div>

      {body}
    </div>
  );
}

function FilterChip(props: { label: string; count: number | null; pressed: boolean; onClick: () => void }) {
  return (
    <button
      type="button"
      aria-pressed={props.pressed}
      onClick={props.onClick}
      className="inline-flex min-h-[34px] items-center gap-1.5 rounded-full border border-hairline bg-surface-1 px-[13px] text-[13px] font-semibold whitespace-nowrap text-text-secondary hover:border-text-muted hover:text-text-primary aria-pressed:border-text-primary aria-pressed:bg-text-primary aria-pressed:text-surface-1 max-[720px]:min-h-[38px]"
    >
      {props.label}
      {props.count !== null && (
        <span className="text-[11.5px] font-semibold opacity-75">{props.count}</span>
      )}
    </button>
  );
}
