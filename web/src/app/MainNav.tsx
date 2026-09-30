import { NavLink } from "react-router-dom";

import { cn } from "@/lib/utils";

import { MENU, type MenuItem } from "./menu";
import { NavBadge } from "./NavBadge";
import { useTodoCount } from "./todo-count";

function useBadgeCount(item: MenuItem): number | undefined {
  const todoCount = useTodoCount();
  return item.id === "todo" ? todoCount : undefined;
}

/**
 * The current menu is marked by more than colour: a filled background, bold
 * text and an indicator bar. Moving keyboard focus over the links does not
 * navigate; only activating one (Enter/click) changes the route, and
 * `aria-current="page"` follows the route alone.
 */

function TopNavLink({ item }: { item: MenuItem }) {
  const count = useBadgeCount(item);
  const Icon = item.icon;

  return (
    <NavLink
      to={item.to}
      end={item.to === "/"}
      className={cn(
        "relative flex items-center gap-[7px] rounded-full px-3.5 py-2 text-sm font-medium whitespace-nowrap text-text-secondary hover:text-text-primary",
        "min-[721px]:max-[979px]:px-[11px]",
        "aria-[current=page]:bg-surface-2 aria-[current=page]:font-bold aria-[current=page]:text-text-primary",
        "aria-[current=page]:after:absolute aria-[current=page]:after:bottom-[3px] aria-[current=page]:after:left-1/2 aria-[current=page]:after:h-[3px] aria-[current=page]:after:w-5 aria-[current=page]:after:-translate-x-1/2 aria-[current=page]:after:rounded-full aria-[current=page]:after:bg-focus aria-[current=page]:after:content-['']",
      )}
    >
      <Icon className="size-4 flex-none opacity-85" />
      {/* Icon-only between 721 and 979px: the name stays for assistive technology. */}
      <span className="bold-reserve min-[721px]:max-[979px]:sr-only" data-text={item.label}>
        {item.label}
      </span>
      <NavBadge count={count} className="min-[721px]:max-[979px]:-ml-[3px]" />
    </NavLink>
  );
}

/** PC and tablet menu in the top bar. Hidden on phones, where the bottom menu replaces it. */
export function TopNav() {
  return (
    <nav aria-label="주요 메뉴" className="flex items-center gap-1 max-[720px]:hidden">
      {MENU.map((item) => (
        <TopNavLink key={item.id} item={item} />
      ))}
    </nav>
  );
}

function BottomNavLink({ item }: { item: MenuItem }) {
  const count = useBadgeCount(item);
  const Icon = item.icon;

  return (
    <NavLink
      to={item.to}
      end={item.to === "/"}
      className={cn(
        "relative flex min-h-[52px] min-w-0 flex-1 basis-0 flex-col items-center justify-center gap-[3px] px-0.5 pt-[9px] pb-2.5 text-[10.5px] font-medium whitespace-nowrap text-text-muted -outline-offset-2",
        "aria-[current=page]:font-bold aria-[current=page]:text-text-primary",
        "aria-[current=page]:before:absolute aria-[current=page]:before:top-0.5 aria-[current=page]:before:h-[3px] aria-[current=page]:before:w-[22px] aria-[current=page]:before:rounded-full aria-[current=page]:before:bg-focus aria-[current=page]:before:content-['']",
      )}
    >
      <Icon className="size-[21px]" />
      <span className="bold-reserve" data-text={item.label}>
        {item.label}
      </span>
      <NavBadge count={count} className="absolute top-[3px] right-[calc(50%-20px)]" />
    </NavLink>
  );
}

/**
 * Phone menu fixed to the bottom edge. It pads itself by the device's safe
 * areas (home indicator, rounded corners) so the entries stay tappable.
 */
export function BottomNav() {
  return (
    <nav
      aria-label="주요 메뉴"
      className="fixed inset-x-0 bottom-0 z-40 hidden items-stretch justify-around border-t border-hairline-soft bg-[color-mix(in_srgb,var(--bg-void)_90%,transparent)] pr-[env(safe-area-inset-right,0px)] pb-[env(safe-area-inset-bottom,0px)] pl-[env(safe-area-inset-left,0px)] backdrop-blur-lg max-[720px]:flex"
    >
      {MENU.map((item) => (
        <BottomNavLink key={item.id} item={item} />
      ))}
    </nav>
  );
}
