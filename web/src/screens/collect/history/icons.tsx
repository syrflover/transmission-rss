import type { ComponentProps } from "react";

/** Line icons of the history tab. They only support the visible text, so they are hidden from assistive technology. */
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

export function ChevronIcon(props: ComponentProps<"svg">) {
  return (
    <Icon {...props}>
      <path d="M9 6l6 6-6 6" />
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

export function PlusIcon(props: ComponentProps<"svg">) {
  return (
    <Icon {...props}>
      <path d="M12 5v14M5 12h14" />
    </Icon>
  );
}
