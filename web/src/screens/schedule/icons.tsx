import type { ComponentProps } from "react";

/** Line icons of the home screen. They only support the visible text, so they are hidden from assistive technology. */
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

/** An arrow down into a tray: the video is being received. */
export function DownloadIcon(props: ComponentProps<"svg">) {
  return (
    <Icon {...props} strokeWidth={2.4}>
      <path d="M12 4v11M7.5 10.5L12 15l4.5-4.5M5 19.5h14" />
    </Icon>
  );
}

/** A speech bubble with lines: a subtitle, and whose it is. */
export function SubtitleIcon(props: ComponentProps<"svg">) {
  return (
    <Icon {...props}>
      <rect x="3.5" y="5" width="17" height="14" rx="2.5" />
      <path d="M7.5 11h4M14 11h2.5M7.5 14.5h2.5M12.5 14.5h4" />
    </Icon>
  );
}
