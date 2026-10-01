import type { ComponentProps } from "react";

/** Line icons of the library screen. They only support the visible text, so they are hidden from assistive technology. */
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

export function GridIcon(props: ComponentProps<"svg">) {
  return (
    <Icon {...props}>
      <rect x="4" y="4" width="6.5" height="6.5" rx="1.2" />
      <rect x="13.5" y="4" width="6.5" height="6.5" rx="1.2" />
      <rect x="4" y="13.5" width="6.5" height="6.5" rx="1.2" />
      <rect x="13.5" y="13.5" width="6.5" height="6.5" rx="1.2" />
    </Icon>
  );
}

export function ListIcon(props: ComponentProps<"svg">) {
  return (
    <Icon {...props}>
      <path d="M9 6h11M9 12h11M9 18h11" />
      <path d="M4 6h.01M4 12h.01M4 18h.01" />
    </Icon>
  );
}

/** A check: the file is there, or the season is the chosen one. */
export function CheckIcon(props: ComponentProps<"svg">) {
  return (
    <Icon {...props} strokeWidth={2.6}>
      <path d="M5 12.5l4.5 4.5L19 7.5" />
    </Icon>
  );
}

/** A dash: there is no such file. */
export function MinusIcon(props: ComponentProps<"svg">) {
  return (
    <Icon {...props} strokeWidth={2.6}>
      <path d="M6 12h12" />
    </Icon>
  );
}

/** A chevron pointing right; turned down with CSS when its section is open. */
export function ChevronIcon(props: ComponentProps<"svg">) {
  return (
    <Icon {...props}>
      <path d="M9 6l6 6-6 6" />
    </Icon>
  );
}

/** The question mark that marks `확인 필요`. */
export function QuestionIcon(props: ComponentProps<"svg">) {
  return (
    <Icon {...props}>
      <circle cx="12" cy="12" r="9" />
      <path d="M9.6 9.4a2.5 2.5 0 1 1 3.4 2.3c-.7.3-1 .9-1 1.6M12 17h.01" />
    </Icon>
  );
}

export function CalendarIcon(props: ComponentProps<"svg">) {
  return (
    <Icon {...props}>
      <rect x="4" y="5.5" width="16" height="14" rx="2" />
      <path d="M4 10h16M8.5 3.5v4M15.5 3.5v4" />
    </Icon>
  );
}

export function FilmIcon(props: ComponentProps<"svg">) {
  return (
    <Icon {...props}>
      <rect x="4" y="4" width="16" height="16" rx="2" />
      <path d="M8 4v16M16 4v16M4 9h4M4 15h4M16 9h4M16 15h4" />
    </Icon>
  );
}

export function StudioIcon(props: ComponentProps<"svg">) {
  return (
    <Icon {...props}>
      <path d="M4 20V9l5 3V9l5 3V6h6v14zM8 20v-3M13 20v-3" />
    </Icon>
  );
}

export function TagIcon(props: ComponentProps<"svg">) {
  return (
    <Icon {...props}>
      <path d="M4 12.5V5h7.5L20 13.5 13.5 20z" />
      <circle cx="8.5" cy="9.5" r="1" />
    </Icon>
  );
}

export function ExternalIcon(props: ComponentProps<"svg">) {
  return (
    <Icon {...props}>
      <path d="M14 5h5v5M19 5l-8 8M18 14v4a1 1 0 0 1-1 1H6a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1h4" />
    </Icon>
  );
}
