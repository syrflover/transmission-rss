import { useEffect, useRef } from "react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { cn } from "@/lib/utils";

import { EmptyState, ScreenFrame } from "./ScreenFrame";
import { btnNeutral, inputClass } from "./collect/channels/styles";
import type { FilterKey, SortKey } from "./library/api";
import { GridIcon, ListIcon } from "./library/icons";
import { FILTERS, SORTS } from "./library/model";
import { useDebounced, useWorkPages } from "./library/pages";
import { useLibraryPrefs, type ViewKey } from "./library/prefs";
import { WorkItems } from "./library/WorkItem";

/** How long typing has to stop before the search is sent. */
const SEARCH_PAUSE_MS = 250;
/** The next page is read this far before the end of the list comes into view. */
const READ_AHEAD = "0px 0px 600px 0px";

const VIEWS: { key: ViewKey; label: string; Icon: typeof GridIcon }[] = [
  { key: "grid", label: "표지", Icon: GridIcon },
  { key: "list", label: "목록", Icon: ListIcon },
];

/**
 * Library: the works as a cover grid or a list, with a sort, a filter and a
 * title search. The server sorts, filters and searches and answers one page at
 * a time; the next page is read when the end of the list comes into view (see
 * `library/pages.ts`). Pages already read stay in memory, so coming back from a
 * work shows them, tall enough to scroll to where the page was, in the first render.
 */
export function LibraryScreen() {
  const prefs = useLibraryPrefs();
  // The list follows typing once it pauses, so the field itself never lags and a request is not sent per key.
  const search = useDebounced(prefs.search, SEARCH_PAUSE_MS);
  const pages = useWorkPages({ sort: prefs.sort, filter: prefs.filter, search });
  const data = pages.data;

  let body;
  if (data === undefined) {
    body =
      pages.error !== null ? (
        <LoadError message={pages.error} onRetry={pages.reload} />
      ) : pages.slow ? (
        <p className="text-[13px] text-text-muted">작품을 불러오는 중이에요.</p>
      ) : null;
  } else if (pages.error !== null) {
    body = <LoadError message={pages.error} onRetry={pages.reload} />;
  } else if (data.libraryCount === 0) {
    body = <EmptyState>아직 발견한 작품이 없어요. 감시 폴더를 연결하면 작품이 여기에 나타나요.</EmptyState>;
  } else if (data.works.length === 0 && !pages.stale) {
    body =
      prefs.filter === "airing" ? (
        <EmptyState>방영 중인 작품이 없어요. 최신 시즌에 방영 중인 AniList 항목을 연결한 작품만 여기에 나와요.</EmptyState>
      ) : (
        <div className="flex flex-col items-start gap-2.5">
          <EmptyState>조건에 맞는 작품이 없어요.</EmptyState>
          <Button
            type="button"
            variant="ghost"
            className={btnNeutral}
            onClick={() => {
              prefs.setSearch("");
              prefs.setFilter("all");
            }}
          >
            검색과 필터 지우기
          </Button>
        </div>
      );
  } else {
    body = (
      <>
        <div className={pages.stale || search !== prefs.search ? "opacity-80" : undefined}>
          <WorkItems works={data.works} view={prefs.view} />
        </div>
        <EndOfList pages={pages} />
      </>
    );
  }

  return (
    <ScreenFrame title="라이브러리">
      {data !== undefined && data.libraryCount > 0 && (
        <div className="flex flex-col gap-3 pb-4">
          <div className="flex flex-wrap items-center gap-2.5">
            <Input
              type="search"
              value={prefs.search}
              onChange={(e) => prefs.setSearch(e.target.value)}
              aria-label="작품명 검색"
              placeholder="작품명 검색"
              autoComplete="off"
              spellCheck={false}
              className={cn(inputClass, "min-w-0 flex-1 basis-56")}
            />
            <select
              aria-label="정렬"
              value={prefs.sort}
              onChange={(e) => prefs.setSort(e.target.value as SortKey)}
              className={cn(inputClass, "w-[170px] max-w-full border py-1")}
            >
              {SORTS.map((s) => (
                <option key={s.key} value={s.key}>
                  {s.label}
                </option>
              ))}
            </select>
            <div role="group" aria-label="보기" className="flex items-center gap-1 rounded-full bg-surface-2 p-0.5">
              {VIEWS.map(({ key, label, Icon }) => (
                <button
                  key={key}
                  type="button"
                  aria-pressed={prefs.view === key}
                  onClick={() => prefs.setView(key)}
                  className={cn(
                    "inline-flex min-h-8 items-center gap-1.5 rounded-full px-3 text-[13px] font-medium text-text-secondary max-[720px]:min-h-9",
                    "aria-pressed:bg-surface-1 aria-pressed:font-bold aria-pressed:text-text-primary aria-pressed:shadow-(--card-shadow)",
                  )}
                >
                  <Icon className="size-4" />
                  {label}
                </button>
              ))}
            </div>
          </div>

          <div role="group" aria-label="필터" className="flex flex-wrap gap-2">
            {FILTERS.map((f) => (
              <FilterChip
                key={f.key}
                label={f.label}
                pressed={prefs.filter === f.key}
                onClick={() => prefs.setFilter(f.key as FilterKey)}
              />
            ))}
          </div>

          <p role="status" className="text-[13px] text-text-muted">
            작품 {data.total}편
          </p>
        </div>
      )}
      {body}
    </ScreenFrame>
  );
}

function FilterChip(props: { label: string; pressed: boolean; onClick: () => void }) {
  return (
    <button
      type="button"
      aria-pressed={props.pressed}
      onClick={props.onClick}
      className="inline-flex min-h-[34px] items-center rounded-full border border-hairline bg-surface-1 px-[13px] text-[13px] font-semibold whitespace-nowrap text-text-secondary hover:border-text-muted hover:text-text-primary aria-pressed:border-text-primary aria-pressed:bg-text-primary aria-pressed:text-surface-1 max-[720px]:min-h-[38px]"
    >
      {props.label}
    </button>
  );
}

function LoadError(props: { message: string; onRetry: () => void }) {
  return (
    <div className="flex flex-col items-start gap-2.5">
      <p role="alert" className="text-[13px] font-semibold text-urgent">
        {props.message}
      </p>
      <Button type="button" variant="ghost" className={btnNeutral} onClick={props.onRetry}>
        재시도
      </Button>
    </div>
  );
}

/**
 * What follows the last loaded work: an empty strip that reads the next page when it
 * comes within reach of the viewport, and the line for a read that is slow or failed.
 * It sits below the list, so appending a page never moves what is above it.
 */
function EndOfList({ pages }: { pages: ReturnType<typeof useWorkPages> }) {
  const { data, loadMore, moreFailed } = pages;
  const strip = useRef<HTMLDivElement>(null);
  const loaded = data?.works.length ?? 0;
  const watching = data?.next != null && !moreFailed;

  // Watching starts again after each page: a strip that is still within reach reports at once.
  useEffect(() => {
    const element = strip.current;
    if (!watching || element === null) return;
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((entry) => entry.isIntersecting)) loadMore();
      },
      { rootMargin: READ_AHEAD },
    );
    observer.observe(element);
    return () => observer.disconnect();
  }, [watching, loaded, loadMore]);

  return (
    <>
      <div ref={strip} aria-hidden="true" className="h-px" />
      {pages.loadingMore && <p className="pt-4 text-[13px] text-text-muted">작품을 더 불러오는 중이에요.</p>}
      {moreFailed && (
        <div className="flex flex-col items-start gap-2.5 pt-4">
          <p role="alert" className="text-[13px] font-semibold text-urgent">
            다음 작품을 불러오지 못했어요.
          </p>
          <Button type="button" variant="ghost" className={btnNeutral} onClick={loadMore}>
            다시 시도
          </Button>
        </div>
      )}
    </>
  );
}
