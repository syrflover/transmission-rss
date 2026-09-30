import { useEffect, useLayoutEffect, useRef } from "react";
import { useLocation, useNavigationType } from "react-router-dom";

/**
 * The part of the path that names a screen. `/collect/rules/new` and
 * `/collect/rules` are one screen (the rules tab keeps its list while a rule
 * is opened); `/settings` and `/settings/import` are two.
 */
function screenOf(pathname: string): string {
  return pathname.split("/").slice(0, 3).join("/").replace(/\/+$/, "");
}

const scrollTo = (y: number) => window.scrollTo({ top: y, left: 0, behavior: "instant" });

/**
 * Page scroll on navigation:
 *
 * - opening another screen (a menu, a tab) starts at the top;
 * - going back or forward returns to where that entry was, as far as the page
 *   is tall enough by then (a screen that shows its last answer from the cache
 *   is tall enough at once);
 * - a change inside one screen (a rule picked with `?rule=`, a filter) does not
 *   move the page.
 *
 * The move is instant: the page's smooth scrolling would otherwise sweep the
 * old position to the top while the new screen is already showing.
 */
export function useScrollOnNavigate() {
  const { key, pathname } = useLocation();
  const type = useNavigationType();
  const here = useRef({ key, screen: screenOf(pathname), y: 0 });
  const saved = useRef(new Map<string, number>());

  // `y` is the position of the page being left. It is read before anything
  // that can change the route (a scroll event comes a frame late; reading it
  // once the route has changed would see the new page's clamped position).
  useEffect(() => {
    const read = () => {
      here.current.y = window.scrollY;
    };
    const events = ["scroll", "pointerdown", "keydown", "popstate"] as const;
    for (const name of events) window.addEventListener(name, read, { capture: true, passive: true });
    return () => {
      for (const name of events) window.removeEventListener(name, read, { capture: true });
    };
  }, []);

  useLayoutEffect(() => {
    const before = here.current;
    if (before.key === key) return;
    saved.current.set(before.key, before.y);
    const screen = screenOf(pathname);
    let y = before.y;
    if (type === "POP") {
      y = saved.current.get(key) ?? 0;
      scrollTo(y);
    } else if (screen !== before.screen) {
      y = 0;
      scrollTo(0);
    }
    here.current = { key, screen, y };
  }, [key, pathname, type]);
}
