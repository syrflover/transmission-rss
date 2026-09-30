import type { ComponentType, ComponentProps, ReactNode } from "react";

import { cn } from "@/lib/utils";

import { AlertIcon, CheckIcon, InfoIcon } from "./icons";

type SvgIcon = ComponentType<ComponentProps<"svg">>;

/** A small tag for a kind, a state or a context. */
export function Tag({
  children,
  icon: Icon,
  tone = "plain",
}: {
  children: ReactNode;
  icon?: SvgIcon;
  tone?: "plain" | "pending" | "warn";
}) {
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1 rounded-md border px-2 py-0.5 text-xs leading-snug font-medium",
        tone === "plain" && "border-hairline bg-surface-2 text-text-secondary",
        tone === "pending" && "border-focus text-text-primary",
        tone === "warn" && "border-hairline bg-surface-2 text-text-primary",
      )}
    >
      {Icon && <Icon className="size-3 flex-none" />}
      {children}
    </span>
  );
}

/** A value with a dim label in front, such as `규칙 24개`. */
export function Labeled({ label, children }: { label: string; children: ReactNode }) {
  return (
    <span className="inline-flex items-baseline gap-1.5 text-[13px]">
      <span className="text-text-muted">{label}</span>
      <span className="text-text-primary">{children}</span>
    </span>
  );
}

/** Items of different kinds side by side, wrapping between whole pieces. */
export function Facts({ children, className }: { children: ReactNode; className?: string }) {
  return <div className={cn("flex flex-wrap items-center gap-x-3.5 gap-y-1.5", className)}>{children}</div>;
}

const BANNER_TONE = {
  calm: { icon: InfoIcon, box: "border-hairline bg-surface-2", ink: "text-focus" },
  done: { icon: CheckIcon, box: "border-hairline bg-surface-2", ink: "text-ok" },
  fail: { icon: AlertIcon, box: "border-urgent bg-surface-1", ink: "text-urgent" },
} as const;

/** A status banner: a question or explanation as the title, then one or two sentences. */
export function Banner({
  tone,
  title,
  children,
  actions,
  role,
}: {
  tone: keyof typeof BANNER_TONE;
  title: string;
  children: ReactNode;
  actions?: ReactNode;
  role?: "status" | "alert";
}) {
  const { icon: Icon, box, ink } = BANNER_TONE[tone];
  return (
    <div role={role} className={cn("flex flex-wrap items-start gap-x-3 gap-y-3 rounded-card border p-3.5", box)}>
      <Icon className={cn("mt-0.5 size-[18px] flex-none", ink)} />
      <div className="min-w-0 flex-1 basis-60 text-[13.5px] leading-relaxed text-text-secondary">
        <strong className="block text-sm font-bold text-text-primary">{title}</strong>
        {children}
      </div>
      {actions && <div className="flex flex-none flex-wrap gap-2">{actions}</div>}
    </div>
  );
}

/** The bar under a step: a sentence on the left, the buttons on the right. */
export function ActionBar({ children, buttons }: { children: ReactNode; buttons?: ReactNode }) {
  return (
    <div className="flex flex-wrap items-center justify-between gap-x-4 gap-y-3 border-t border-hairline-soft pt-4">
      <div className="min-w-0 flex-1 basis-60 text-[13.5px] leading-relaxed text-text-secondary">{children}</div>
      {buttons && <div className="flex flex-none flex-wrap gap-2">{buttons}</div>}
    </div>
  );
}

/** The three steps of an import; the current one is filled and bold, finished ones are checked. */
export function Steps({ current, labels }: { current: number; labels: string[] }) {
  return (
    <ol aria-label="진행 단계" className="m-0 flex list-none flex-wrap gap-2 p-0">
      {labels.map((label, index) => {
        const no = index + 1;
        const here = current === no;
        return (
          <li
            key={label}
            aria-current={here ? "step" : undefined}
            className={cn(
              "inline-flex items-center gap-2 rounded-full border px-3 py-1.5 text-[13px]",
              here ? "border-focus bg-surface-3 font-bold text-text-primary" : "border-hairline text-text-secondary",
            )}
          >
            <span
              className={cn(
                "grid size-5 place-items-center rounded-full text-[11px] font-bold",
                here ? "bg-focus text-primary-foreground" : "bg-surface-3",
              )}
            >
              {current > no ? <CheckIcon className="size-3" /> : no}
            </span>
            {label}
          </li>
        );
      })}
    </ol>
  );
}

/** Class names for a solid main-action button, a plain move button and a bordered action button. */
export const BTN = {
  main: "inline-flex h-9 items-center justify-center gap-1.5 rounded-lg bg-primary px-4 text-sm font-bold text-primary-foreground hover:opacity-90 disabled:cursor-not-allowed disabled:opacity-45",
  plain:
    "inline-flex h-9 items-center justify-center gap-1.5 rounded-lg border border-hairline bg-transparent px-3.5 text-sm font-medium text-text-primary hover:bg-surface-2 disabled:cursor-not-allowed disabled:opacity-45",
  action:
    "inline-flex h-9 items-center justify-center gap-1.5 rounded-lg border border-focus bg-transparent px-3.5 text-sm font-medium text-focus hover:bg-surface-2 disabled:cursor-not-allowed disabled:opacity-45",
};

/** The cards of the settings panels. */
export const CARD = "rounded-card border border-hairline bg-surface-1 shadow-(--card-shadow)";
