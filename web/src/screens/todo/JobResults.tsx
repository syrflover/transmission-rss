import { Link } from "react-router-dom";

import { cn } from "@/lib/utils";

import { jobPath, type JobFile, type JobItem, type Placement } from "./api";
import { FailureTag, ItemBadge } from "./badges";
import { FORMAT_LABEL, episodeName, shownItem, sizeText } from "./format";

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

const FILE_STATE = {
  receiving: "receive",
  done: "done",
  held: "held",
  failed: "failed",
} as const;

/**
 * `회차별 결과`: what each episode of the job really came to and why, with the
 * files it received. A job where some failed says so above the list; it never
 * reads as complete.
 */
export function JobResults({
  items,
  placements = [],
}: {
  items: readonly JobItem[];
  placements?: readonly Placement[];
}) {
  if (items.length === 0) {
    return (
      <p className="text-[13px] text-text-muted">
        아직 회차가 정해지지 않았어요.
      </p>
    );
  }
  return (
    <div className="flex flex-col gap-2.5">
      {items.length > 1 && (
        <p className="text-[13px] font-semibold text-text-secondary">
          {summary(items)}
        </p>
      )}
      <ul className="m-0 flex list-none flex-col gap-2.5 p-0">
        {items.map((item) => (
          <ItemBlock key={item.id} item={item} placements={placements} />
        ))}
      </ul>
    </div>
  );
}

function ItemBlock({ item, placements }: { item: JobItem; placements: readonly Placement[] }) {
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
        {((item.reason !== null && item.reason !== "") ||
          item.failure !== null) && (
          <span className="min-w-0 flex-1 basis-48 text-[13px] leading-snug text-text-secondary">
            {item.failure !== null && <FailureTag failure={item.failure} />}
            {item.reason}
          </span>
        )}
      </div>
      {item.unchanged_from !== null && (
        <p className="text-[13px] leading-snug text-text-secondary">
          바꿀 것이 없음 · 받은 파일이{" "}
          <Link
            to={jobPath(item.unchanged_from)}
            className="rounded-sm underline underline-offset-2 focus-visible:outline-2 focus-visible:outline-focus"
          >
            이전에 받은 것
          </Link>
          과 같아요
        </p>
      )}
      {item.files.length > 0 && (
        <ul className="m-0 flex list-none flex-col gap-2.5 border-t border-hairline-soft p-0 pt-2.5">
          {item.files.map((file, i) => (
            <FileLine
              key={`${file.name}:${i}`}
              file={file}
              placements={placements.filter((p) => p.file_id === file.id)}
            />
          ))}
        </ul>
      )}
    </li>
  );
}

/** What answered a failed file: `HTTP 404 · text/html · 150 B`, from what is known. */
function answerLine(file: JobFile): string | null {
  const parts = [
    file.http_status !== null ? `HTTP ${file.http_status}` : null,
    file.content_type,
    file.response_size !== null ? sizeText(file.response_size) : null,
  ].filter((part): part is string => part !== null && part !== "");
  return parts.length > 0 ? parts.join(" · ") : null;
}

/** The words a font file's name carries on the posts seen. */
const FONT_NAME = /폰트|글꼴|font/i;

/** What came of a placed file, in a word, with the reason or the episode it went to. */
function placementText(p: Placement): { word: string; detail: string | null; urgent: boolean } {
  const episode = p.episode !== null ? episodeName(String(p.episode)) : null;
  if (p.outcome === null) {
    return p.question !== null
      ? { word: "회차 확인 필요", detail: p.question, urgent: false }
      : { word: "보관 대기", detail: null, urgent: false };
  }
  switch (p.outcome) {
    case "applied":
      return { word: "적용함", detail: episode, urgent: false };
    case "stored":
    case "existing":
    case "no_video":
      return { word: "보관만 함", detail: p.note, urgent: false };
    case "held":
      return { word: "보류", detail: p.note, urgent: false };
    case "failed":
      return { word: "보관·적용 실패", detail: p.note, urgent: true };
    case "dropped":
      return { word: "버림", detail: p.note, urgent: false };
  }
}

function PlacementLine({ placement }: { placement: Placement }) {
  const { word, detail, urgent } = placementText(placement);
  return (
    <p className="text-xs leading-snug text-text-secondary">
      <b className={cn("font-semibold", urgent ? "text-urgent" : "text-text-primary")}>{word}</b>
      {detail !== null && detail !== "" && <span> · {detail}</span>}
    </p>
  );
}

function FileLine({ file, placements }: { file: JobFile; placements: readonly Placement[] }) {
  // The size and format of what was received; a failed file's bytes are gone.
  // A ZIP is received whole as a bundle: which of its files serve which
  // episode is the analysis's, after the receipt. A ZIP named for fonts holds
  // the post's fonts, not episodes.
  const facts = [
    file.state !== "failed" && file.size !== null ? sizeText(file.size) : null,
    file.format !== null ? FORMAT_LABEL[file.format] : null,
    file.format === "zip"
      ? FONT_NAME.test(file.name)
        ? "폰트 묶음"
        : "묶음으로 받음"
      : null,
  ].filter((fact): fact is string => fact !== null);
  const answer = file.state === "failed" ? answerLine(file) : null;
  return (
    <li className="flex min-w-0 flex-col gap-1">
      <div className="flex min-w-0 flex-wrap items-center gap-x-2.5 gap-y-1">
        <span className="min-w-0 text-[13px] leading-snug font-semibold [overflow-wrap:anywhere]">
          {file.folder !== null ? `${file.folder}/${file.name}` : file.name}
        </span>
        {facts.length > 0 && (
          <span className="text-xs whitespace-nowrap text-text-muted">
            {facts.join(" · ")}
          </span>
        )}
        {file.state !== "done" && <ItemBadge shown={FILE_STATE[file.state]} />}
        {file.shared_with !== null && (
          <span className="text-xs text-text-muted">
            같은 파일 · {episodeName(file.shared_with)}에서 받음
          </span>
        )}
      </div>
      {((file.reason !== null && file.reason !== "") ||
        file.failure !== null) && (
        <p className="text-xs leading-snug text-text-secondary">
          {file.failure !== null && <FailureTag failure={file.failure} />}
          {file.reason}
        </p>
      )}
      {answer !== null && (
        <p className="text-xs leading-snug text-text-muted">{answer}</p>
      )}
      {placements.map((p) => (
        <PlacementLine key={p.position} placement={p} />
      ))}
      {file.path !== null && (
        <p className="text-xs leading-snug text-text-muted [overflow-wrap:anywhere]">
          {file.path}
        </p>
      )}
    </li>
  );
}
