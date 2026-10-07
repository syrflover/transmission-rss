import type { ArchivedWork } from "./api";
import { archivedWorkNotice } from "./archivedWork";

/**
 * Before a rule is made (or a paused one turned on): its work folder is in the
 * archive folder, so it is moved into the collect folder first.
 */
export function ArchivedWorkNotice({ archived }: { archived: ArchivedWork }) {
  const { title, detail } = archivedWorkNotice(archived);
  return (
    <div
      role="status"
      data-testid="archived-work-notice"
      className="flex min-w-0 flex-col gap-1 rounded-xl border border-hairline bg-surface-2 px-3.5 py-3 text-[13px] leading-normal"
    >
      <p className="font-bold break-words text-text-primary">{title}</p>
      <p className="break-words text-text-secondary">{detail}</p>
    </div>
  );
}
