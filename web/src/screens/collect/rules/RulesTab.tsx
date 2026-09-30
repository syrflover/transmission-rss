import { useCallback, useLayoutEffect, useRef, useState } from "react";
import { useLocation, useNavigate, useSearchParams } from "react-router-dom";

import { Button } from "@/components/ui/button";
import { peek, store, useCached } from "@/lib/cached";
import { cn } from "@/lib/utils";

import { KEYS, ruleCountChanged, withoutRule } from "../cache";
import { btnNeutral } from "../channels/styles";
import { listRules, ruleTitle, type Rule, type RuleList as Loaded } from "./api";
import { RuleDetail } from "./RuleDetail";
import { RuleList, type SortKey } from "./RuleList";

const DISCARD = "저장하지 않은 변경이 있어요. 버리고 옮길까요?";

const PHONE = "(max-width: 720px)";

/**
 * On a phone the list and a rule are separate screens on one page: opening a
 * rule starts it at the top instead of wherever the long list was scrolled to,
 * and going back to the list returns to the row that was tapped. Returns the
 * function to call, before the list is left, to remember where it was.
 */
function useListScroll(detailOpen: boolean): () => void {
  const saved = useRef<number | null>(null);
  const was = useRef(detailOpen);
  useLayoutEffect(() => {
    if (was.current === detailOpen) return;
    was.current = detailOpen;
    if (!window.matchMedia(PHONE).matches) return;
    if (detailOpen) window.scrollTo({ top: 0, left: 0, behavior: "instant" });
    else if (saved.current !== null) window.scrollTo({ top: saved.current, left: 0, behavior: "instant" });
  }, [detailOpen]);
  return () => {
    if (!was.current) saved.current = window.scrollY;
  };
}

/**
 * The 규칙 tab: every channel's rules in one list, and the selected rule
 * beside it (a screen of its own on phones).
 *
 * The URL says what is open, so the list keeps its place while a rule is
 * picked (`?rule=<id>` does not change the path and so does not scroll the
 * page):
 *
 * - `/collect/rules` the list;
 * - `/collect/rules?rule=<id>` a stored rule;
 * - `/collect/rules/new?channel=<id>&match=<title>` a new rule, prefilled.
 *   Nothing is saved until the user saves. Ticket 0008 links here from a
 *   history item.
 */
export function RulesTab() {
  const rules = useCached<Loaded>(KEYS.rules, listRules, "규칙을 불러오지 못했어요.");
  const [sort, setSortState] = useState<SortKey>(() => peek<SortKey>(KEYS.ruleSort) ?? "order");
  const setSort = (next: SortKey) => {
    store(KEYS.ruleSort, next);
    setSortState(next);
  };
  const [notice, setNotice] = useState<string | null>(null);
  const dirty = useRef(false);
  const navigate = useNavigate();
  const { pathname } = useLocation();
  const [params, setParams] = useSearchParams();

  const isNewRule = pathname.replace(/\/+$/, "").endsWith("/rules/new");
  const selectedId = isNewRule ? null : params.get("rule");

  const detailOpen =
    isNewRule || (selectedId !== null && (rules.data?.rules.some((r) => r.id === selectedId) ?? false));
  const rememberListScroll = useListScroll(detailOpen);

  const onDirtyChange = useCallback((value: boolean) => {
    dirty.current = value;
  }, []);

  /**
   * A rule was saved or archived: the cached list takes the saved rule at once
   * (its version decides the next save), then the list is read again for what
   * the change did to the other rules (overlaps, order).
   */
  const changed = (saved?: Rule) => {
    if (saved) {
      rules.update((list) => ({
        ...list,
        rules: list.rules.map((r) => (r.id === saved.id ? saved : r)),
      }));
    }
    rules.reload();
  };

  /** Runs `go` unless the user declines to drop unsaved changes. */
  const leave = (go: () => void) => {
    if (dirty.current && !window.confirm(DISCARD)) return;
    dirty.current = false;
    go();
  };

  const open = (rule: Rule) => {
    if (rule.id === selectedId) return;
    setNotice(null);
    leave(() => {
      rememberListScroll();
      if (isNewRule) navigate(`/collect/rules?rule=${encodeURIComponent(rule.id)}`);
      else setParams({ rule: rule.id });
    });
  };
  const close = () => leave(() => navigate("/collect/rules"));
  const openNew = () => {
    setNotice(null);
    const first = rules.data?.channels[0]?.id;
    leave(() => {
      rememberListScroll();
      navigate(`/collect/rules/new${first ? `?channel=${encodeURIComponent(first)}` : ""}`);
    });
  };

  if (rules.data === undefined) {
    if (rules.error !== null) {
      return (
        <div className="flex flex-col items-start gap-2.5">
          <p role="alert" className="text-[13px] font-semibold text-urgent">
            {rules.error}
          </p>
          <Button type="button" variant="ghost" className={btnNeutral} onClick={rules.reload}>
            재시도
          </Button>
        </div>
      );
    }
    // A quick answer never shows the loading line.
    return rules.slow ? <p className="text-[13px] text-text-muted">규칙을 불러오는 중이에요.</p> : null;
  }

  const { rules: ruleList, channels } = rules.data;
  const selected = selectedId ? ruleList.find((r) => r.id === selectedId) : undefined;
  const missing = selectedId !== null && selected === undefined;
  const presetChannel = params.get("channel");
  const presetMatch = params.get("match") ?? "";

  const column = "min-w-0 min-[721px]:max-h-[calc(100dvh-var(--topbar-h)-96px)] min-[721px]:overflow-y-auto min-[721px]:pr-1";

  return (
    <div className="flex min-w-0 flex-col gap-3">
      <p role="status" className="text-[13px] font-semibold text-text-secondary empty:hidden">
        {missing ? "그 규칙을 찾지 못했어요. 삭제됐을 수 있어요." : notice}
      </p>
      <div className="grid min-w-0 grid-cols-[minmax(260px,360px)_minmax(0,1fr)] items-start gap-6 max-[720px]:grid-cols-1">
        <div className={cn(column, detailOpen && "max-[720px]:hidden")}>
          <RuleList
            rules={ruleList}
            channels={channels}
            selectedId={selected?.id ?? null}
            sort={sort}
            onSort={setSort}
            onSelect={open}
            onNew={openNew}
          />
        </div>

        <div className={cn(column, !detailOpen && "max-[720px]:hidden")}>
          {selected ? (
            <RuleDetail
              key={selected.id}
              rule={selected}
              channels={channels}
              rules={ruleList}
              onChanged={changed}
              onCreated={() => undefined}
              onDeleted={(rule) => {
                // The cached list drops the rule at once; the read confirms it.
                rules.update((list) => withoutRule(list, rule.id));
                ruleCountChanged(rule.channel_id, -1);
                rules.reload();
                dirty.current = false;
                setNotice(`${ruleTitle(rule)} 규칙을 삭제했어요.`);
                navigate("/collect/rules", { replace: true });
              }}
              onBack={close}
              onDirtyChange={onDirtyChange}
            />
          ) : isNewRule ? (
            <RuleDetail
              key={`new:${presetChannel}:${presetMatch}`}
              rule={null}
              channels={channels}
              rules={ruleList}
              presetChannelId={presetChannel}
              presetMatch={presetMatch}
              onChanged={changed}
              onCreated={(created) => {
                dirty.current = false;
                ruleCountChanged(created.channel_id, 1);
                // Read the list first so the new rule is in it when it is opened.
                listRules().then(
                  (data) => {
                    rules.update(data);
                    navigate(`/collect/rules?rule=${encodeURIComponent(created.id)}`, { replace: true });
                  },
                  () => {
                    rules.reload();
                    navigate("/collect/rules", { replace: true });
                  },
                );
              }}
              onDeleted={() => undefined}
              onBack={close}
              onDirtyChange={onDirtyChange}
            />
          ) : (
            <p className="rounded-card border border-dashed border-hairline px-4 py-10 text-center text-[13.5px] leading-relaxed text-text-muted max-[720px]:hidden">
              목록에서 규칙을 고르면 여기서 고치고, 저장하기 전에 무엇이 받아질지 미리 볼 수 있어요.
            </p>
          )}
        </div>
      </div>
    </div>
  );
}
