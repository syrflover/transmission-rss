import { useState } from "react";

import { ApiError } from "@/lib/api";
import { isOpen } from "@/lib/commands";
import { cn } from "@/lib/utils";

import { btnAction, btnNeutral, hintClass } from "../channels/styles";
import { applyEpisode, type EpisodeUndo, type Rule } from "./api";
import { useEpisodeUndo, type UndoPhase } from "./useEpisodeUndo";

/**
 * What the app says about the rule's episode offset under its field: the
 * grounds of a value it set (the `자동` mark sits on the label) with
 * `되돌리기`, or a suggestion with `적용`. Applying saves the offset at once as
 * the user's own, like the switches; a stale version takes the current rule
 * and says so.
 *
 * `되돌리기` puts back the value the app replaced, through the worker
 * (`episode_undo`): the rule's automatic mark goes, the app never decides the
 * rule again, and the videos received under the app's value are renamed to the
 * restored one. The files it could not rename stay listed with the reason. An
 * undo that stopped with files still to rename (the value is back already)
 * stays shown, whatever the rule's mark, with `이어서 되돌리기`.
 *
 * `shown` is false while the field holds something other than the stored
 * offset: the grounds and the suggestion are about the stored one.
 */
export function EpisodeGrounds({
  rule,
  shown,
  disabled,
  onChanged,
}: {
  rule: Rule;
  shown: boolean;
  /** Something else is changing the rule (a save, an archive move). */
  disabled: boolean;
  /** The rule as the server has it after `적용` or `되돌리기`, or after the conflict that showed it. */
  onChanged: (rule: Rule) => void;
}) {
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  /** An undo ended while this screen followed it: its summary is worth a line. */
  const [undone, setUndone] = useState(false);
  const undo = useEpisodeUndo(rule, (fresh) => {
    setUndone(true);
    onChanged(fresh);
  });
  const undoing = undo.phase.kind === "sending" || undo.phase.kind === "waiting" || undo.phase.kind === "unconfirmed";

  const suggestion = shown ? rule.episode_suggestion : null;
  const grounds = shown && rule.episode_auto ? (rule.episode_basis ?? "앱이 정한 값이에요.") : null;
  const undoable = rule.episode_auto && rule.episode_previous !== null;
  const last = rule.episode_undo && !isOpen(rule.episode_undo.command) ? rule.episode_undo : null;
  const outcome = undoOutcome(undo.phase, last, undone, rule);
  if (grounds === null && suggestion === null && outcome === null) return null;

  const apply = async (value: number) => {
    setBusy(true);
    setMessage(null);
    try {
      onChanged(await applyEpisode(rule, value));
    } catch (e) {
      if (e instanceof ApiError && e.code === "conflict" && e.current) {
        onChanged(e.current as Rule);
        setMessage("다른 곳에서 먼저 바꿨어요. 지금 상태를 보여드려요.");
      } else {
        setMessage(e instanceof ApiError ? e.message : "적용하지 못했어요. 잠시 뒤 다시 시도해 주세요.");
      }
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="flex min-w-0 flex-col gap-1.5" data-testid="episode-grounds">
      {grounds !== null && (
        <p className={hintClass} data-testid="episode-basis">
          {grounds} 직접 바꾸면 자동 표시가 사라지고, 앱은 그 값을 다시 바꾸지 않아요.
        </p>
      )}
      {grounds !== null && undoable && (
        <div className="flex min-w-0 flex-wrap items-center gap-2">
          <button
            type="button"
            className={`${btnNeutral} max-[720px]:min-h-11`}
            disabled={busy || disabled || undoing}
            onClick={() => undo.submit()}
          >
            되돌리기
          </button>
          <span className={hintClass}>
            전의 값으로 돌아가고, 이 값으로 받은 영상의 이름도 그 값에 맞게 바꿔요. 앱은 이 규칙의 값을 다시 정하지
            않아요.
          </span>
        </div>
      )}
      {outcome !== null && (
        <section
          role="status"
          aria-label="회차 변환 되돌리기"
          data-testid="episode-undo"
          className={cn(
            "flex min-w-0 flex-col gap-1.5 rounded-lg border bg-surface-2 p-2.5 text-[12.5px] leading-normal",
            outcome.failed ? "border-[color-mix(in_srgb,var(--accent-urgent)_50%,transparent)]" : "border-hairline",
          )}
        >
          <p className={cn("font-semibold", outcome.failed ? "text-urgent" : "text-text-primary")}>{outcome.title}</p>
          {outcome.detail && <p className="break-words text-text-secondary">{outcome.detail}</p>}
          {outcome.kept.length > 0 && (
            <ul className="m-0 flex list-none flex-col gap-1 p-0">
              {outcome.kept.map((line) => (
                <li key={line} className="break-words text-text-secondary">
                  {line}
                </li>
              ))}
            </ul>
          )}
          {outcome.resume !== null && (
            <div>
              <button
                type="button"
                className={`${btnAction} max-[720px]:min-h-11`}
                disabled={busy || disabled || undoing}
                onClick={() => undo.submit(outcome.resume!)}
              >
                이어서 되돌리기
              </button>
            </div>
          )}
          {undo.phase.kind === "unconfirmed" && undo.phase.lost && (
            <div>
              <button type="button" className={btnAction} onClick={undo.resend}>
                같은 요청 다시 보내기
              </button>
            </div>
          )}
        </section>
      )}
      {suggestion !== null && (
        <div
          className="flex min-w-0 flex-col gap-2 rounded-lg border border-hairline bg-surface-2 p-2.5"
          data-testid="episode-suggestion"
        >
          <p className="text-[12.5px] leading-normal text-text-secondary">{suggestion.basis}</p>
          {suggestion.value !== null && (
            <div className="flex min-w-0 flex-wrap items-center gap-2">
              <span className="font-mono text-[13px]">{suggestion.value}</span>
              <button
                type="button"
                className={`${btnNeutral} max-[720px]:min-h-11`}
                disabled={busy || disabled}
                onClick={() => void apply(suggestion.value!)}
              >
                적용
              </button>
            </div>
          )}
        </div>
      )}
      {message !== null && (
        <p role="status" className={`${hintClass} font-semibold`}>
          {message}
        </p>
      )}
    </div>
  );
}

interface Outcome {
  title: string;
  detail: string | null;
  /** `이름을 되돌리지 못했어요: …` for each file left as it was. */
  kept: string[];
  failed: boolean;
  /** The value `이어서 되돌리기` asks to undo, for an undo that stopped half done. */
  resume: number | null;
}

/** The files an undo left as they were, a sentence each. */
function keptLines(undo: EpisodeUndo): string[] {
  return undo.files
    .filter((f) => f.state === "kept")
    .map((f) => `이름을 되돌리지 못했어요: ${f.from_name} → ${f.to_name}. ${f.reason ?? "까닭은 알 수 없어요."}`);
}

/** The files an undo has still to rename, a sentence each, with why one waits. */
function pendingLines(undo: EpisodeUndo): string[] {
  return undo.files
    .filter((f) => f.state === "pending")
    .map((f) => `아직 이름을 바꾸지 않았어요: ${f.from_name} → ${f.to_name}.${f.reason ? ` ${f.reason}` : ""}`);
}

/**
 * What the undo box says: the undo under way and its files so far, or the last
 * one that ended. One that stopped with files still to rename shows them,
 * whatever the rule's mark, until it is carried on. A finished undo keeps
 * saying which files it could not rename; its summary shows only right after
 * this screen followed it. A failed one shows while the value it was about is
 * still the rule's.
 */
function undoOutcome(phase: UndoPhase, last: EpisodeUndo | null, undone: boolean, rule: Rule): Outcome | null {
  switch (phase.kind) {
    case "sending":
      return {
        title: "회차 변환을 되돌리는 중이에요.",
        detail: "접수하는 중이에요.",
        kept: [],
        failed: false,
        resume: null,
      };
    case "waiting": {
      const files = phase.progress?.files ?? [];
      const ended = files.filter((f) => f.state !== "pending").length;
      return {
        title: "회차 변환을 되돌리는 중이에요.",
        detail:
          files.length > 0
            ? `영상 ${files.length}개 중 ${ended}개를 살펴봤어요.`
            : "접수됐어요. 끝나면 여기에 결과가 나와요.",
        kept: phase.progress ? keptLines(phase.progress) : [],
        failed: false,
        resume: null,
      };
    }
    case "unconfirmed":
      return {
        title: "회차 변환을 되돌리는 중이에요.",
        detail: phase.lost
          ? "접수됐는지 확인했더니 서버에 요청이 없어요. 같은 요청을 다시 보낼 수 있어요."
          : "접수됐는지 확인하지 못했어요. 결과를 알 수 없어 다시 확인하고 있어요.",
        kept: [],
        failed: false,
        resume: null,
      };
    case "ended":
      return {
        title: phase.command.state === "failed" ? "되돌리지 못했어요." : "회차 변환을 되돌렸어요.",
        detail: phase.command.outcome?.reason ?? null,
        kept: [],
        failed: phase.command.state === "failed",
        resume: null,
      };
    case "idle":
      break;
  }
  if (phase.error !== null) {
    return { title: "되돌리지 못했어요.", detail: phase.error, kept: [], failed: true, resume: null };
  }
  if (last === null) return null;
  const pending = pendingLines(last);
  if (pending.length > 0 && last.from !== null) {
    // A paused undo says itself why it stopped; a failed one (an internal
    // error) gets the reason after the summary.
    const why = last.command.outcome?.result === "paused" ? null : last.command.outcome?.reason;
    return {
      title: "되돌리다 멈췄어요.",
      detail:
        `회차 변환은 이미 전의 값으로 돌아왔어요${last.to === null ? "" : `(${last.to})`}. ` +
        `영상 ${last.files.length}개 중 ${pending.length}개는 아직 이름을 바꾸지 않았어요. ` +
        "이어서 되돌리면 남은 영상의 이름을 마저 바꿔요." +
        (why ? ` 멈춘 까닭: ${why}` : ""),
      kept: [...pending, ...keptLines(last)],
      failed: true,
      resume: last.from,
    };
  }
  if (last.command.state === "failed") {
    // About a value the rule no longer has, it says nothing now.
    if (!rule.episode_auto) return null;
    return {
      title: "되돌리지 못했어요.",
      detail: last.command.outcome?.reason ?? "까닭은 알 수 없어요.",
      kept: [],
      failed: true,
      resume: null,
    };
  }
  const kept = keptLines(last);
  if (!undone && kept.length === 0) return null;
  return {
    title: "회차 변환을 되돌렸어요.",
    detail: undone ? (last.command.outcome?.reason ?? null) : null,
    kept,
    failed: false,
    resume: null,
  };
}
