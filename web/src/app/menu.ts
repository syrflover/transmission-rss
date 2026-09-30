import type { ComponentType, SVGProps } from "react";

import {
  ChecklistIcon,
  CollectIcon,
  GearIcon,
  LibraryIcon,
  WeekIcon,
} from "@/components/icons";

export type MenuId = "schedule" | "library" | "todo" | "collect" | "settings";

export interface MenuItem {
  id: MenuId;
  /** Route path of the menu's top screen; sub-routes keep the menu current. */
  to: string;
  label: string;
  icon: ComponentType<SVGProps<SVGSVGElement>>;
}

/** The five main menus, in display order. */
export const MENU: readonly MenuItem[] = [
  { id: "schedule", to: "/", label: "이번 주 편성", icon: WeekIcon },
  { id: "library", to: "/library", label: "라이브러리", icon: LibraryIcon },
  { id: "todo", to: "/todo", label: "할 일", icon: ChecklistIcon },
  { id: "collect", to: "/collect", label: "수집", icon: CollectIcon },
  { id: "settings", to: "/settings", label: "설정", icon: GearIcon },
];

export const APP_NAME = "TRSS";
