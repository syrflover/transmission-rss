/**
 * What the tab row of a remote screen shows. Pure, so the rules are tested
 * without a page.
 */
import type { Tab } from "./protocol";

/** What a tab with neither a title nor a host is called. */
export const UNNAMED_TAB = "새 창";

/** The name of a tab: its title, else its host, else {@link UNNAMED_TAB}. */
export function tabLabel(tab: Tab): string {
  if (tab.title !== "") return tab.title;
  return tab.host ?? UNNAMED_TAB;
}

/** The row is only worth its room when there is more than one page to choose from. */
export function showsTabRow(tabs: Tab[]): boolean {
  return tabs.length >= 2;
}

/**
 * The tabs as the person should see them while a switch they asked for is on its way: `wanted` marked as the page
 * shown, as long as it is one of the tabs. The server's own tabs replace this when they come.
 */
export function withShown(tabs: Tab[], wanted: string | null): Tab[] {
  if (wanted === null || !tabs.some((tab) => tab.id === wanted)) return tabs;
  if (tabs.every((tab) => tab.shown === (tab.id === wanted))) return tabs;
  return tabs.map((tab) => ({ ...tab, shown: tab.id === wanted }));
}
