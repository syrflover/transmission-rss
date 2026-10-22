import { useEffect, useId, useRef, useState } from "react";

import { Button } from "@/components/ui/button";
import { ago, dateTime } from "@/lib/time";
import { cn } from "@/lib/utils";

import { btnAction, btnDangerSolid, btnNeutral } from "../../collect/channels/styles";
import { channelName, ruleTitle } from "../../collect/rules/api";
import type { WorkDetail, WorkRule } from "../api";
import { archiveNotice, type WorkArchivePhase } from "./archiveNotice";

interface WorkArchiveNoticeProps {
  work: WorkDetail;
  phase: WorkArchivePhase;
  /** `다시 옮기기`: a new move of the work, with no rule to delete. */
  onMoveAgain: () => void;
  onResend: () => void;
  onRecheck: () => void;
}

/**
 * Where the work's `보관 폴더로 옮기기` is, in sentences: moving, moved, kept (and why) or not moved (and why). A move
 * that did not go through stays shown from the server while the work is in the collect folder with no rule saving
 * into it, and `다시 옮기기` tries again.
 */
export function WorkArchiveNotice({ work, phase, onMoveAgain, onResend, onRecheck }: WorkArchiveNoticeProps) {
  const notice = archiveNotice(work, phase);
  if (notice === null) return null;
  const { line, error, moveAgain } = notice;

  return (
    <section
      role="status"
      aria-label="보관 폴더로 옮기기"
      data-testid="work-archive"
      className={cn(
        "mb-5 flex min-w-0 flex-col gap-2 rounded-xl border bg-surface-2 px-3.5 py-3 text-[13px] leading-normal",
        line?.failed || error ? "border-[color-mix(in_srgb,var(--accent-urgent)_50%,transparent)]" : "border-hairline",
      )}
    >
      {line && (
        <p className={cn("font-bold", line.failed ? "text-urgent" : "text-text-primary")}>
          {line.title}
          {line.at !== null && (
            <span className="ml-1.5 font-normal text-text-muted" title={dateTime(line.at)}>
              {ago(line.at)}
            </span>
          )}
        </p>
      )}
      {line?.detail && <p className="break-words text-text-secondary">{line.detail}</p>}
      {error && (
        <p role="alert" className="font-semibold break-words text-urgent">
          {error}
        </p>
      )}
      {(moveAgain || phase.kind === "unconfirmed") && (
        <div className="flex flex-wrap gap-2">
          {moveAgain && (
            <Button type="button" variant="ghost" className={btnAction} onClick={onMoveAgain}>
              다시 옮기기
            </Button>
          )}
          {phase.kind === "unconfirmed" &&
            (phase.lost ? (
              <Button type="button" variant="ghost" className={btnAction} onClick={onResend}>
                같은 요청 다시 보내기
              </Button>
            ) : (
              <Button type="button" variant="ghost" className={btnNeutral} onClick={onRecheck}>
                다시 확인
              </Button>
            ))}
        </div>
      )}
    </section>
  );
}

const STATE: Record<WorkRule["state"], string> = {
  active: "수집 중",
  paused: "멈춤",
  archived: "보관됨",
};

interface ArchiveActionProps {
  work: WorkDetail;
  /** A move is under way: no new one is offered. */
  busy: boolean;
  /** The move was confirmed with the rules the confirmation showed. */
  onConfirm: (rules: WorkRule[]) => void;
}

/**
 * `보관 폴더로 옮기기` in the `수집` card, for a work in the collect folder when an archive folder is set. Pressing it
 * opens a confirmation in place that lists the rules saving into the work folder, every state, with their channel and
 * save folder: confirming deletes them and moves the folder, cancelling changes nothing.
 */
export function ArchiveAction({ work, busy, onConfirm }: ArchiveActionProps) {
  const [confirming, setConfirming] = useState(false);
  const uid = useId();
  const confirmRef = useRef<HTMLDivElement>(null);
  const buttonRef = useRef<HTMLButtonElement>(null);
  const opened = useRef(false);

  useEffect(() => {
    if (confirming) {
      opened.current = true;
      confirmRef.current?.focus();
    } else if (opened.current) {
      buttonRef.current?.focus();
    }
  }, [confirming]);

  if (!work.archive.available) return null;
  const rules = work.rules;
  const count = rules.length;

  if (!confirming) {
    return (
      <Button
        ref={buttonRef}
        type="button"
        variant="ghost"
        className={cn(btnNeutral, "self-start")}
        disabled={busy}
        onClick={() => setConfirming(true)}
      >
        보관 폴더로 옮기기
      </Button>
    );
  }

  return (
    <div
      ref={confirmRef}
      tabIndex={-1}
      role="group"
      aria-labelledby={`${uid}-title`}
      className="flex flex-col gap-2.5 rounded-xl border border-[color-mix(in_srgb,var(--accent-urgent)_50%,transparent)] p-3.5 outline-offset-2"
    >
      <p id={`${uid}-title`} className="text-sm font-bold">
        {count === 0 ? "작품 폴더를 보관 폴더로 옮길까요?" : `규칙 ${count}개를 지우고 보관 폴더로 옮길까요?`}
      </p>
      {count > 0 && (
        <ul className="m-0 flex list-none flex-col gap-1.5 p-0" aria-label="지울 규칙">
          {rules.map((rule) => (
            <li key={rule.id} className="flex min-w-0 flex-col gap-0.5 rounded-[10px] bg-surface-2 px-3 py-2">
              <span className="flex flex-wrap items-center gap-x-2 gap-y-1">
                <span className={cn("text-[13px] font-bold", rule.match === null && "text-text-secondary")}>{ruleTitle(rule)}</span>
                <span className="rounded-full border border-hairline px-1.5 py-px text-[11.5px] font-semibold text-text-secondary">
                  {STATE[rule.state]}
                </span>
              </span>
              <span className="text-xs text-text-muted">{channelName(rule.channel)}</span>
              <span className="font-mono text-[12px] break-all text-text-secondary">{rule.directory}</span>
            </li>
          ))}
        </ul>
      )}
      <p className="text-[13px] leading-normal text-text-secondary">
        {count === 0
          ? "작품 폴더를 수집 폴더에서 보관 폴더로 옮겨요."
          : "이 규칙을 모두 지운 뒤 작품 폴더를 수집 폴더에서 보관 폴더로 옮겨요. 이미 받은 파일과 수집 기록은 남아요. 지운 규칙은 되돌릴 수 없어요."}
      </p>
      <div className="flex flex-wrap gap-2">
        <Button
          type="button"
          variant="ghost"
          className={count === 0 ? btnAction : btnDangerSolid}
          disabled={busy}
          onClick={() => {
            setConfirming(false);
            onConfirm(rules);
          }}
        >
          {count === 0 ? "옮기기" : "규칙 지우고 옮기기"}
        </Button>
        <Button type="button" variant="ghost" className={btnNeutral} onClick={() => setConfirming(false)}>
          취소
        </Button>
      </div>
    </div>
  );
}
