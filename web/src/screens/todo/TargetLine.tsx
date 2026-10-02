import type { ReactNode } from "react";

import { cn } from "@/lib/utils";

import { episodeLabel } from "./format";
import { SubtitleIcon } from "./icons";

/**
 * What a to-do or a job is about, each kind of fact in its own shape: the
 * episodes bold, the creator with a subtitle icon, context as small tags
 * (`children`). Never joined into one string with ` · `.
 */
export function TargetLine({
  episodes,
  creator,
  children,
  className,
}: {
  episodes: readonly string[];
  creator: string | null;
  children?: ReactNode;
  className?: string;
}) {
  const label = episodeLabel(episodes);
  if (label === null && creator === null && !children) return null;
  return (
    <p className={cn("flex min-w-0 flex-wrap items-center gap-x-2.5 gap-y-1 text-[12.5px] leading-snug", className)}>
      {label !== null && (
        <span className="text-text-primary">
          <b className="font-bold">{label.label}</b>
          {label.more > 0 && <span className="text-text-muted"> 외 {label.more}개</span>}
        </span>
      )}
      {creator !== null && (
        <span className="inline-flex min-w-0 items-center gap-1 text-text-secondary">
          <SubtitleIcon className="size-3.5 flex-none text-text-muted" />
          <span className="min-w-0 [overflow-wrap:anywhere]">{creator}</span>
        </span>
      )}
      {children}
    </p>
  );
}
