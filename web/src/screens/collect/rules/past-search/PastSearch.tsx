import { useEffect, useId, useMemo, useState } from "react";

import { Link } from "react-router-dom";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { cn } from "@/lib/utils";

import { btnAction, btnNeutral, hintClass, inputClass, labelClass } from "../../channels/styles";
import { PickRow, ProgressRows, SelectionBar } from "../../subs/add/PastItems";
import { useCommandRows } from "../../subs/add/useReceive";
import {
  rangeLabel,
  receivePast,
  type ResultItem,
  type ResultState,
  type SearchContext,
  type SearchResult,
} from "./api";
import { usePastSearch } from "./usePastSearch";

const STATE: Record<ResultState, { label: string; badge: string }> = {
  missing: { label: "받을 회차", badge: "border-ok text-ok" },
  replace: { label: "수정본 교체", badge: "border-arch text-arch" },
  version_unknown: { label: "버전 미상", badge: "border-arch text-arch" },
  have: { label: "이미 있어요", badge: "border-hairline text-text-secondary" },
  superseded: { label: "낮은 수정본", badge: "border-hairline text-text-muted" },
  alternate: { label: "다른 릴리스", badge: "border-hairline text-text-muted" },
  batch: { label: "배치", badge: "border-hairline text-text-secondary" },
  unnumbered: { label: "회차 불명", badge: "border-hairline text-text-muted" },
};

const STEPS = ["범위 확인", "검색 결과 미리보기", "받기"];

function digits(text: string): number | null {
  const trimmed = text.trim();
  return /^\d{1,6}$/.test(trimmed) ? Number(trimmed) : null;
}

/** The three steps, with the current one marked. */
function Steps({ current }: { current: 0 | 1 | 2 }) {
  return (
    <ol className="m-0 flex list-none flex-wrap gap-x-2 gap-y-1 p-0 text-xs" aria-label="진행 단계">
      {STEPS.map((name, index) => (
        <li
          key={name}
          aria-current={index === current ? "step" : undefined}
          className={cn(
            "inline-flex items-center gap-1.5 rounded-full border px-2.5 py-0.5 font-semibold",
            index === current ? "border-focus text-focus" : "border-hairline text-text-muted",
          )}
        >
          <span aria-hidden>{index + 1}</span>
          {name}
        </li>
      ))}
    </ol>
  );
}

/** Step 1: the search words and the release range the person confirms. */
function RangeStep({
  context,
  error,
  disabled,
  onSearch,
  onClose,
}: {
  context: SearchContext;
  error: string | null;
  disabled: string | null;
  onSearch: (body: { query: string; from: number; to: number }) => void;
  onClose: () => void;
}) {
  const uid = useId();
  const [query, setQuery] = useState(context.query);
  const [from, setFrom] = useState(context.suggestion.from?.toString() ?? "");
  const [to, setTo] = useState(context.suggestion.to?.toString() ?? "");
  const a = digits(from);
  const b = digits(to);
  const valid = query.trim() !== "" && a !== null && b !== null && a >= 1 && a <= b;
  const blocked = disabled ?? context.blocked;

  return (
    <div className="flex min-w-0 flex-col gap-3">
      <p className="text-[13px] leading-normal text-text-secondary">
        RSS에서 빠진 지난 회차를 채널의 검색으로 한 번 찾아봐요. 검색 결과는 고르기 전에는 아무것도 받지 않고, 찾은 것은 이 화면에만 남아요.
      </p>

      <div className="flex min-w-0 flex-col gap-1.5">
        <Label htmlFor={`${uid}-q`} className={labelClass}>
          검색어
        </Label>
        <Input
          id={`${uid}-q`}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          className={inputClass}
          placeholder="[SubsPlease] 작품 제목 1080p"
          autoComplete="off"
          autoCapitalize="none"
          spellCheck={false}
          aria-describedby={`${uid}-q-hint`}
        />
        <p id={`${uid}-q-hint`} className={hintClass}>
          {context.empty_because ??
            (context.from_format ? (
              <>
                채널의 검색 형식 <span className="font-mono break-all">{context.format}</span>에 일치 문구를 넣었어요.
              </>
            ) : (
              <>
                이 채널에는 지난 회차 검색 형식이 없어서 일치 문구로 채웠어요. 릴리스 그룹이나 화질을 넣어 이번 검색만 고칠 수 있어요.{" "}
                <Link to="/collect/channels" className="font-semibold text-focus underline underline-offset-2">
                  채널 탭
                </Link>
                에서 형식을 정해 두면 다음부터 채워져요.
              </>
            ))}
        </p>
      </div>

      <fieldset className="m-0 flex min-w-0 flex-col gap-1.5 border-0 p-0">
        <legend className={`${labelClass} p-0`}>릴리스 회차 범위</legend>
        <div className="flex min-w-0 flex-wrap items-center gap-2">
          <Input
            aria-label="범위 시작"
            value={from}
            onChange={(e) => setFrom(e.target.value)}
            className={`${inputClass} w-24 font-mono`}
            inputMode="numeric"
            autoComplete="off"
          />
          <span aria-hidden>–</span>
          <Input
            aria-label="범위 끝"
            value={to}
            onChange={(e) => setTo(e.target.value)}
            className={`${inputClass} w-24 font-mono`}
            inputMode="numeric"
            autoComplete="off"
          />
          {valid && (
            <span className="min-w-0 font-mono text-[13px] font-semibold break-all" data-testid="past-range">
              릴리스 {a}–{b} → {rangeLabel(a, b, context.offset, context.season)}
            </span>
          )}
        </div>
        <p className={hintClass} data-testid="past-range-basis">
          {context.suggestion.basis}
        </p>
        {a !== null && b !== null && a > b && (
          <p role="alert" className="text-xs font-semibold text-urgent">
            범위의 시작이 끝보다 클 수 없어요.
          </p>
        )}
      </fieldset>

      {error && (
        <p role="alert" className="text-[13px] font-semibold text-urgent">
          {error}
        </p>
      )}
      {blocked && <p className={hintClass}>{blocked}</p>}

      <div className="flex flex-wrap items-center gap-2">
        <Button
          type="button"
          variant="ghost"
          className={btnAction}
          disabled={!valid || blocked !== null}
          onClick={() => valid && onSearch({ query: query.trim(), from: a, to: b })}
        >
          검색
        </Button>
        <Button type="button" variant="ghost" className={btnNeutral} onClick={onClose}>
          닫기
        </Button>
      </div>
    </div>
  );
}

function Row({ item, picked, onToggle }: { item: ResultItem; picked: boolean; onToggle: () => void }) {
  const state = STATE[item.state];
  return (
    <PickRow can={item.selectable} checked={picked} onChange={onToggle}>
      <span className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1">
        <span
          className={cn(
            "inline-flex flex-none items-center rounded-full border px-2 py-px text-[11.5px] font-semibold",
            state.badge,
          )}
        >
          {state.label}
        </span>
        {item.release !== null && (
          <span className="flex-none font-mono text-[11.5px] text-text-secondary">
            릴리스 {item.release}
            {item.half ? ".5" : ""}
            {item.version > 1 ? `v${item.version}` : ""}
            {item.folder ? ` → ${item.folder}` : ""}
          </span>
        )}
      </span>
      <span className="min-w-0 text-[13px] leading-snug break-all" data-testid="past-title">
        {item.title}
      </span>
      {item.note && <span className="min-w-0 text-xs leading-normal text-text-secondary">{item.note}</span>}
    </PickRow>
  );
}

/** Steps 2 and 3: the judged results, and receiving the ones the person ticks. */
function ResultStep({
  context,
  searchId,
  result,
  disabled,
  onHold,
  onAgain,
  onClose,
  onAdded,
}: {
  context: SearchContext;
  searchId: string;
  result: SearchResult;
  disabled: string | null;
  onHold: () => void;
  onAgain: () => void;
  onClose: () => void;
  onAdded: (count: number) => void;
}) {
  const initial = useMemo(() => new Set(result.items.filter((i) => i.selected).map((i) => i.key)), [result]);
  const [picked, setPicked] = useState<ReadonlySet<string>>(initial);
  const titleOf = useMemo(() => new Map(result.items.map((i) => [i.key, i] as const)), [result]);

  const { rows, start, retry } = useCommandRows<string>(context.rule_id, (commandId, key, rule) =>
    receivePast(commandId, rule, searchId, key),
  );
  const receiving = rows.length > 0;
  const added = rows.filter((r) => r.phase.kind === "added").length;
  useEffect(() => onAdded(added), [added, onAdded]);

  const chosen = result.items.filter((i) => picked.has(i.key) && i.selectable);
  const replacing = chosen.filter((i) => i.state === "replace" || i.state === "version_unknown").length;
  const toggle = (key: string) => {
    const next = new Set(picked);
    if (!next.delete(key)) next.add(key);
    setPicked(next);
  };
  const receive = () => {
    if (chosen.length === 0) return;
    onHold();
    start(
      context.rule_id,
      chosen.map((i) => i.key),
    );
  };
  const blocked = disabled ?? context.blocked;
  const label = rangeLabel(result.from, result.to, context.offset, context.season);

  return (
    <div className="flex min-w-0 flex-col gap-3">
      <Steps current={receiving ? 2 : 1} />
      <div className="flex min-w-0 flex-col gap-1.5" data-testid="past-summary">
        <p className="text-[13px] leading-normal text-text-secondary">
          릴리스 <strong>{result.from}–{result.to}</strong> (<span className="font-mono">{label}</span>)에서 결과{" "}
          <strong>{result.items.length}개</strong>를 보여요.
          {result.out_of_range > 0 && <> 범위 밖 {result.out_of_range}개는 접어 두었어요.</>}
          {result.not_picked > 0 && <> 규칙이 고르지 않는 {result.not_picked}개는 뺐어요.</>}
        </p>
        {result.extra_needed > 0 && (
          <p className={hintClass}>
            첫 검색 결과가 가득 차서 빠진 회차를 묶어 {result.extra_sent}번 더 검색하고 합쳤어요.
          </p>
        )}
        {result.missing.length > 0 && (
          <p className={hintClass}>
            범위에서 아직 없는 회차: {result.missing_ranges.join(", ")} ({result.missing.length}개)
            {result.not_found.length > 0 && <> · 검색 결과에도 없는 회차: {result.not_found_ranges.join(", ")}</>}
          </p>
        )}
        {result.missing.length === 0 && <p className={hintClass}>범위의 회차가 모두 있어요.</p>}
        {result.notes.map((note) => (
          <p key={note} role="status" className="text-xs leading-normal font-semibold text-text-secondary">
            {note}
          </p>
        ))}
      </div>

      {!receiving && result.items.length > 0 && (
        <>
          <SelectionBar
            count={chosen.length}
            allLabel="기본 선택"
            onAll={() => setPicked(initial)}
            onNone={() => setPicked(new Set())}
          />
          <ul className="m-0 flex list-none flex-col gap-2 p-0" aria-label="검색 결과">
            {result.items.map((item) => (
              <Row key={item.key} item={item} picked={picked.has(item.key)} onToggle={() => toggle(item.key)} />
            ))}
          </ul>
        </>
      )}
      {!receiving && result.items.length === 0 && (
        <p className="text-[13px] text-text-secondary">범위 안에서 규칙이 고르는 결과가 없어요.</p>
      )}

      {receiving && (
        <ProgressRows
          label="지난 회차 받기"
          entries={rows.map((r) => ({ key: r.key, title: titleOf.get(r.key)?.title ?? r.key, phase: r.phase }))}
          onRetry={retry}
        />
      )}

      {!receiving && replacing > 0 && (
        <p role="status" className="text-xs leading-normal font-semibold text-text-secondary">
          고른 것 중 {replacing}개는 폴더의 영상을 바꿔요. 받고 확인이 끝나면 이전 영상이 지워져요.
        </p>
      )}
      {!receiving && blocked && <p className={hintClass}>{blocked}</p>}

      <div className="flex flex-wrap items-center gap-2">
        {!receiving && (
          <Button
            type="button"
            variant="ghost"
            className={btnAction}
            disabled={chosen.length === 0 || blocked !== null}
            onClick={receive}
          >
            선택한 {chosen.length}개 받기
          </Button>
        )}
        <Button type="button" variant="ghost" className={btnNeutral} onClick={onAgain}>
          범위 다시 정하기
        </Button>
        <Button type="button" variant="ghost" className={btnNeutral} onClick={onClose}>
          닫기
        </Button>
      </div>
    </div>
  );
}

/**
 * `지난 회차 검색` of the rule detail (`docs/specs/collection.md`): the range
 * the person confirms, the judged results, and receiving the ones they tick.
 * The server searches and judges; a search is never saved as a channel.
 */
export function PastSearch({
  ruleId,
  disabled,
  onAdded,
}: {
  ruleId: string;
  /** Why a search or a receive cannot go now (unsaved edits), or `null`. */
  disabled: string | null;
  /** The number of results added so far, so the page can read its history again. */
  onAdded: (count: number) => void;
}) {
  const search = usePastSearch(ruleId);
  const { phase } = search;

  return (
    <section
      aria-labelledby="past-search-heading"
      data-testid="past-search"
      className="flex min-w-0 flex-col gap-3 border-t border-hairline-soft pt-4"
    >
      <div className="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1">
        <h3 id="past-search-heading" className="text-[15px] font-bold">
          지난 회차 검색
        </h3>
        {(phase.kind === "range" || phase.kind === "searching") && (
          <Steps current={phase.kind === "range" ? 0 : 1} />
        )}
      </div>

      {phase.kind === "closed" && (
        <div className="flex min-w-0 flex-wrap items-center gap-2">
          <Button type="button" variant="ghost" className={btnNeutral} onClick={() => void search.open()}>
            지난 회차 찾기
          </Button>
          <span className="min-w-0 flex-1 basis-48 text-xs text-text-muted">
            RSS에 남아 있지 않은 회차를 채널의 검색으로 찾아 골라 받아요.
          </span>
        </div>
      )}

      {phase.kind === "loading" && <p className="text-[13px] text-text-muted">검색을 준비하는 중이에요.</p>}

      {phase.kind === "unavailable" && (
        <div className="flex min-w-0 flex-col gap-2">
          <p role="alert" className="text-[13px] font-semibold text-urgent">
            {phase.message}
          </p>
          <div>
            <Button type="button" variant="ghost" className={btnNeutral} onClick={() => void search.open()}>
              다시 시도
            </Button>
          </div>
        </div>
      )}

      {phase.kind === "range" && (
        <RangeStep
          key={phase.context.rule_id}
          context={phase.context}
          error={phase.error}
          disabled={disabled}
          onSearch={(body) => void search.begin(phase.context, body)}
          onClose={search.close}
        />
      )}

      {phase.kind === "searching" && (
        <div className="flex min-w-0 flex-col gap-2.5" data-testid="past-searching">
          <p role="status" className="text-[13px] leading-normal text-text-secondary">
            검색하는 중이에요.
            {phase.needed > 0
              ? ` 첫 결과가 가득 차서 빠진 회차를 묶어 더 찾고 있어요 (${phase.sent}/${phase.needed}).`
              : " 결과가 많으면 요청 사이에 간격을 두어서 1분쯤 걸릴 수 있어요."}
          </p>
          <div>
            <Button
              type="button"
              variant="ghost"
              className={btnNeutral}
              onClick={() => search.back(phase.context)}
            >
              검색 취소
            </Button>
          </div>
        </div>
      )}

      {phase.kind === "preview" && (
        <ResultStep
          key={phase.searchId}
          context={phase.context}
          searchId={phase.searchId}
          result={phase.result}
          disabled={disabled}
          onHold={search.hold}
          onAgain={() => search.back(phase.context)}
          onClose={search.close}
          onAdded={onAdded}
        />
      )}
    </section>
  );
}
