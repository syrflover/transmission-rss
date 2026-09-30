import { useMemo } from "react";

import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

import { EmptyState } from "../../ScreenFrame";
import { PlusIcon } from "../icons";
import { btnAction } from "../channels/styles";
import { channelName, ruleTitle, type ChannelBrief, type Rule } from "./api";

export type SortKey = "order" | "title";

export const SORTS: { key: SortKey; label: string }[] = [
  { key: "order", label: "검사 순서" },
  { key: "title", label: "제목순" },
];

/**
 * The rules in the order shown. This only arranges the list on screen: the
 * order the worker checks rules in is each rule's `order` within its channel
 * and no sort changes it.
 */
export function sortRules(rules: Rule[], channels: ChannelBrief[], sort: SortKey): Rule[] {
  const channelPosition = new Map(channels.map((c) => [c.id, c.position]));
  const byOrder = (a: Rule, b: Rule) =>
    (channelPosition.get(a.channel_id) ?? 0) - (channelPosition.get(b.channel_id) ?? 0) ||
    a.channel_id.localeCompare(b.channel_id) ||
    a.order - b.order;
  const copy = [...rules];
  if (sort === "order") return copy.sort(byOrder);
  return copy.sort(
    (a, b) =>
      Number(a.match === null) - Number(b.match === null) ||
      ruleTitle(a).localeCompare(ruleTitle(b), "ko") ||
      byOrder(a, b),
  );
}

const badge =
  "inline-flex flex-none items-center rounded-full border px-2 py-px text-[11.5px] leading-tight font-semibold";

/** 수집 중 / 보관됨 / 제목 대기 */
export function StateBadge({ rule }: { rule: Pick<Rule, "state" | "match"> }) {
  if (rule.state === "archived") return <span className={cn(badge, "border-arch text-arch")}>보관됨</span>;
  if (rule.match === null || rule.match === "")
    return <span className={cn(badge, "border-focus text-focus")}>제목 대기</span>;
  return <span className={cn(badge, "border-ok text-ok")}>수집 중</span>;
}

export function ChannelTag({ channel }: { channel: ChannelBrief | undefined }) {
  return (
    <span className="inline-flex min-w-0 max-w-full items-center rounded-full bg-surface-3 px-2 py-px text-[11.5px] leading-tight font-medium text-text-secondary">
      <span className="truncate">{channel ? channelName(channel) : "채널 없음"}</span>
    </span>
  );
}

interface RuleListProps {
  rules: Rule[];
  channels: ChannelBrief[];
  selectedId: string | null;
  sort: SortKey;
  onSort: (sort: SortKey) => void;
  onSelect: (rule: Rule) => void;
  onNew: () => void;
}

/** Every channel's rules in one list, with a sort that only changes what is shown. */
export function RuleList({ rules, channels, selectedId, sort, onSort, onSelect, onNew }: RuleListProps) {
  const shown = useMemo(() => sortRules(rules, channels, sort), [rules, channels, sort]);
  const channelById = useMemo(() => new Map(channels.map((c) => [c.id, c])), [channels]);

  return (
    <div className="flex min-w-0 flex-col gap-3">
      <div className="flex flex-wrap items-center justify-between gap-x-3 gap-y-2">
        <div role="group" aria-label="정렬" className="flex items-center gap-1 rounded-full bg-surface-2 p-0.5">
          {SORTS.map((s) => (
            <button
              key={s.key}
              type="button"
              aria-pressed={sort === s.key}
              onClick={() => onSort(s.key)}
              className={cn(
                "min-h-8 rounded-full px-3 text-[13px] font-medium text-text-secondary max-[720px]:min-h-9",
                "aria-pressed:bg-surface-1 aria-pressed:font-bold aria-pressed:text-text-primary aria-pressed:shadow-(--card-shadow)",
              )}
            >
              {s.label}
            </button>
          ))}
        </div>
        <Button type="button" variant="ghost" className={btnAction} onClick={onNew} disabled={channels.length === 0}>
          <PlusIcon className="size-[15px]" />
          규칙 추가
        </Button>
      </div>

      {sort === "order" && channels.length > 1 && (
        <p className="text-xs leading-normal text-text-muted">
          채널마다 위에서부터 차례로 검사해요. 두 규칙이 같은 항목에 맞으면 앞 규칙이 가져가요.
        </p>
      )}

      {shown.length === 0 ? (
        <EmptyState>
          {channels.length === 0
            ? "아직 채널이 없어요. 채널 탭에서 RSS 채널을 추가하면 그 채널의 규칙을 여기서 만들 수 있어요."
            : "아직 규칙이 없어요. 규칙을 추가하면 모든 채널의 규칙이 여기에 한 목록으로 모여요."}
        </EmptyState>
      ) : (
        <ul className="m-0 flex list-none flex-col gap-2 p-0" aria-label="규칙 목록">
          {shown.map((rule) => {
            const selected = rule.id === selectedId;
            return (
              <li key={rule.id}>
                <button
                  type="button"
                  aria-current={selected ? "true" : undefined}
                  onClick={() => onSelect(rule)}
                  className={cn(
                    "flex w-full min-w-0 flex-col gap-1.5 rounded-card border border-hairline-soft bg-surface-1 px-3.5 py-3 text-left shadow-(--card-shadow) hover:border-hairline",
                    "aria-[current=true]:border-focus aria-[current=true]:bg-surface-2",
                  )}
                >
                  <span
                    className={cn(
                      "min-w-0 text-[14.5px] leading-snug font-semibold break-all",
                      rule.match === null && "text-text-secondary",
                    )}
                  >
                    {ruleTitle(rule)}
                  </span>
                  <span className="flex min-w-0 flex-wrap items-center gap-1.5">
                    <StateBadge rule={rule} />
                    <ChannelTag channel={channelById.get(rule.channel_id)} />
                    {rule.overlap && rule.state === "active" && (
                      <span
                        className={cn(badge, "border-hairline text-text-secondary")}
                        title="다른 규칙이 먼저 가져가는 항목이 있어요"
                      >
                        겹침
                      </span>
                    )}
                    {rule.error && <span className={cn(badge, "border-urgent text-urgent")}>정규식 오류</span>}
                  </span>
                </button>
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}
