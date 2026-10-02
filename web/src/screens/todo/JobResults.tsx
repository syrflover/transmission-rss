import { cn } from "@/lib/utils";

import type { JobFile, JobItem } from "./api";
import { ItemBadge } from "./badges";
import { episodeName, shownItem, sizeText } from "./format";

/** `받음 2 · 실패 1`: how many items ended in each way, for a job with more than one. */
function summary(items: readonly JobItem[]): string {
  const order: [string, (item: JobItem) => boolean][] = [
    ["받음", (i) => i.state === "done"],
    ["실패", (i) => i.state === "failed"],
    ["보류", (i) => i.state === "held"],
    ["인증 필요", (i) => i.state === "waiting" && i.wait === "auth"],
    ["자막 대기", (i) => i.state === "waiting" && i.wait !== "auth"],
    ["받는 중", (i) => i.state === "running"],
    ["대기", (i) => i.state === "pending"],
  ];
  return order
    .map(([label, match]) => [label, items.filter(match).length] as const)
    .filter(([, n]) => n > 0)
    .map(([label, n]) => `${label} ${n}`)
    .join(" · ");
}

const FILE_STATE = { receiving: "receive", done: "done", held: "held", failed: "failed" } as const;

/**
 * `회차별 결과`: what each episode of the job really came to and why, with the
 * files it received. A job where some failed says so above the list; it never
 * reads as complete.
 */
export function JobResults({ items }: { items: readonly JobItem[] }) {
  if (items.length === 0) {
    return <p className="text-[13px] text-text-muted">아직 회차가 정해지지 않았어요.</p>;
  }
  return (
    <div className="flex flex-col gap-2.5">
      {items.length > 1 && <p className="text-[13px] font-semibold text-text-secondary">{summary(items)}</p>}
      <ul className="m-0 flex list-none flex-col gap-2.5 p-0">
        {items.map((item) => (
          <ItemBlock key={item.id} item={item} />
        ))}
      </ul>
    </div>
  );
}

function ItemBlock({ item }: { item: JobItem }) {
  return (
    <li
      className={cn(
        "flex min-w-0 flex-col gap-2 rounded-card border bg-surface-1 px-3.5 py-3 shadow-(--card-shadow)",
        item.state === "failed" ? "border-urgent" : "border-hairline",
      )}
    >
      <div className="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-1.5">
        <b className="text-[14.5px] font-bold">{episodeName(item.episode)}</b>
        <ItemBadge shown={shownItem(item.state, item.wait)} />
        {item.reason !== null && item.reason !== "" && (
          <span className="min-w-0 flex-1 basis-48 text-[13px] leading-snug text-text-secondary">{item.reason}</span>
        )}
      </div>
      {item.files.length > 0 && (
        <ul className="m-0 flex list-none flex-col gap-2.5 border-t border-hairline-soft p-0 pt-2.5">
          {item.files.map((file, i) => (
            <FileLine key={`${file.name}:${i}`} file={file} />
          ))}
        </ul>
      )}
    </li>
  );
}

function FileLine({ file }: { file: JobFile }) {
  return (
    <li className="flex min-w-0 flex-col gap-1">
      <div className="flex min-w-0 flex-wrap items-center gap-x-2.5 gap-y-1">
        <span className="min-w-0 text-[13px] leading-snug font-semibold [overflow-wrap:anywhere]">{file.name}</span>
        {file.size !== null && <span className="text-xs whitespace-nowrap text-text-muted">{sizeText(file.size)}</span>}
        {file.state !== "done" && <ItemBadge shown={FILE_STATE[file.state]} />}
        {file.shared_with !== null && (
          <span className="text-xs text-text-muted">같은 파일 · {episodeName(file.shared_with)}에서 받음</span>
        )}
      </div>
      {file.reason !== null && file.reason !== "" && (
        <p className="text-xs leading-snug text-text-secondary">{file.reason}</p>
      )}
      {file.path !== null && (
        <p className="text-xs leading-snug text-text-muted [overflow-wrap:anywhere]">{file.path}</p>
      )}
    </li>
  );
}
