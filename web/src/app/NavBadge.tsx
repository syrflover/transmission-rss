import { cn } from "@/lib/utils";

interface NavBadgeProps {
  count: number | undefined;
  className?: string;
}

/** Count badge for a menu entry. Renders nothing when there is nothing to count. */
export function NavBadge({ count, className }: NavBadgeProps) {
  if (!count || count < 1) return null;

  return (
    <span
      className={cn(
        "inline-flex h-[17px] min-w-[17px] items-center justify-center rounded-full bg-urgent px-[5px] text-[11px] font-bold text-urgent-ink",
        className,
      )}
    >
      <span className="sr-only">처리 필요 </span>
      {count}
    </span>
  );
}
