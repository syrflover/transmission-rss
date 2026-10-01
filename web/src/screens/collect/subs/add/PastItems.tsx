import { Button } from "@/components/ui/button";
import { dateTime } from "@/lib/time";

import { btnNeutral, hintClass } from "../../channels/styles";
import type { PreviewItem } from "../../rules/api";
import { PHASE_TEXT, type ReceivePhase } from "./useReceive";

const check = "mt-0.5 size-[18px] flex-none accent-focus";

/**
 * The past items of a channel that a rule would take but that no rule has
 * received: the ones the user may tick. A new rule lists them as `mine`, a
 * subscription whose creation or title came after them as `past`.
 */
export function receivable(item: PreviewItem): boolean {
  return (item.kind === "mine" || item.kind === "past") && item.stored_result === "no_match";
}

/** The items a past-items list shows: what the rule would take, ticked or not. */
export function listed(items: PreviewItem[]): PreviewItem[] {
  return items.filter((i) => i.kind === "mine" || i.kind === "past");
}

/**
 * The recorded items of a rule, unticked until the user ticks them. Nothing
 * here receives anything: the caller sends only the ticked ones.
 */
export function PastChecklist({
  items,
  ticked,
  onTicked,
  label,
}: {
  items: PreviewItem[];
  ticked: ReadonlySet<number>;
  onTicked: (next: ReadonlySet<number>) => void;
  label: string;
}) {
  const pickable = items.filter(receivable);
  const tickedNow = pickable.filter((i) => ticked.has(i.id));
  const toggle = (id: number) => {
    const next = new Set(ticked);
    if (!next.delete(id)) next.add(id);
    onTicked(next);
  };

  return (
    <>
      {pickable.length > 0 && (
        <div className="flex flex-wrap items-center gap-2">
          <Button
            type="button"
            variant="ghost"
            className={btnNeutral}
            onClick={() => onTicked(new Set(pickable.map((i) => i.id)))}
          >
            모두 선택
          </Button>
          <Button type="button" variant="ghost" className={btnNeutral} onClick={() => onTicked(new Set())}>
            선택 해제
          </Button>
          <span className="text-xs text-text-muted" role="status">
            {tickedNow.length}개 선택
          </span>
        </div>
      )}

      {items.length > 0 && (
        <ul className="m-0 flex list-none flex-col gap-2 p-0" aria-label={label}>
          {items.map((item) => {
            const can = receivable(item);
            return (
              <li key={item.id}>
                <label
                  className={
                    can
                      ? "flex min-w-0 cursor-pointer items-start gap-3 rounded-[10px] border border-hairline-soft bg-surface-2 px-3 py-2.5 has-[:checked]:border-focus"
                      : "flex min-w-0 items-start gap-3 rounded-[10px] border border-hairline-soft bg-surface-2 px-3 py-2.5 opacity-70"
                  }
                >
                  <input
                    type="checkbox"
                    className={check}
                    disabled={!can}
                    checked={can && ticked.has(item.id)}
                    onChange={() => toggle(item.id)}
                  />
                  <span className="flex min-w-0 flex-col gap-0.5">
                    <span className="min-w-0 text-[13px] leading-snug break-all">{item.title}</span>
                    <span className="text-xs text-text-muted">
                      {dateTime(item.first_seen_at)} 기록
                      {!can && " · 이미 처리된 항목이라 받을 수 없어요"}
                    </span>
                  </span>
                </label>
              </li>
            );
          })}
        </ul>
      )}
    </>
  );
}

/** How each ticked item's receive goes, one row each, until the worker ends it. */
export function ReceiveProgress({
  entries,
  titleOf,
  onRetry,
}: {
  entries: { itemId: number; phase: ReceivePhase }[];
  titleOf: (itemId: number) => string;
  onRetry: (itemId: number) => void;
}) {
  if (entries.length === 0) return null;
  const settled = entries.every((e) => e.phase.kind === "added" || e.phase.kind === "failed");
  return (
    <>
      <ul className="m-0 flex list-none flex-col gap-2 p-0" aria-label="지난 항목 받기">
        {entries.map((entry) => (
          <li
            key={entry.itemId}
            className="flex min-w-0 flex-col gap-1 rounded-[10px] border border-hairline-soft bg-surface-2 px-3 py-2.5"
          >
            <p className="min-w-0 text-[13px] leading-snug break-all">{titleOf(entry.itemId)}</p>
            <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
              <span
                role="status"
                className={
                  entry.phase.kind === "failed"
                    ? "text-xs font-semibold text-urgent"
                    : entry.phase.kind === "added"
                      ? "text-xs font-semibold text-ok"
                      : "text-xs font-semibold text-text-secondary"
                }
              >
                {PHASE_TEXT[entry.phase.kind]}
              </span>
              {entry.phase.kind === "failed" && (
                <>
                  <span className="min-w-0 text-xs break-words text-text-secondary">{entry.phase.message}</span>
                  <Button type="button" variant="ghost" className={btnNeutral} onClick={() => onRetry(entry.itemId)}>
                    다시 받기
                  </Button>
                </>
              )}
            </div>
          </li>
        ))}
      </ul>
      {!settled && <p className={hintClass}>worker가 하나씩 추가해요. 이 화면을 떠나도 계속돼요.</p>}
    </>
  );
}
