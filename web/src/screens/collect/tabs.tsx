import type { ReactNode } from "react";

import { EmptyState } from "../ScreenFrame";
import { ChannelsTab } from "./channels/ChannelsTab";

/**
 * The tabs of the collection screen, in display order. Each tab has its own URL
 * (`/collect/<path>`) and an `element` shown in the tab panel.
 *
 * Later tickets fill their tab by replacing its `element`: the subscription
 * and rules tabs (0006) and the history tab (0008). The status board that sits
 * above the tab row for every tab belongs in `CollectScreen`, before
 * `<CollectTabs>`.
 */
export interface CollectTab {
  /** Also the URL segment and the base of the DOM ids. */
  path: "subs" | "rules" | "history" | "channels";
  label: string;
  element: ReactNode;
}

export const COLLECT_TABS: readonly CollectTab[] = [
  {
    path: "subs",
    label: "구독",
    element: (
      <EmptyState>
        아직 구독한 작품이 없어요. 구독하면 작품마다 받을 채널과 저장 폴더가 여기에 모여요.
      </EmptyState>
    ),
  },
  {
    path: "rules",
    label: "규칙",
    element: (
      <EmptyState>
        아직 규칙이 없어요. 규칙을 만들면 모든 채널의 규칙이 여기에 한 목록으로 모여요.
      </EmptyState>
    ),
  },
  {
    path: "history",
    label: "기록",
    element: (
      <EmptyState>
        아직 수집 기록이 없어요. 채널의 새 항목을 확인하면 받은 항목과 받지 않은 항목이 시간순으로 쌓여요.
      </EmptyState>
    ),
  },
  { path: "channels", label: "채널", element: <ChannelsTab /> },
];

export const DEFAULT_TAB = COLLECT_TABS[0].path;

export const tabId = (path: string) => `collect-tab-${path}`;
export const PANEL_ID = "collect-panel";
