import type { MouseEvent } from "react";
import { Outlet } from "react-router-dom";

import { BottomNav, TopNav } from "./MainNav";
import { APP_NAME } from "./menu";
import { useScrollOnNavigate } from "./scroll";
import { ThemeToggle } from "./ThemeToggle";

function SkipLink() {
  // A plain "#main" link would put a fragment into the route, so move focus by hand.
  const skip = (event: MouseEvent<HTMLAnchorElement>) => {
    event.preventDefault();
    document.getElementById("main")?.focus();
  };

  return (
    <a
      href="#main"
      onClick={skip}
      className="fixed top-[-60px] left-3 z-[200] rounded-lg bg-surface-3 px-4 py-2.5 text-text-primary transition-[top] duration-150 focus:top-3"
    >
      본문으로 건너뛰기
    </a>
  );
}

/**
 * The app frame: top bar (name, PC/tablet menu, screen-mode toggle), the routed
 * screen, and the phone bottom menu.
 */
export function AppShell() {
  useScrollOnNavigate();

  return (
    <>
      <SkipLink />

      <header className="sticky top-0 z-50 flex items-center justify-between gap-4 border-b border-hairline-soft bg-[color-mix(in_srgb,var(--bg-void)_84%,transparent)] py-3.5 pr-[max(var(--gutter),env(safe-area-inset-right))] pl-[max(var(--gutter),env(safe-area-inset-left))] backdrop-blur-[14px] backdrop-saturate-[1.4] max-[720px]:py-3">
        <div className="flex-none text-[19px] font-bold tracking-[0.01em]">{APP_NAME}</div>
        <div className="flex items-center gap-2.5">
          <TopNav />
          <ThemeToggle />
        </div>
      </header>

      <main
        id="main"
        tabIndex={-1}
        className="mx-auto w-full max-w-(--page-max) pr-[max(var(--gutter),env(safe-area-inset-right))] pb-16 pl-[max(var(--gutter),env(safe-area-inset-left))] outline-none max-[720px]:pb-[calc(96px+env(safe-area-inset-bottom,0px))]"
      >
        <Outlet />
      </main>

      <BottomNav />
    </>
  );
}
