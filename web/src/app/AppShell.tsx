import type { MouseEvent } from "react";
import { Outlet } from "react-router-dom";

import { BottomNav, TopNav } from "./MainNav";
import { useScrollOnNavigate } from "./scroll";

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
 * The app frame: the PC/tablet top bar, which holds only the menu, the routed
 * screen, and the phone bottom menu. Phones have no top bar, so the screen
 * keeps clear of the device's top safe area itself (the 화면 모드 is in 설정).
 */
export function AppShell() {
  useScrollOnNavigate();

  return (
    <>
      <SkipLink />

      <header className="sticky top-0 z-50 flex h-(--topbar-h) items-center border-b border-hairline-soft bg-[color-mix(in_srgb,var(--bg-void)_84%,transparent)] pr-[max(var(--gutter),env(safe-area-inset-right))] pl-[max(var(--gutter),env(safe-area-inset-left))] backdrop-blur-[14px] backdrop-saturate-[1.4] max-[720px]:hidden">
        <TopNav />
      </header>

      <main
        id="main"
        tabIndex={-1}
        className="mx-auto w-full max-w-(--page-max) pr-[max(var(--gutter),env(safe-area-inset-right))] pb-16 pl-[max(var(--gutter),env(safe-area-inset-left))] outline-none max-[720px]:pt-(--topbar-h) max-[720px]:pb-[calc(96px+env(safe-area-inset-bottom,0px))]"
      >
        <Outlet />
      </main>

      <BottomNav />
    </>
  );
}
