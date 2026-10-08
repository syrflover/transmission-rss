import { useEffect, useRef, useState } from "react";
import { useNavigate } from "react-router-dom";

import { Button } from "@/components/ui/button";
import { ApiError } from "@/lib/api";
import { forget } from "@/lib/cached";

import { btnAction, btnNeutral } from "../../collect/channels/styles";
import { POLICY_KEY, type SubtitleFormat } from "../../settings/policy/api";
import { jobPath } from "../../todo/api";
import { applyStored, deleteSubtitleOrder, putSubtitleOrder, type FormatOrder, type WorkSubtitles } from "../api";
import { ApplyStatus, JobLink } from "./ApplyStatus";
import { Card } from "./SideCards";
import {
  ACTION_LABEL,
  actionMode,
  actionName,
  applyOutcome,
  canSaveOrder,
  cardSummary,
  copyView,
  formatText,
  groupTitle,
  moveFormat,
  orderOwnerText,
  orderText,
  seasonGroups,
  type CopyAction,
  type CopyView,
} from "./subtitles.ts";

const subHeading = "text-xs font-bold text-text-muted";

/** The buttons are at least 40px tall on a phone (`btnNeutral` and `btnAction` do that). */
const rowBox = "flex flex-col gap-1.5 rounded-[10px] border border-hairline-soft bg-surface-2 px-3 py-2.5";

/**
 * `자막`: the chosen season's received subtitles by creator, the one beside each video marked `적용`, and the order of
 * the formats the work picks by. A copy is applied, compared with the episode's subtitle (the job's page opens) or
 * added beside the applied one by the server's say (`choice`, `can_add`, `blocked`). `onChanged` reads the work again;
 * `onOrder` shows the order the server saved.
 */
export function SubtitlesCard({
  workId,
  subtitles,
  season,
  collapsible,
  onChanged,
  onOrder,
}: {
  workId: string;
  subtitles: WorkSubtitles;
  season: number;
  collapsible: boolean;
  onChanged: () => Promise<void>;
  onOrder: (order: FormatOrder) => void;
}) {
  const groups = seasonGroups(subtitles, season);
  return (
    <Card title="자막" summary={cardSummary(groups)} collapsible={collapsible}>
      <OrderLine workId={workId} order={subtitles.format_order} onSaved={onOrder} />
      <div className="flex flex-col gap-3 border-t border-hairline-soft pt-3">
        {groups.length === 0 ? (
          <p className="text-[13px] leading-relaxed text-text-muted">이 시즌에 보관한 자막이 없어요.</p>
        ) : (
          groups.map((group) => (
            <section key={group.creator ?? ""} className="flex flex-col gap-1.5" aria-label={groupTitle(group)}>
              <h3 className="text-[12.5px] font-bold text-text-secondary">{groupTitle(group)}</h3>
              <ul className="m-0 flex list-none flex-col gap-2 p-0">
                {group.copies.map((copy) => (
                  <li key={copy.id} className="min-w-0">
                    <CopyRow workId={workId} view={copyView(copy, group.copies)} onChanged={onChanged} />
                  </li>
                ))}
              </ul>
            </section>
          ))
        )}
      </div>
    </Card>
  );
}

type Phase =
  | { kind: "idle" }
  | { kind: "sending"; action: CopyAction }
  | { kind: "sent"; job: string }
  | { kind: "error"; text: string; job: string | null };

/** One stored copy: what it is, where it lies when applied, and what can be done with it. */
function CopyRow({ workId, view, onChanged }: { workId: string; view: CopyView; onChanged: () => Promise<void> }) {
  const navigate = useNavigate();
  const [phase, setPhase] = useState<Phase>({ kind: "idle" });
  const sending = phase.kind === "sending";

  const ask = async (action: CopyAction) => {
    setPhase({ kind: "sending", action });
    try {
      const outcome = applyOutcome(await applyStored(workId, view.id, actionMode(action)));
      if (outcome.kind === "compare") {
        // The job plans a replacement and waits for `교체 승인` in its own page.
        navigate(jobPath(outcome.job));
        return;
      }
      // The job only took the apply: `ApplyStatus` reads the work again once the job ends and the work shows it.
      setPhase({ kind: "sent", job: outcome.job });
    } catch (e) {
      setPhase({ kind: "error", text: e instanceof ApiError ? e.message : "적용을 요청하지 못했어요.", job: null });
    }
  };

  return (
    <div className={rowBox}>
      <span className="flex flex-wrap items-center gap-x-2 gap-y-1">
        <span className="text-[13.5px] font-bold">{view.episode}</span>
        <span className="text-[12.5px] font-semibold text-text-secondary">{view.format}</span>
        {view.isApplied && (
          <span className="rounded-full border border-focus px-1.5 py-px text-[11.5px] font-bold text-focus">적용</span>
        )}
        <span className="text-xs text-text-muted">{view.received}</span>
      </span>
      <span title={view.name} className="font-mono text-[12px] leading-snug [overflow-wrap:anywhere]">
        {view.name}
      </span>
      {view.places && (
        <dl className="m-0 flex flex-col gap-1 text-[12px] leading-snug">
          {view.places.applied.map((path) => (
            <Place key={path} label="적용 위치" path={path} />
          ))}
          <Place label="보관 위치" path={view.places.stored} />
        </dl>
      )}
      {phase.kind === "sent" ? (
        <ApplyStatus
          job={phase.job}
          workId={workId}
          storedId={view.id}
          onEnded={onChanged}
          onFailed={(text) => setPhase({ kind: "error", text, job: phase.job })}
        />
      ) : view.blocked !== null ? (
        <span className="text-xs leading-snug text-text-muted">{view.blocked}</span>
      ) : (
        view.actions.length > 0 && (
          <span className="flex flex-wrap items-center gap-2">
            {view.actions.map((action) => (
              <Button
                key={action}
                type="button"
                variant="ghost"
                className={action === "compare" ? btnNeutral : btnAction}
                aria-label={actionName(view, action)}
                disabled={sending}
                onClick={() => void ask(action)}
              >
                {phase.kind === "sending" && phase.action === action ? "요청하는 중…" : ACTION_LABEL[action]}
              </Button>
            ))}
          </span>
        )
      )}
      {phase.kind === "error" && (
        <span role="alert" className="text-xs leading-snug text-urgent">
          {phase.text}
          {phase.job !== null && (
            <>
              {" "}
              <JobLink job={phase.job} />
            </>
          )}
        </span>
      )}
    </div>
  );
}

/** A labelled place of a copy: the label tells the applied file beside the video from the stored one. */
function Place({ label, path }: { label: string; path: string }) {
  return (
    <div className="grid grid-cols-[4.6em_minmax(0,1fr)] gap-x-2">
      <dt className="font-semibold text-text-muted">{label}</dt>
      <dd className="m-0 font-mono [overflow-wrap:anywhere] text-text-secondary">{path}</dd>
    </div>
  );
}

/** The order the work picks the format of a first apply by, and the editor that changes it. */
function OrderLine({
  workId,
  order,
  onSaved,
}: {
  workId: string;
  order: FormatOrder;
  onSaved: (order: FormatOrder) => void;
}) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState<SubtitleFormat[]>(order.order);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const list = useRef<HTMLOListElement>(null);
  const opener = useRef<HTMLButtonElement>(null);
  // The move button to keep focus on after the list reorders, and whether the editor just closed.
  const [focus, setFocus] = useState<{ format: SubtitleFormat; down: boolean } | null>(null);
  const closed = useRef(false);

  useEffect(() => {
    if (focus === null) return;
    const buttons = list.current?.querySelectorAll<HTMLButtonElement>(`[data-format="${focus.format}"]`);
    const [up, down] = [buttons?.[0], buttons?.[1]];
    const wanted = focus.down ? down : up;
    (wanted && !wanted.disabled ? wanted : focus.down ? up : down)?.focus();
    setFocus(null);
  }, [focus]);

  // The editor is gone: the focus goes back to the button that opened it.
  useEffect(() => {
    if (editing || !closed.current) return;
    closed.current = false;
    opener.current?.focus();
  }, [editing]);

  const open = () => {
    setDraft(order.order);
    setError(null);
    setEditing(true);
  };
  const close = () => {
    closed.current = true;
    setEditing(false);
  };

  const move = (index: number, by: -1 | 1) => {
    setDraft(moveFormat(draft, index, by));
    setFocus({ format: draft[index], down: by === 1 });
  };

  const run = async (change: () => Promise<FormatOrder>) => {
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      const saved = await change();
      // The settings list names the works with their own order.
      forget(POLICY_KEY);
      onSaved(saved);
      close();
    } catch (e) {
      setError(e instanceof ApiError ? e.message : "순서를 저장하지 못했어요. 잠시 뒤 다시 시도해 주세요.");
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="flex flex-col gap-1.5">
      <h3 className={subHeading}>형식 순서</h3>
      <p className="flex flex-wrap items-baseline gap-x-2 text-[13px]">
        <span className="font-semibold">{orderText(order.order)}</span>
        <span className="text-xs text-text-muted">{orderOwnerText(order)}</span>
      </p>
      <p className="text-xs leading-relaxed text-text-muted">이 작품의 다음 첫 적용은 위에 있는 형식부터 골라요.</p>
      {editing ? (
        <div className="flex flex-col gap-2">
          <ol ref={list} className="m-0 flex list-none flex-col gap-2 p-0" aria-label="형식 순서 바꾸기">
            {draft.map((format, index) => (
              <li key={format} className="flex items-center gap-2 rounded-[10px] border border-hairline bg-surface-2 px-3 py-1.5">
                <span className="grid size-5 flex-none place-items-center rounded-full bg-surface-3 text-[11.5px] font-bold text-text-secondary">
                  {index + 1}
                </span>
                <span className="min-w-0 flex-1 text-[13px] font-bold">{formatText(format)}</span>
                <Button
                  type="button"
                  variant="ghost"
                  data-format={format}
                  className={btnNeutral}
                  aria-label={`${formatText(format)} 위로`}
                  disabled={index === 0 || busy}
                  onClick={() => move(index, -1)}
                >
                  위로
                </Button>
                <Button
                  type="button"
                  variant="ghost"
                  data-format={format}
                  className={btnNeutral}
                  aria-label={`${formatText(format)} 아래로`}
                  disabled={index === draft.length - 1 || busy}
                  onClick={() => move(index, 1)}
                >
                  아래로
                </Button>
              </li>
            ))}
          </ol>
          {error !== null && (
            <p role="alert" className="text-xs leading-snug font-semibold text-urgent">
              {error}
            </p>
          )}
          <div className="flex flex-wrap items-center gap-2">
            <Button
              type="button"
              variant="ghost"
              className={btnAction}
              disabled={busy || !canSaveOrder(draft, order)}
              onClick={() => void run(() => putSubtitleOrder(workId, draft))}
            >
              {busy ? "저장하는 중…" : "저장"}
            </Button>
            <Button type="button" variant="ghost" className={btnNeutral} disabled={busy} onClick={close}>
              취소
            </Button>
            {order.own && (
              <Button
                type="button"
                variant="ghost"
                className={btnNeutral}
                disabled={busy}
                onClick={() => void run(() => deleteSubtitleOrder(workId))}
              >
                전역 순서로 되돌리기
              </Button>
            )}
          </div>
        </div>
      ) : (
        <span>
          <Button
            ref={opener}
            type="button"
            variant="ghost"
            className={btnNeutral}
            aria-label="형식 순서 바꾸기"
            onClick={open}
          >
            바꾸기
          </Button>
        </span>
      )}
    </div>
  );
}
