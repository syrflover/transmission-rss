import { useId, useState } from "react";
import { Dialog } from "radix-ui";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { ApiError } from "@/lib/api";
import { cn } from "@/lib/utils";

import { btnAction, btnNeutral, btnPrimary, hintClass, inputClass } from "../../collect/channels/styles";
import {
  revertMapping,
  saveMapping,
  type CandidateMapping,
  type MappingChoice,
  type MappingException,
  type MappingRow,
  type SourceMapping,
} from "../api";
import { useMappingPreview } from "./useMappingPreview";

/** What the dialog and the buttons know about the season and the creator. */
export interface MappingTarget {
  workId: string;
  season: number;
  sourceId: string;
  creator: string;
  /** The creator's mapping now; `undefined` while it has none. */
  mapping: CandidateMapping | undefined;
  /** The episodes of the earlier seasons together when each is known. */
  previous: number | null | undefined;
}

const CHANGED_FIRST = "다른 곳에서 먼저 바꿨어요.";

/** The rows of the dialog for the exceptions a mapping has. */
function rowsOf(exceptions: readonly MappingException[]): MappingRow[] {
  return exceptions.map((e) => ({
    episode: e.episode,
    target: e.target === null ? "" : String(e.target),
    skip: e.target === null,
  }));
}

/** The mapping a `409` carries (`{ source_id, mapping }`), when it has that shape. */
function currentOf(error: unknown): { mapping: CandidateMapping | null } | null {
  if (!(error instanceof ApiError) || error.code !== "conflict") return null;
  const current = error.current as Partial<SourceMapping> | undefined;
  return current && typeof current === "object" && "mapping" in current ? { mapping: current.mapping ?? null } : null;
}

const optionClass =
  "flex cursor-pointer items-start gap-2.5 rounded-[10px] border border-hairline-soft p-3 has-[:checked]:border-focus has-[:disabled]:cursor-not-allowed has-[:disabled]:opacity-70";

function ChoiceOption({
  name,
  value,
  checked,
  disabled,
  onSelect,
  title,
  children,
}: {
  name: string;
  value: MappingChoice;
  checked: boolean;
  disabled?: boolean;
  onSelect: (value: MappingChoice) => void;
  title: string;
  children: React.ReactNode;
}) {
  return (
    <label className={optionClass}>
      <input
        type="radio"
        name={name}
        value={value}
        checked={checked}
        disabled={disabled}
        onChange={() => onSelect(value)}
        className="mt-0.5 size-[18px] flex-none accent-focus max-[720px]:size-5"
      />
      <span className="flex min-w-0 flex-1 flex-col gap-1">
        <span className="text-sm font-bold">{title}</span>
        {children}
      </span>
    </label>
  );
}

/** The preview of a choice: where the creator's episodes go under it, in one short text; dimmed while the next is asked. */
function Preview({ text, stale }: { text: string; stale: boolean }) {
  return text === "" ? null : (
    <span className={cn("text-xs leading-snug text-text-secondary [overflow-wrap:anywhere]", stale && "opacity-60")}>{text}</span>
  );
}

function Body({ target, onChanged, onClose }: { target: MappingTarget; onChanged: (mapping: CandidateMapping | null) => void; onClose: () => void }) {
  const { workId, season, sourceId, mapping, previous } = target;
  const radioName = useId();
  // The choice the stored mapping already means is the one preselected (the server names it); a source with no
  // basis has none, so nothing is saved by default.
  const first = mapping?.choice ?? null;
  const [choice, setChoice] = useState<MappingChoice | null>(first);
  const [custom, setCustom] = useState(first === "custom" && mapping?.offset != null ? String(mapping.offset) : "");
  const [rows, setRows] = useState<MappingRow[]>(() => rowsOf(mapping?.exceptions ?? []));
  const [version, setVersion] = useState(mapping?.version ?? 0);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // What the dialog shows for its input comes from the server; it can be saved once the answer is for this input.
  const { preview, fresh, failure } = useMappingPreview(workId, season, sourceId, { choice, custom, exceptions: rows });
  const reason = preview?.continue_reason ?? null;
  const stale = !fresh;
  const saved = fresh && preview !== null && preview.offset !== null && preview.exceptions_problem === null ? preview : null;

  const change = (index: number, part: Partial<MappingRow>) =>
    setRows((prev) => prev.map((row, i) => (i === index ? { ...row, ...part } : row)));

  const save = async () => {
    if (busy || saved === null || saved.offset === null) return;
    setBusy(true);
    setError(null);
    try {
      onChanged(await saveMapping(workId, season, sourceId, { version, offset: saved.offset, exceptions: saved.exceptions }));
      onClose();
    } catch (e) {
      const current = currentOf(e);
      if (current) {
        // The page shows the current mapping; the input stays so the user can look at both.
        setVersion(current.mapping?.version ?? 0);
        onChanged(current.mapping);
        setError(
          `${CHANGED_FIRST} 지금 저장된 대응은 ‘${current.mapping ? current.mapping.line : "없음"}’이에요. 입력은 그대로 두었어요. 그대로 저장하려면 다시 눌러 주세요.`,
        );
      } else {
        setError(e instanceof ApiError ? e.message : "대응을 저장하지 못했어요. 잠시 뒤 다시 시도해 주세요.");
      }
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-4 overflow-x-hidden overflow-y-auto overscroll-contain p-4 pb-[calc(16px+env(safe-area-inset-bottom,0px))]">
      <fieldset className="m-0 flex min-w-0 flex-col gap-2 border-0 p-0">
        <legend className="mb-1.5 p-0 text-[13px] font-semibold text-text-secondary">기본 대응</legend>
        <ChoiceOption name={radioName} value="same" checked={choice === "same"} onSelect={setChoice} title="같은 번호">
          <span className={hintClass}>Anissia의 회차 번호를 시즌의 회차 번호로 써요.</span>
          <Preview text={preview?.previews.same ?? ""} stale={stale} />
        </ChoiceOption>
        <ChoiceOption
          name={radioName}
          value="continue"
          checked={choice === "continue"}
          disabled={reason !== null}
          onSelect={setChoice}
          title="앞 시즌에 이어 셈"
        >
          <span className={hintClass}>
            {reason ?? `앞 시즌들의 ${previous}화에 이어 센 번호예요. 13화가 시즌의 1화가 되는 식이에요.`}
          </span>
          {reason === null && <Preview text={preview?.previews.continue ?? ""} stale={stale} />}
        </ChoiceOption>
        <ChoiceOption name={radioName} value="custom" checked={choice === "custom"} onSelect={setChoice} title="직접 차이">
          <span className={hintClass}>Anissia의 회차에 더할 수를 써요. 12화를 빼려면 -12예요.</span>
          <Input
            aria-label="더할 수"
            inputMode="numeric"
            value={custom}
            placeholder="-12"
            onFocus={() => setChoice("custom")}
            onChange={(e) => {
              setCustom(e.target.value);
              setChoice("custom");
            }}
            className={cn(inputClass, "w-28 max-w-full")}
          />
          {preview?.offset_problem && (
            <span role="alert" className="text-xs font-semibold text-urgent">
              {preview.offset_problem}
            </span>
          )}
          <Preview text={preview?.previews.custom ?? ""} stale={stale} />
        </ChoiceOption>
      </fieldset>

      <section aria-label="예외" className="flex flex-col gap-2 border-t border-hairline-soft pt-3">
        <h3 className="text-[13px] font-semibold text-text-secondary">예외</h3>
        <p className={hintClass}>기본 대응이 맞지 않는 회차는 하나씩 정해요. 예외가 기본 대응보다 먼저 적용돼요.</p>
        {rows.length > 0 && (
          <ul className="m-0 flex list-none flex-col gap-2 p-0">
            {rows.map((row, index) => (
              <li key={index} className="flex flex-wrap items-center gap-x-2 gap-y-1.5 rounded-[10px] border border-hairline-soft p-2">
                <Input
                  aria-label="Anissia의 회차"
                  value={row.episode}
                  placeholder="13.5"
                  onChange={(e) => change(index, { episode: e.target.value })}
                  className={cn(inputClass, "w-24 min-w-0")}
                />
                <span aria-hidden="true" className="text-text-muted">
                  →
                </span>
                <span className="inline-flex items-center gap-1">
                  <Input
                    aria-label="시즌의 회차"
                    inputMode="numeric"
                    value={row.target}
                    placeholder="3"
                    disabled={row.skip}
                    onChange={(e) => change(index, { target: e.target.value })}
                    className={cn(inputClass, "w-20 min-w-0")}
                  />
                  <span className="text-sm text-text-secondary">화</span>
                </span>
                <label className="flex min-h-9 items-center gap-1.5 text-[13px] max-[720px]:min-h-10">
                  <input
                    type="checkbox"
                    checked={row.skip}
                    onChange={(e) => change(index, { skip: e.target.checked })}
                    className="size-[18px] accent-focus max-[720px]:size-5"
                  />
                  받지 않음
                </label>
                <Button
                  type="button"
                  variant="ghost"
                  className={cn(btnNeutral, "ml-auto")}
                  onClick={() => setRows((prev) => prev.filter((_, i) => i !== index))}
                >
                  지우기
                </Button>
              </li>
            ))}
          </ul>
        )}
        {preview !== null && preview.warnings.length > 0 && (
          <ul className={cn("m-0 flex list-none flex-col gap-0.5 p-0", stale && "opacity-60")} aria-label="확인해 주세요">
            {preview.warnings.map((warning) => (
              <li key={warning} role="status" className="text-[13px] leading-relaxed font-semibold text-text-primary [overflow-wrap:anywhere]">
                확인해 주세요. {warning}. 그대로 저장할 수도 있어요.
              </li>
            ))}
          </ul>
        )}
        {preview?.exceptions_problem && (
          <p role="alert" className="m-0 text-[13px] leading-relaxed font-semibold text-urgent">
            {preview.exceptions_problem}
          </p>
        )}
        {preview !== null && preview.to_add.length > 0 && (
          <div className="flex flex-col gap-1.5">
            <p className={hintClass}>기본 대응으로 시즌에 맞지 않는 회차예요. 눌러서 예외로 더해요.</p>
            <div className="flex flex-wrap gap-1.5">
              {preview.to_add.map((episode) => (
                <button
                  key={episode}
                  type="button"
                  onClick={() => setRows((prev) => [...prev, { episode, target: "", skip: true }])}
                  className="min-h-9 rounded-full border border-hairline bg-surface-1 px-3 text-[13px] font-semibold hover:border-text-secondary max-[720px]:min-h-10"
                >
                  {episode} 더하기
                </button>
              ))}
            </div>
          </div>
        )}
        <Button
          type="button"
          variant="ghost"
          className={cn(btnNeutral, "self-start")}
          onClick={() => setRows((prev) => [...prev, { episode: "", target: "", skip: false }])}
        >
          예외 추가
        </Button>
      </section>

      <div className="flex flex-col gap-2 border-t border-hairline-soft pt-3">
        {failure && !error && (
          <p role="alert" className="m-0 text-[13px] leading-relaxed font-semibold text-urgent">
            {failure}
          </p>
        )}
        {error && (
          <p role="alert" className="m-0 text-[13px] leading-relaxed font-semibold text-urgent">
            {error}
          </p>
        )}
        {choice === null && <p className={hintClass}>기본 대응을 골라야 저장할 수 있어요.</p>}
        <div className="flex flex-wrap gap-2">
          <Button type="button" variant="ghost" className={btnPrimary} disabled={busy || saved === null} onClick={() => void save()}>
            {busy ? "저장하는 중…" : "저장"}
          </Button>
          <Button type="button" variant="ghost" className={btnNeutral} disabled={busy} onClick={onClose}>
            취소
          </Button>
        </div>
      </div>
    </div>
  );
}

/**
 * `회차 대응 정하기`: the user sets a creator's default offset and exceptions as one unit. Nothing is saved
 * until `저장`; when someone saved first, the dialog tells what is stored now and keeps the input.
 */
export function MappingDialog({
  target,
  open,
  onOpenChange,
  onChanged,
}: {
  target: MappingTarget;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onChanged: (mapping: CandidateMapping | null) => void;
}) {
  return (
    <Dialog.Root open={open} onOpenChange={onOpenChange}>
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-[250] bg-[rgba(4,6,10,0.62)] data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:animate-in data-[state=open]:fade-in-0" />
        <Dialog.Content
          className={cn(
            "fixed top-1/2 left-1/2 z-[251] flex max-h-[min(92dvh,860px)] w-[min(560px,calc(100vw-32px))] -translate-x-1/2 -translate-y-1/2 flex-col rounded-2xl border border-hairline bg-surface-1 shadow-[0_20px_50px_-16px_var(--shadow-color)] outline-none",
            "data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:animate-in data-[state=open]:fade-in-0",
          )}
        >
          <div className="flex items-center gap-2 border-b border-hairline-soft py-2 pr-2 pl-4">
            <div className="flex min-w-0 flex-1 flex-col">
              <Dialog.Title className="truncate text-[17px] font-bold">{target.creator}의 회차 대응</Dialog.Title>
              <Dialog.Description className="text-xs text-text-muted">
                시즌 {target.season}에서 Anissia의 회차를 몇 화로 받을지 정해요.
              </Dialog.Description>
            </div>
            <Dialog.Close
              aria-label="닫기"
              className="inline-flex size-11 flex-none items-center justify-center rounded-full text-text-secondary hover:bg-surface-2 hover:text-text-primary"
            >
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" aria-hidden="true" focusable="false" className="size-5">
                <path d="M6 6l12 12M18 6L6 18" />
              </svg>
            </Dialog.Close>
          </div>
          <Body target={target} onChanged={onChanged} onClose={() => onOpenChange(false)} />
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

/**
 * The buttons of an opened creator group: `회차 대응 정하기` for any creator, and for a mapping the user set
 * `자동으로 되돌리기`, which drops it so the app decides again.
 */
export function MappingControls({ target, onChanged }: { target: MappingTarget; onChanged: (mapping: CandidateMapping | null) => void }) {
  const [open, setOpen] = useState(false);
  const [reverting, setReverting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const { mapping } = target;

  const revert = async () => {
    if (!mapping || reverting) return;
    setReverting(true);
    setError(null);
    try {
      const answer = await revertMapping(target.workId, target.season, target.sourceId, mapping.version);
      onChanged(answer.mapping);
    } catch (e) {
      const current = currentOf(e);
      if (current) {
        onChanged(current.mapping);
        setError(`${CHANGED_FIRST} 지금 대응을 보여드려요. 되돌리려면 다시 눌러 주세요.`);
      } else {
        setError(e instanceof ApiError ? e.message : "자동으로 되돌리지 못했어요. 잠시 뒤 다시 시도해 주세요.");
      }
    } finally {
      setReverting(false);
    }
  };

  return (
    <div className="mt-2 flex flex-col gap-1.5">
      <div className="flex flex-wrap items-center gap-2">
        <Button type="button" variant="ghost" className={btnAction} aria-haspopup="dialog" onClick={() => setOpen(true)}>
          회차 대응 정하기
        </Button>
        {mapping?.kind === "user" && (
          <>
            <Button type="button" variant="ghost" className={btnNeutral} disabled={reverting} onClick={() => void revert()}>
              {reverting ? "되돌리는 중…" : "자동으로 되돌리기"}
            </Button>
            <span className={hintClass}>직접 정한 대응과 예외를 지우고 앱이 다시 정해요.</span>
          </>
        )}
      </div>
      {error && (
        <p role="alert" className="m-0 text-[13px] font-semibold text-urgent">
          {error}
        </p>
      )}
      <MappingDialog target={target} open={open} onOpenChange={setOpen} onChanged={onChanged} />
    </div>
  );
}
