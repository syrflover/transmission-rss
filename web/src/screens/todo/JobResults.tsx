import { Link } from "react-router-dom";

import { cn } from "@/lib/utils";

import { jobPath, type JobFile, type JobItem, type Placement, type Replacement } from "./api";
import { FailureTag, ItemBadge } from "./badges";
import { archiveReceiptText, fontReceiptText } from "./fontReceipt";
import { FORMAT_LABEL, episodeName, shownItem, sizeText, unpackText } from "./format";
import { waitingPositions } from "./replacementView";

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
 * reads as complete. A file whose episode has a replacement to decide reads
 * `교체 승인 대기`, not as under way.
 */
export function JobResults({
  items,
  placements = [],
  replacements = [],
}: {
  items: readonly JobItem[];
  placements?: readonly Placement[];
  replacements?: readonly Replacement[];
}) {
  if (items.length === 0) {
    return (
      <p className="text-[13px] text-text-muted">
        아직 회차가 정해지지 않았어요.
      </p>
    );
  }
  const waiting = waitingPositions(replacements);
  return (
    <div className="flex flex-col gap-2.5">
      {items.length > 1 && (
        <p className="text-[13px] font-semibold text-text-secondary">
          {summary(items)}
        </p>
      )}
      <ul className="m-0 flex list-none flex-col gap-2.5 p-0">
        {items.map((item) => (
          <ItemBlock key={item.id} item={item} placements={placements} waiting={waiting} />
        ))}
      </ul>
    </div>
  );
}

/**
 * `회차별 결과` of an upload or a find job: what came of its one package's files, in groups. Its files are listed
 * above (`올린 파일`, `받은 파일`), and its item is on no episode of its own.
 */
export function PackageResults({
  placements,
  replacements = [],
}: {
  placements: readonly Placement[];
  replacements?: readonly Replacement[];
}) {
  return (
    <div className="flex min-w-0 flex-col rounded-card border border-hairline bg-surface-1 px-3.5 py-3 shadow-(--card-shadow)">
      <PackageGroups placements={placements} waiting={waitingPositions(replacements)} divided={false} />
    </div>
  );
}

function ItemBlock({
  item,
  placements,
  waiting,
}: {
  item: JobItem;
  placements: readonly Placement[];
  waiting: ReadonlySet<number>;
}) {
  // A package of several files says what came of them in groups, not under
  // each file.
  const own = new Set(item.files.map((f) => f.id));
  const ofItem = placements.filter((p) => own.has(p.file_id));
  const grouped = ofItem.length > 1;
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
              placements={grouped ? [] : placements.filter((p) => p.file_id === file.id)}
              waiting={waiting}
            />
          ))}
        </ul>
      )}
      {grouped && <PackageGroups placements={ofItem} waiting={waiting} />}
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
function placementText(
  p: Placement,
  waiting: ReadonlySet<number>,
): { word: string; detail: string | null; urgent: boolean } {
  const episode = p.episode !== null ? episodeName(String(p.episode)) : null;
  if (p.outcome === null) {
    if (waiting.has(p.position)) return { word: "교체 승인 대기", detail: null, urgent: false };
    return p.question !== null
      ? { word: "회차 확인 필요", detail: p.question, urgent: false }
      : { word: "보관 대기", detail: null, urgent: false };
  }
  switch (p.outcome) {
    case "applied":
      return { word: "적용함", detail: episode, urgent: false };
    case "stored":
    case "existing":
      return { word: "보관만 함", detail: p.note, urgent: false };
    case "no_video":
      return { word: "영상 대기", detail: p.note, urgent: false };
    case "held":
      return { word: "보류", detail: p.note, urgent: false };
    case "failed":
      return { word: "보관·적용 실패", detail: p.note, urgent: true };
    case "dropped":
      return { word: "버림", detail: p.note, urgent: false };
  }
}

/** The groups a package's files are told in, in this order; the first two are what a person decides. */
const GROUPS: {
  key: string;
  title: string;
  urgent?: boolean;
  match: (p: Placement, waiting: ReadonlySet<number>) => boolean;
}[] = [
  {
    key: "ask",
    title: "확인 필요",
    match: (p) => p.outcome === null && p.question !== null,
  },
  {
    key: "approval",
    title: "교체 승인 대기",
    match: (p, waiting) => p.outcome === null && waiting.has(p.position),
  },
  { key: "held", title: "보류", match: (p) => p.outcome === "held" },
  {
    key: "failed",
    title: "보관·적용 실패",
    urgent: true,
    match: (p) => p.outcome === "failed",
  },
  { key: "applied", title: "적용함", match: (p) => p.outcome === "applied" },
  { key: "video", title: "영상 대기", match: (p) => p.outcome === "no_video" },
  {
    key: "stored",
    title: "보관만 함",
    match: (p) =>
      p.kind === "subtitle" &&
      p.episode !== null &&
      (p.outcome === "stored" || p.outcome === "existing"),
  },
  {
    key: "loose",
    title: "회차에 붙이지 않음",
    match: (p) =>
      p.kind === "subtitle" && p.episode === null && p.outcome === "stored",
  },
  {
    key: "assets",
    title: "폰트·첨부",
    match: (p) => p.kind !== "subtitle" && p.outcome === "stored",
  },
  { key: "dropped", title: "버림", match: (p) => p.outcome === "dropped" },
  {
    key: "pending",
    title: "처리 대기",
    match: (p, waiting) => p.outcome === null && p.question === null && !waiting.has(p.position),
  },
];

const ASSET_KIND: Record<Placement["kind"], string> = {
  subtitle: "자막",
  font: "폰트",
  attachment: "첨부",
  companion: "구성 파일",
  other: "그 밖의 파일",
};

/** One file of a group: its name, with its episode and why when it says more, and how a kept font was received. */
function GroupLine({
  placement: p,
  group,
}: {
  placement: Placement;
  group: string;
}) {
  const detail = [
    p.episode !== null ? episodeName(String(p.episode)) : null,
    group === "assets" ? ASSET_KIND[p.kind] : null,
    group === "assets" ? fontReceiptText(p.font_receipt) : null,
    group === "ask" ? p.question : group === "applied" ? null : p.note,
  ].filter((part): part is string => part !== null && part !== "");
  return (
    <li className="flex min-w-0 flex-col gap-0.5">
      <span className="text-[13px] leading-snug [overflow-wrap:anywhere]">
        {p.name}
      </span>
      {detail.length > 0 && (
        <span className="text-xs leading-snug text-text-secondary [overflow-wrap:anywhere]">
          {detail.join(" · ")}
        </span>
      )}
    </li>
  );
}

/**
 * What came of a package's files, grouped: applied, waiting for the video,
 * stored only (another episode, another format), on no episode, fonts and
 * attachments, dropped, and first what needs a person.
 */
function PackageGroups({
  placements,
  waiting,
  divided = true,
}: {
  placements: readonly Placement[];
  waiting: ReadonlySet<number>;
  /** Set apart from the item's files above it. */
  divided?: boolean;
}) {
  const groups = GROUPS.map((g) => ({
    ...g,
    rows: placements.filter((p) => g.match(p, waiting)),
  })).filter((g) => g.rows.length > 0);
  return (
    <div className={cn("flex flex-col gap-2.5", divided && "border-t border-hairline-soft pt-2.5")}>
      {groups.map((g) => (
        <section key={g.key} aria-label={g.title} className="flex flex-col gap-1.5">
          <h4
            className={cn(
              "text-xs font-bold",
              g.urgent ? "text-urgent" : "text-text-primary",
            )}
          >
            {g.title}{" "}
            <span className="font-semibold text-text-muted">
              {g.rows.length}
            </span>
          </h4>
          <ul className="m-0 flex list-none flex-col gap-1.5 p-0">
            {g.rows.map((p) => (
              <GroupLine key={p.position} placement={p} group={g.key} />
            ))}
          </ul>
        </section>
      ))}
    </div>
  );
}

function PlacementLine({ placement, waiting }: { placement: Placement; waiting: ReadonlySet<number> }) {
  const { word, detail, urgent } = placementText(placement, waiting);
  const font = fontReceiptText(placement.font_receipt);
  return (
    <p className="text-xs leading-snug text-text-secondary">
      <b className={cn("font-semibold", urgent ? "text-urgent" : "text-text-primary")}>{word}</b>
      {detail !== null && detail !== "" && <span> · {detail}</span>}
      {font !== null && <span> · {font}</span>}
    </p>
  );
}

function FileLine({
  file,
  placements,
  waiting,
}: {
  file: JobFile;
  placements: readonly Placement[];
  waiting: ReadonlySet<number>;
}) {
  // The size and format of what was received; a failed file's bytes are gone.
  // A ZIP is received whole as a bundle: which of its files serve which
  // episode is the analysis's, after the receipt. A ZIP named for fonts holds
  // the post's fonts, not episodes. An archive is unpacked after it is
  // received: its files are the package's below, and it says how many of
  // them were new. A Drive font that did not change was not received: it
  // says so in place of the format of bytes that never came.
  const unpacked = file.unpack !== null ? unpackText(file.unpack) : null;
  const archive = archiveReceiptText(file.new_assets);
  const facts = [
    file.state !== "failed" && file.size !== null ? sizeText(file.size) : null,
    file.unchanged ? fontReceiptText("unchanged") : file.format !== null ? FORMAT_LABEL[file.format] : null,
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
      {unpacked !== null && (
        <p className="text-xs leading-snug text-text-secondary [overflow-wrap:anywhere]">
          <b className={cn("font-semibold", unpacked.urgent ? "text-urgent" : "text-text-primary")}>
            {unpacked.text}
          </b>
          {unpacked.reason !== null && <span> · {unpacked.reason}</span>}
        </p>
      )}
      {archive !== null && <p className="text-xs leading-snug text-text-secondary">{archive}</p>}
      {placements.map((p) => (
        <PlacementLine key={p.position} placement={p} waiting={waiting} />
      ))}
      {file.path !== null && (
        <p className="text-xs leading-snug text-text-muted [overflow-wrap:anywhere]">
          {file.path}
        </p>
      )}
    </li>
  );
}
