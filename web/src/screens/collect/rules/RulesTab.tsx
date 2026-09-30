import { useCallback, useEffect, useRef, useState } from "react";
import { useLocation, useNavigate, useSearchParams } from "react-router-dom";

import { ApiError } from "@/lib/api";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

import { btnNeutral } from "../channels/styles";
import { listRules, ruleTitle, type Rule, type RuleList as Loaded } from "./api";
import { RuleDetail } from "./RuleDetail";
import { RuleList, type SortKey } from "./RuleList";

type Load =
  | { state: "loading" }
  | { state: "failed"; message: string }
  | { state: "ready"; data: Loaded };

const DISCARD = "저장하지 않은 변경이 있어요. 버리고 옮길까요?";

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
  const [load, setLoad] = useState<Load>({ state: "loading" });
  const [sort, setSort] = useState<SortKey>("order");
  const [notice, setNotice] = useState<string | null>(null);
  const dirty = useRef(false);
  const navigate = useNavigate();
  const { pathname } = useLocation();
  const [params, setParams] = useSearchParams();

  const isNewRule = pathname.replace(/\/+$/, "").endsWith("/rules/new");
  const selectedId = isNewRule ? null : params.get("rule");

  const fetchRules = useCallback((quiet: boolean) => {
    let current = true;
    if (!quiet) setLoad({ state: "loading" });
    listRules().then(
      (data) => current && setLoad({ state: "ready", data }),
      (e: unknown) =>
        current &&
        setLoad((prev) =>
          quiet && prev.state === "ready"
            ? prev
            : { state: "failed", message: e instanceof ApiError ? e.message : "규칙을 불러오지 못했어요." },
        ),
    );
    return () => {
      current = false;
    };
  }, []);
  useEffect(() => fetchRules(false), [fetchRules]);

  const onDirtyChange = useCallback((value: boolean) => {
    dirty.current = value;
  }, []);

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
      if (isNewRule) navigate(`/collect/rules?rule=${encodeURIComponent(rule.id)}`);
      else setParams({ rule: rule.id });
    });
  };
  const close = () => leave(() => navigate("/collect/rules"));
  const openNew = () => {
    setNotice(null);
    const first = load.state === "ready" ? load.data.channels[0]?.id : undefined;
    leave(() => navigate(`/collect/rules/new${first ? `?channel=${encodeURIComponent(first)}` : ""}`));
  };

  if (load.state === "loading") return <p className="text-[13px] text-text-muted">규칙을 불러오는 중이에요.</p>;
  if (load.state === "failed") {
    return (
      <div className="flex flex-col items-start gap-2.5">
        <p role="alert" className="text-[13px] font-semibold text-urgent">
          {load.message}
        </p>
        <Button type="button" variant="ghost" className={btnNeutral} onClick={() => fetchRules(false)}>
          재시도
        </Button>
      </div>
    );
  }

  const { rules, channels } = load.data;
  const selected = selectedId ? rules.find((r) => r.id === selectedId) : undefined;
  const missing = selectedId !== null && selected === undefined;
  const detailOpen = isNewRule || selected !== undefined;
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
            rules={rules}
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
              rules={rules}
              onChanged={() => fetchRules(true)}
              onCreated={() => undefined}
              onDeleted={(rule) => {
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
              rules={rules}
              presetChannelId={presetChannel}
              presetMatch={presetMatch}
              onChanged={() => fetchRules(true)}
              onCreated={(created) => {
                dirty.current = false;
                // Read the list first so the new rule is in it when it is opened.
                listRules().then(
                  (data) => {
                    setLoad({ state: "ready", data });
                    navigate(`/collect/rules?rule=${encodeURIComponent(created.id)}`, { replace: true });
                  },
                  () => navigate("/collect/rules", { replace: true }),
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
