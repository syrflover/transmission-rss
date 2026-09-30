import { useRef, type KeyboardEvent } from "react";
import { useNavigate } from "react-router-dom";

import { cn } from "@/lib/utils";

import { PANEL_ID, tabId, type CollectTab } from "./tabs";

interface CollectTabsProps {
  tabs: readonly CollectTab[];
  /** The `path` of the tab whose URL is open. */
  current: string;
}

/**
 * The tab row. Arrow keys, Home and End only move keyboard focus between the
 * tabs; the tab (and so the URL) changes when a tab is activated with Enter,
 * Space or a click. Only the current tab is in the Tab order, so Tab leaves the
 * row for the panel.
 */
export function CollectTabs({ tabs, current }: CollectTabsProps) {
  const navigate = useNavigate();
  const buttons = useRef<(HTMLButtonElement | null)[]>([]);

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const from = buttons.current.findIndex((b) => b === document.activeElement);
    if (from < 0) return;
    let to: number;
    switch (event.key) {
      case "ArrowRight":
        to = (from + 1) % tabs.length;
        break;
      case "ArrowLeft":
        to = (from + tabs.length - 1) % tabs.length;
        break;
      case "Home":
        to = 0;
        break;
      case "End":
        to = tabs.length - 1;
        break;
      default:
        return;
    }
    event.preventDefault();
    buttons.current[to]?.focus();
  };

  return (
    <div
      role="tablist"
      aria-label="수집 작업"
      onKeyDown={onKeyDown}
      className="sticky top-(--topbar-h) z-40 mx-[calc(var(--gutter)*-1)] flex justify-start gap-1 border-b border-hairline-soft bg-[color-mix(in_srgb,var(--bg-void)_92%,transparent)] px-(--gutter) backdrop-blur-[12px] max-[720px]:justify-between max-[720px]:gap-0"
    >
      {tabs.map((tab, index) => {
        const selected = tab.path === current;
        return (
          <button
            key={tab.path}
            ref={(el) => {
              buttons.current[index] = el;
            }}
            type="button"
            role="tab"
            id={tabId(tab.path)}
            aria-selected={selected}
            aria-controls={PANEL_ID}
            tabIndex={selected ? 0 : -1}
            onClick={() => navigate(`/collect/${tab.path}`)}
            className={cn(
              "-mb-px inline-flex items-center justify-center border-b-[3px] border-transparent px-4 pt-[13px] pb-3 text-[15px] font-medium whitespace-nowrap text-text-secondary hover:text-text-primary",
              "max-[720px]:flex-1 max-[720px]:basis-0 max-[720px]:px-0.5 max-[720px]:pt-3 max-[720px]:pb-[11px] max-[720px]:text-[14.5px]",
              "aria-selected:border-focus aria-selected:font-bold aria-selected:text-text-primary",
            )}
          >
            <span className="bold-reserve" data-text={tab.label}>
              {tab.label}
            </span>
          </button>
        );
      })}
    </div>
  );
}
