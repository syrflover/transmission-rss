import { useEffect } from "react";
import { Navigate, Route, Routes, useLocation } from "react-router-dom";

import { store } from "@/lib/cached";

import { ScreenFrame } from "./ScreenFrame";
import { KEYS } from "./collect/cache";
import { CollectTabs } from "./collect/CollectTabs";
import { StatusBoard } from "./collect/status/StatusBoard";
import { COLLECT_TABS, DEFAULT_TAB, PANEL_ID, tabId } from "./collect/tabs";

/**
 * Collection screen: four tabs, each with its own URL under `/collect`.
 * `/collect` and unknown sub-paths open the first tab.
 */
export function CollectScreen() {
  const { pathname, search } = useLocation();
  const segment = pathname.split("/")[2];
  const current = COLLECT_TABS.find((tab) => tab.path === segment)?.path ?? DEFAULT_TAB;

  // A tab is opened again the way it was left: the rule that was open, the
  // history filter. (A new, unsaved rule is not kept.)
  const isNew = pathname.replace(/\/+$/, "").endsWith("/new");
  useEffect(() => {
    if (!isNew) store(KEYS.search(current), search);
  }, [current, search, isNew]);

  return (
    <ScreenFrame title="수집">
      <StatusBoard />
      <CollectTabs tabs={COLLECT_TABS} current={current} />
      <div
        role="tabpanel"
        id={PANEL_ID}
        aria-labelledby={tabId(current)}
        tabIndex={0}
        className="pt-[22px] outline-offset-4"
      >
        <Routes>
          {COLLECT_TABS.map((tab) => (
            // `/*` lets a tab own sub-paths, like `/collect/rules/new`.
            <Route key={tab.path} path={`${tab.path}/*`} element={tab.element} />
          ))}
          <Route path="*" element={<Navigate to={`/collect/${DEFAULT_TAB}`} replace />} />
        </Routes>
      </div>
    </ScreenFrame>
  );
}
