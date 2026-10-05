import { useEffect, useId, useMemo, useState } from "react";

import { Button } from "@/components/ui/button";
import { ApiError } from "@/lib/api";
import { peek, store, useStored } from "@/lib/cached";
import { cn } from "@/lib/utils";

import { btnNeutral } from "../collect/channels/styles";
import { ChevronIcon } from "../library/icons";
import {
  fetchReplacementLines,
  type DialogueLine,
  type Replacement,
  type ReplacementLines,
  type TimingLine,
} from "./api";
import { Tag } from "./badges";
import {
  changesView,
  cutLines,
  dialogueRows,
  LINES_STEP,
  needsLines,
  timingRows,
  type ChangeTag,
  type ItemDetail,
  type ItemView,
  type RowText,
} from "./changes";
import { planEpisode } from "./replacementView";

const LINES_FAILED = "줄을 불러오지 못했어요. 잠시 뒤 다시 시도해 주세요.";

type LinesState =
  | { status: "idle" | "loading" }
  | { status: "error"; message: string }
  | { status: "ready"; lines: ReplacementLines };

/**
 * The dialogue and timing lines of a plan, read from the server once the first time `enabled` turns true and kept for
 * the life of the page: the job is polled, and the lines of a whole-file change can be hundreds of KB, so they never
 * come with it. A plan's comparison does not change, so the kept lines are not read again.
 */
function useLines(jobId: string, planId: string, enabled: boolean): { state: LinesState; retry: () => void } {
  const key = `todo:replacement-lines:${planId}`;
  const lines = useStored<ReplacementLines>(key);
  const [failure, setFailure] = useState<{ key: string; message: string } | null>(null);
  const [round, setRound] = useState(0);

  useEffect(() => {
    if (!enabled || peek<ReplacementLines>(key) !== undefined) return;
    const controller = new AbortController();
    setFailure(null);
    fetchReplacementLines(jobId, planId, controller.signal).then(
      (value) => store(key, value),
      (e: unknown) => {
        if (controller.signal.aborted) return;
        setFailure({ key, message: e instanceof ApiError ? e.message : LINES_FAILED });
      },
    );
    return () => controller.abort();
  }, [enabled, key, jobId, planId, round]);

  const state: LinesState =
    lines !== undefined
      ? { status: "ready", lines }
      : failure?.key === key
        ? { status: "error", message: failure.message }
        : { status: enabled ? "loading" : "idle" };
  return { state, retry: () => setRound((n) => n + 1) };
}

/**
 * `변경 사항` of an open replacement (`docs/specs/subtitles.md`, 교체 비교와 승인): the content of the new subtitle
 * against the current one, as the decision's body. Four items, collapsed at first, each with its number tags; an item
 * expands to what changed. The dialogue and timing lines are read when one of them is first opened.
 */
export function ChangesSection({ jobId, r, many }: { jobId: string; r: Replacement; many: boolean }) {
  const headingId = useId();
  const view = changesView(r.comparison);
  const [open, setOpen] = useState<ReadonlySet<string>>(new Set());
  const [wanted, setWanted] = useState(false);
  const { state, retry } = useLines(jobId, r.plan_id, wanted);

  const toggle = (item: ItemView) => {
    if (needsLines(item.key)) setWanted(true);
    setOpen((was) => {
      const next = new Set(was);
      if (!next.delete(item.key)) next.add(item.key);
      return next;
    });
  };

  return (
    <section aria-labelledby={headingId} className="mt-5 max-[720px]:mt-4">
      <h2 id={headingId} className="mb-2.5 text-[17px] font-bold max-[720px]:mb-2">
        {many ? `${planEpisode(r)} 변경 사항` : "변경 사항"}
      </h2>
      {view.kind === "none" ? (
        <div className="flex flex-col items-start gap-1.5 rounded-card border border-hairline-soft bg-surface-1 px-3.5 py-3 shadow-(--card-shadow)">
          <Tag>비교 불가</Tag>
          <p className="text-[13px] leading-snug text-text-secondary [overflow-wrap:anywhere]">{view.reason}</p>
        </div>
      ) : (
        <ul className="m-0 min-w-0 list-none rounded-card border border-hairline-soft bg-surface-1 p-0 shadow-(--card-shadow)">
          {view.items.map((item) => (
            <Item
              key={item.key}
              item={item}
              open={open.has(item.key)}
              onToggle={() => toggle(item)}
              state={state}
              retry={retry}
            />
          ))}
        </ul>
      )}
    </section>
  );
}

function NumberTag({ tag }: { tag: ChangeTag }) {
  return (
    <span
      className={cn(
        "inline-flex items-center rounded-md border border-hairline-soft bg-surface-2 px-1.5 py-px text-[11.5px] leading-snug whitespace-nowrap tabular-nums",
        tag.dim ? "font-medium text-text-muted" : "font-semibold text-text-primary",
      )}
    >
      {tag.text}
    </span>
  );
}

function Item({
  item,
  open,
  onToggle,
  state,
  retry,
}: {
  item: ItemView;
  open: boolean;
  onToggle: () => void;
  state: LinesState;
  retry: () => void;
}) {
  const panelId = useId();
  const head = (
    <>
      <span className="w-[3.25rem] flex-none text-[13.5px] font-bold">{item.name}</span>
      <span className="flex min-w-0 flex-1 flex-wrap items-center gap-1.5">
        {item.tags.map((tag) => (
          <NumberTag key={tag.text} tag={tag} />
        ))}
      </span>
    </>
  );
  return (
    <li className="min-w-0 border-t border-hairline-soft first:border-t-0">
      {item.detail === null ? (
        <div className="flex items-center gap-2 px-3.5 py-2.5">{head}</div>
      ) : (
        <>
          <button
            type="button"
            aria-expanded={open}
            aria-controls={panelId}
            onClick={onToggle}
            className="flex w-full items-center gap-2 rounded-card px-3.5 py-2.5 text-left hover:bg-surface-2 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-focus"
          >
            {head}
            <ChevronIcon
              className={cn("size-[15px] flex-none text-text-muted transition-transform", open && "rotate-90")}
            />
          </button>
          {open && (
            <div id={panelId} className="min-w-0 px-3.5 pt-0.5 pb-3">
              <Detail detail={item.detail} state={state} retry={retry} />
            </div>
          )}
        </>
      )}
    </li>
  );
}

function Detail({ detail, state, retry }: { detail: ItemDetail; state: LinesState; retry: () => void }) {
  switch (detail.kind) {
    case "reason":
      return <p className="text-[13px] leading-snug text-text-secondary [overflow-wrap:anywhere]">{detail.reason}</p>;
    case "dialogue":
      return (
        <div className="flex flex-col gap-2.5">
          {detail.notes.map((note) => (
            <p key={note} className="text-[12.5px] leading-snug text-text-secondary [overflow-wrap:anywhere]">
              {note}
            </p>
          ))}
          <Fetched state={state} retry={retry}>
            {(lines) => <DialogueList lines={lines.dialogue} />}
          </Fetched>
        </div>
      );
    case "timing":
      return (
        <Fetched state={state} retry={retry}>
          {(lines) => <TimingList lines={lines.timing} />}
        </Fetched>
      );
    case "styles":
      return (
        <div className="flex flex-col gap-2 text-[13px] leading-snug">
          <Names label="추가" names={detail.added} />
          {detail.changed.map((style) => (
            <div key={style.name} className="flex min-w-0 flex-col gap-0.5">
              <p className="font-semibold [overflow-wrap:anywhere]">
                <span className="mr-2 text-text-muted">변경</span>
                {style.name}
              </p>
              <ul className="m-0 flex list-none flex-col gap-0.5 p-0 pl-3 text-[12.5px]">
                {style.fields.map((f) => (
                  <li key={f.field} className="[overflow-wrap:anywhere]">
                    <span className="text-text-muted">{f.field}</span> {f.old === "" ? "(비어 있음)" : f.old} →{" "}
                    <span className="font-semibold">{f.new === "" ? "(비어 있음)" : f.new}</span>
                  </li>
                ))}
              </ul>
            </div>
          ))}
          <Names label="삭제" names={detail.removed} />
        </div>
      );
    case "fonts":
      return (
        <div className="flex flex-col gap-2 text-[13px] leading-snug">
          <Names label="추가" names={detail.added} />
          <Names label="삭제" names={detail.removed} />
        </div>
      );
  }
}

/** A label and the names under it; nothing when there are none. */
function Names({ label, names }: { label: string; names: readonly string[] }) {
  if (names.length === 0) return null;
  return (
    <p className="[overflow-wrap:anywhere]">
      <span className="mr-2 font-semibold text-text-muted">{label}</span>
      {names.join(", ")}
    </p>
  );
}

/** The loading line, the error with its retry, or what the lines draw. */
function Fetched({
  state,
  retry,
  children,
}: {
  state: LinesState;
  retry: () => void;
  children: (lines: ReplacementLines) => React.ReactNode;
}) {
  if (state.status === "ready") return <>{children(state.lines)}</>;
  if (state.status === "error") {
    return (
      <div className="flex flex-wrap items-center gap-x-3 gap-y-2">
        <p role="alert" className="text-[13px] font-semibold text-urgent [overflow-wrap:anywhere]">
          {state.message}
        </p>
        <Button type="button" variant="ghost" className={btnNeutral} onClick={retry}>
          다시 시도
        </Button>
      </div>
    );
  }
  return (
    <p role="status" className="text-[13px] text-text-muted">
      줄을 불러오는 중이에요.
    </p>
  );
}

/** The first lines of a list, and `n줄 더 보기` for the rest. */
function useCut(total: number) {
  const [asked, setAsked] = useState(LINES_STEP);
  const { visible, more } = cutLines(total, asked);
  return { visible, more, showMore: () => setAsked(visible + LINES_STEP) };
}

function MoreButton({ more, onClick }: { more: number; onClick: () => void }) {
  if (more === 0) return null;
  return (
    <Button type="button" variant="ghost" className={cn(btnNeutral, "mt-2 w-full")} onClick={onClick}>
      {more}줄 더 보기
    </Button>
  );
}

/** The page polls the job; the rows of a whole file's lines are made once, not on every refresh. */
function DialogueList({ lines }: { lines: readonly DialogueLine[] }) {
  const rows = useMemo(() => dialogueRows(lines), [lines]);
  const { visible, more, showMore } = useCut(rows.length);
  return (
    <div>
      <ol className="m-0 flex list-none flex-col p-0">
        {rows.slice(0, visible).map((row) => (
          <li key={row.key} className="flex flex-col gap-1 border-t border-hairline-soft py-2 first:border-t-0">
            <p className="flex flex-wrap items-center gap-x-2 gap-y-1">
              <time className="text-xs font-semibold text-text-muted tabular-nums">{row.time}</time>
              {row.class !== null && <Tag>{row.class}</Tag>}
            </p>
            <LineText mark="−" label="이전" text={row.old} />
            <LineText mark="+" label="새" text={row.new} />
          </li>
        ))}
      </ol>
      <MoreButton more={more} onClick={showMore} />
    </div>
  );
}

/**
 * One row of a changed line: `−` and the old text struck through in the muted colour, or `+` and the new text
 * underlined. A line that did not exist on that side has a bar as long as the row, in the strikethrough's colour and
 * thickness, so the two rows keep their places.
 */
function LineText({ mark, label, text }: { mark: "−" | "+"; label: string; text: RowText }) {
  const old = mark === "−";
  return (
    <div className="grid grid-cols-[1rem_minmax(0,1fr)] gap-x-1.5 text-[13.5px] leading-snug">
      <span aria-hidden className="font-bold text-text-muted">
        {mark}
      </span>
      {text.bar ? (
        <span className="flex h-[1.375em] items-center">
          <span aria-hidden className="h-px w-full bg-text-muted" />
          <span className="sr-only">{label} 줄 없음</span>
        </span>
      ) : (
        <p
          className={cn(
            "whitespace-pre-line [overflow-wrap:anywhere]",
            old
              ? "text-text-muted line-through decoration-1"
              : "text-text-primary underline decoration-1 underline-offset-2",
          )}
        >
          <span className="sr-only">{label} 줄: </span>
          {text.text}
        </p>
      )}
    </div>
  );
}

function TimingList({ lines }: { lines: readonly TimingLine[] }) {
  const rows = useMemo(() => timingRows(lines), [lines]);
  const { visible, more, showMore } = useCut(rows.length);
  return (
    <div>
      <ol className="m-0 flex list-none flex-col p-0">
        {rows.slice(0, visible).map((row) => (
          <li key={row.key} className="flex flex-col gap-0.5 border-t border-hairline-soft py-2 first:border-t-0">
            <p className="text-[13.5px] leading-snug whitespace-pre-line [overflow-wrap:anywhere]">
              {row.class !== null && (
                <span className="mr-2 align-[1px]">
                  <Tag>{row.class}</Tag>
                </span>
              )}
              {row.text}
            </p>
            <p className="flex flex-wrap gap-x-3 gap-y-0.5 text-xs text-text-secondary tabular-nums">
              {row.moves.map((m) => (
                <span key={m.name} className="whitespace-nowrap">
                  {m.name} {m.from} → {m.to}
                </span>
              ))}
            </p>
          </li>
        ))}
      </ol>
      <MoreButton more={more} onClick={showMore} />
    </div>
  );
}
