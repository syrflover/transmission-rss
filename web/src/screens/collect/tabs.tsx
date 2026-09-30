import type { ReactNode } from "react";

import { ChannelsTab } from "./channels/ChannelsTab";
import { HistoryTab } from "./history/HistoryTab";
import { RulesTab } from "./rules/RulesTab";
import { SubsTab } from "./subs/SubsTab";

/**
 * The tabs of the collection screen, in display order. Each tab has its own URL
 * (`/collect/<path>`) and an `element` shown in the tab panel.
 *
 * Each tab's content lives in its own folder (`subs/`, `rules/`, `history/`,
 * `channels/`), so the tickets that fill them do not edit this list. The
 * status board that sits above the tab row for every tab belongs in
 * `CollectScreen`, before `<CollectTabs>`.
 */
export interface CollectTab {
  /** Also the URL segment and the base of the DOM ids. */
  path: "subs" | "rules" | "history" | "channels";
  label: string;
  element: ReactNode;
}

export const COLLECT_TABS: readonly CollectTab[] = [
  { path: "subs", label: "구독", element: <SubsTab /> },
  { path: "rules", label: "규칙", element: <RulesTab /> },
  { path: "history", label: "기록", element: <HistoryTab /> },
  { path: "channels", label: "채널", element: <ChannelsTab /> },
];

export const DEFAULT_TAB = COLLECT_TABS[0].path;

export const tabId = (path: string) => `collect-tab-${path}`;
export const PANEL_ID = "collect-panel";
