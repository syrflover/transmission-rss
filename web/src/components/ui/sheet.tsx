import * as React from "react";
import { Dialog } from "radix-ui";

import { cn } from "@/lib/utils";

/**
 * A sheet that rises from the bottom edge, for choices on a phone. It is a
 * modal dialog: Escape, a press on the backdrop and the close button close it,
 * the page behind is inert while it is open, and focus returns to the button
 * that opened it (`SheetTrigger`) however it closed. A long body scrolls
 * inside the sheet, never sideways.
 */
const Sheet = Dialog.Root;
const SheetTrigger = Dialog.Trigger;

interface SheetContentProps {
  /** The sheet's name, read when it opens. */
  title: string;
  className?: string;
  /** Where focus goes when the sheet opens; by default the first control (the close button). */
  onOpenAutoFocus?: (event: Event) => void;
  children: React.ReactNode;
}

function SheetContent({ title, className, onOpenAutoFocus, children }: SheetContentProps) {
  return (
    <Dialog.Portal>
      <Dialog.Overlay className="fixed inset-0 z-[250] bg-[rgba(4,6,10,0.62)] data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:animate-in data-[state=open]:fade-in-0" />
      <Dialog.Content
        aria-describedby={undefined}
        onOpenAutoFocus={onOpenAutoFocus}
        className={cn(
          "fixed inset-x-0 bottom-0 z-[251] flex max-h-[84dvh] flex-col rounded-t-2xl border border-b-0 border-hairline bg-surface-1 shadow-[0_-20px_50px_-16px_var(--shadow-color)] outline-none",
          "data-[state=closed]:animate-out data-[state=closed]:slide-out-to-bottom data-[state=open]:animate-in data-[state=open]:slide-in-from-bottom",
          className,
        )}
      >
        <div className="flex items-center gap-2 border-b border-hairline-soft py-2 pr-2 pl-4">
          <Dialog.Title className="min-w-0 flex-1 text-[17px] font-bold">{title}</Dialog.Title>
          <Dialog.Close
            aria-label="닫기"
            className="inline-flex size-11 flex-none items-center justify-center rounded-full text-text-secondary hover:bg-surface-2 hover:text-text-primary"
          >
            <svg
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth={2}
              strokeLinecap="round"
              aria-hidden="true"
              focusable="false"
              className="size-5"
            >
              <path d="M6 6l12 12M18 6L6 18" />
            </svg>
          </Dialog.Close>
        </div>
        <div className="flex min-h-0 flex-1 flex-col gap-2.5 overflow-x-hidden overflow-y-auto overscroll-contain px-4 pt-3 pb-[calc(16px+env(safe-area-inset-bottom,0px))]">
          {children}
        </div>
      </Dialog.Content>
    </Dialog.Portal>
  );
}

export { Sheet, SheetContent, SheetTrigger };
