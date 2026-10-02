import type { ComponentProps } from "react";

/**
 * Line icons of the 할 일 screen and the job detail. They only support the
 * visible text, so every one is hidden from assistive technology.
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

/** A caption box: the subtitle creator. */
export function SubtitleIcon(props: ComponentProps<"svg">) {
  return (
    <Icon {...props}>
      <rect x="3.5" y="5.5" width="17" height="13" rx="2" />
      <path d="M7 12h3.5M13 12h4M7 15.5h6M15.5 15.5H17" />
    </Icon>
  );
}

export function LockIcon(props: ComponentProps<"svg">) {
  return (
    <Icon {...props}>
      <rect x="5" y="11" width="14" height="9" rx="2" />
      <path d="M8 11V8a4 4 0 0 1 8 0v3" />
    </Icon>
  );
}

/** A triangle with an exclamation mark: something failed. */
export function WarningIcon(props: ComponentProps<"svg">) {
  return (
    <Icon {...props}>
      <path d="M12 4 2.8 19.5h18.4z" />
      <path d="M12 10v4.5M12 17v.01" />
    </Icon>
  );
}

export function ClockIcon(props: ComponentProps<"svg">) {
  return (
    <Icon {...props}>
      <circle cx="12" cy="12" r="9" />
      <path d="M12 7.5V12l3 2" />
    </Icon>
  );
}

/** Two bars: held, not failed. */
export function PauseIcon(props: ComponentProps<"svg">) {
  return (
    <Icon {...props}>
      <circle cx="12" cy="12" r="9" />
      <path d="M10 9v6M14 9v6" />
    </Icon>
  );
}

export function DownloadIcon(props: ComponentProps<"svg">) {
  return (
    <Icon {...props}>
      <path d="M12 4v11M7 11l5 5 5-5M5 20h14" />
    </Icon>
  );
}

export function CheckIcon(props: ComponentProps<"svg">) {
  return (
    <Icon {...props} strokeWidth={2.6}>
      <path d="M5 12.5l4.5 4.5L19 7.5" />
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

export function BackIcon(props: ComponentProps<"svg">) {
  return (
    <Icon {...props}>
      <path d="m15 6-6 6 6 6" />
    </Icon>
  );
}

/** A step that has not been reached. */
export function CircleIcon(props: ComponentProps<"svg">) {
  return (
    <Icon {...props}>
      <circle cx="12" cy="12" r="3" />
    </Icon>
  );
}

/** The step the job is at, moving. */
export function DotIcon(props: ComponentProps<"svg">) {
  return (
    <Icon {...props} fill="currentColor" stroke="none">
      <circle cx="12" cy="12" r="4.5" />
    </Icon>
  );
}

export function ListIcon(props: ComponentProps<"svg">) {
  return (
    <Icon {...props}>
      <path d="M9 6h11M9 12h11M9 18h11M4 6h.01M4 12h.01M4 18h.01" />
    </Icon>
  );
}
