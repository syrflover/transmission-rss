import type { ComponentProps } from "react";

/**
 * Line icons for the app shell, redrawn from the accepted prototype's icon
 * sprite. They are decorative: the visible or screen-reader-only text carries
 * the meaning, so every icon is hidden from assistive technology.
 */
function Icon({ children, ...props }: ComponentProps<"svg">) {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
      {...props}
    >
      {children}
    </svg>
  );
}

export function ThemeSystemIcon(props: ComponentProps<"svg">) {
  return (
    <Icon {...props}>
      <circle cx="12" cy="12" r="8" />
      <path d="M12 4a8 8 0 0 1 0 16z" fill="currentColor" stroke="none" />
    </Icon>
  );
}

export function ThemeLightIcon(props: ComponentProps<"svg">) {
  return (
    <Icon {...props}>
      <circle cx="12" cy="12" r="4" />
      <path d="M12 3v2M12 19v2M3 12h2M19 12h2M5.6 5.6l1.4 1.4M17 17l1.4 1.4M5.6 18.4L7 17M17 7l1.4-1.4" />
    </Icon>
  );
}

export function ThemeDarkIcon(props: ComponentProps<"svg">) {
  return (
    <Icon {...props}>
      <path d="M20 14.5A8 8 0 1 1 9.5 4a6.5 6.5 0 0 0 10.5 10.5z" />
    </Icon>
  );
}

export function WeekIcon(props: ComponentProps<"svg">) {
  return (
    <Icon {...props}>
      <rect x="3" y="5" width="18" height="16" rx="2" />
      <path d="M3 10h18M8 3v4M16 3v4" />
    </Icon>
  );
}

export function LibraryIcon(props: ComponentProps<"svg">) {
  return (
    <Icon {...props}>
      <path d="M5 4v16M10 4v16M15 6.5v13.5M19.5 8.5l1 11.3" />
    </Icon>
  );
}

export function ChecklistIcon(props: ComponentProps<"svg">) {
  return (
    <Icon {...props}>
      <rect x="4" y="4" width="16" height="16" rx="3" />
      <path d="M8 12.2l2.4 2.4L16 9" />
    </Icon>
  );
}

export function CollectIcon(props: ComponentProps<"svg">) {
  return (
    <Icon {...props}>
      <path d="M5 5a14 14 0 0 1 14 14M5 11a8 8 0 0 1 8 8" />
      <circle cx="6" cy="18" r="1.4" fill="currentColor" stroke="none" />
    </Icon>
  );
}

export function GearIcon(props: ComponentProps<"svg">) {
  return (
    <Icon {...props}>
      <circle cx="12" cy="12" r="3.2" />
      <path d="M12 3.5v3M12 17.5v3M4.9 7.5l2.6 1.5M16.5 15l2.6 1.5M4.9 16.5l2.6-1.5M16.5 9l2.6-1.5M3 12h3M18 12h3" />
    </Icon>
  );
}
