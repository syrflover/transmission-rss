import { useEffect, useState } from "react";
import { Link } from "react-router-dom";

import { Button } from "@/components/ui/button";
import { getCommand, isOpen } from "@/lib/commands";
import { useCommandWatch } from "@/lib/useCommandWatch";
import { cn } from "@/lib/utils";
import type { RuleArchived } from "@/lib/api-types";
import { workPath } from "@/screens/library/api";

import { archivedLine } from "../archive/archivedLine";
import { btnNeutral } from "../channels/styles";

/** How long to wait before reading again a move whose last read failed, in milliseconds. */
const RETRY_AFTER_MS = 2000;

interface ArchivedNoticeProps {
  /** The rule's title as the list showed it. */
  name: string;
  result: RuleArchived;
  onClose: () => void;
}

/**
 * What the rule detail's `보관` did, said in the rule tab once the detail has
 * closed: the rule is deleted, and its work folder moved, stays (and why), is
 * moving, or could not move, with a link to the work, whose detail shows the
 * move and moves it again. A move still open is followed until it ends.
 */
export function ArchivedNotice({ name, result, onClose }: ArchivedNoticeProps) {
  const [command, setCommand] = useState(result.command);
  const watching = useCommandWatch();
  const id = result.command?.id ?? null;
  const open = result.command !== null && isOpen(result.command);

  useEffect(() => {
    if (id === null || !open) return;
    const read = async () => {
      try {
        const fresh = await getCommand(id);
        setCommand(fresh);
        if (!isOpen(fresh)) watching.stop();
      } catch {
        watching.retry(id, read, RETRY_AFTER_MS);
      }
    };
    watching.watch(id, read);
    return () => watching.stop();
  }, [id, open, watching]);

  const line = archivedLine(name, command, result.reason);
  return (
    <div
      role="status"
      data-testid="rule-archived"
      className={cn(
        "flex min-w-0 flex-wrap items-center justify-between gap-x-3 gap-y-2 rounded-[10px] border bg-surface-1 px-3.5 py-2.5 text-[13px] leading-normal",
        line.failed ? "border-[color-mix(in_srgb,var(--accent-urgent)_50%,transparent)]" : "border-hairline",
      )}
    >
      <p className={cn("min-w-0 flex-1 basis-64 break-words font-semibold", line.failed ? "text-urgent" : "text-text-primary")}>
        {line.text}
      </p>
      <div className="flex flex-wrap items-center gap-2">
        {result.work_id !== null && (
          <Button asChild variant="ghost" className={btnNeutral}>
            <Link to={workPath(result.work_id)}>작품 보기</Link>
          </Button>
        )}
        <Button type="button" variant="ghost" className={btnNeutral} onClick={onClose}>
          닫기
        </Button>
      </div>
    </div>
  );
}
