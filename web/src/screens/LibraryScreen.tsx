import { useDeferredValue, useMemo } from "react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { useCached } from "@/lib/cached";
import { cn } from "@/lib/utils";

import { EmptyState, ScreenFrame } from "./ScreenFrame";
import { btnNeutral, inputClass } from "./collect/channels/styles";
import { loadWorks, WORKS_KEY, type LibraryWorkList } from "./library/api";
import { GridIcon, ListIcon } from "./library/icons";
import { FILTERS, prepare, selectWorks, sortWorks, SORTS, type FilterKey, type SortKey } from "./library/model";
import { useLibraryPrefs, type ViewKey } from "./library/prefs";
import { WorkItems } from "./library/WorkItem";

const LOAD_FAILED = "작품 목록을 불러오지 못했어요.";

const VIEWS: { key: ViewKey; label: string; Icon: typeof GridIcon }[] = [
  { key: "grid", label: "표지", Icon: GridIcon },
  { key: "list", label: "목록", Icon: ListIcon },
];

/**
 * Library: every work in one list, as a cover grid or a list, with a sort, a
 * filter and a title search. The whole answer is kept in memory (see
 * `lib/cached.ts`), so sorting and searching never wait for the server and
 * coming back from a work shows the list, tall enough to scroll to where it was,
 * in the first render.
 */
export function LibraryScreen() {
  const prefs = useLibraryPrefs();
  const works = useCached<LibraryWorkList>(WORKS_KEY, loadWorks, LOAD_FAILED);

  const all = useMemo(() => (works.data ? prepare(works.data.works) : null), [works.data]);
  const sorted = useMemo(() => (all ? sortWorks(all, prefs.sort) : null), [all, prefs.sort]);
  // The list follows typing a moment later, so the field itself never lags.
  const search = useDeferredValue(prefs.search);
  const shown = useMemo(
    () => (sorted ? selectWorks(sorted, prefs.filter, search) : null),
    [sorted, prefs.filter, search],
  );

  let body;
  if (shown === null) {
    body =
      works.error !== null ? (
        <div className="flex flex-col items-start gap-2.5">
          <p role="alert" className="text-[13px] font-semibold text-urgent">
            {works.error}
          </p>
          <Button type="button" variant="ghost" className={btnNeutral} onClick={works.reload}>
            재시도
          </Button>
        </div>
      ) : works.slow ? (
        <p className="text-[13px] text-text-muted">작품을 불러오는 중이에요.</p>
      ) : null;
  } else if (all!.length === 0) {
    body = <EmptyState>아직 발견한 작품이 없어요. 감시 폴더를 연결하면 작품이 여기에 나타나요.</EmptyState>;
  } else if (shown.length === 0) {
    body =
      prefs.filter === "airing" ? (
        <EmptyState>방영 정보를 아직 알 수 없어서 방영 중인 작품을 가려낼 수 없어요.</EmptyState>
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
      <div className={shown === null || search !== prefs.search ? "opacity-80" : undefined}>
        <WorkItems works={shown} view={prefs.view} />
      </div>
    );
  }

  return (
    <ScreenFrame title="라이브러리">
      {all !== null && all.length > 0 && (
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

          {shown !== null && (
            <p role="status" className="text-[13px] text-text-muted">
              작품 {shown.length}편
            </p>
          )}
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
