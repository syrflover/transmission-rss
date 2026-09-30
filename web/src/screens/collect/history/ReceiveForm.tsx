import { useId, useState, type FormEvent, type ReactNode } from "react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";

import { btnAction, btnNeutral, hintClass, inputClass, labelClass } from "../channels/styles";
import { DownloadIcon } from "./icons";
import type { ReceivePhase } from "./useReceiveOnce";

interface ReceiveFormProps {
  /** The channel's base folder; the folder typed here is below it. */
  baseDir: string;
  phase: ReceivePhase;
  onSubmit: (folder: string) => void;
  onResend: () => void;
  onRecheck: () => void;
  /** More buttons for the row of actions after the receive button. */
  children?: ReactNode;
}

/**
 * The 저장 폴더 field and the `한 번 받기` button of an expanded row. The
 * channel's base folder is fixed in front of the field; leaving it empty saves
 * to the base folder itself. The server decides what is acceptable.
 */
export function ReceiveForm({ baseDir, phase, onSubmit, onResend, onRecheck, children }: ReceiveFormProps) {
  const [folder, setFolder] = useState("");
  const inputId = useId();
  const hintId = useId();
  // While a request is in flight or unconfirmed the folder is fixed with its ID.
  const locked = phase.kind === "sending" || phase.kind === "waiting" || phase.kind === "unconfirmed";

  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (!locked) onSubmit(folder);
  };

  return (
    <form onSubmit={submit} className="flex flex-col gap-2.5">
      <div className="flex flex-col gap-1.5">
        <label htmlFor={inputId} className={labelClass}>
          저장 폴더
        </label>
        <p className="font-mono text-xs break-all text-text-muted">{baseDir.replace(/\/+$/, "")}/</p>
        <Input
          id={inputId}
          value={folder}
          onChange={(e) => setFolder(e.target.value)}
          disabled={locked}
          placeholder="예: LIAR GAME/Season 01"
          spellCheck={false}
          autoComplete="off"
          aria-describedby={hintId}
          aria-invalid={phase.kind === "idle" && phase.error !== null ? true : undefined}
          className={`${inputClass} font-mono`}
        />
        <p id={hintId} className={hintClass}>
          비워 두면 위의 기본 폴더에 받아요. 기본 폴더 밖으로 나가는 경로는 쓸 수 없어요.
        </p>
      </div>

      {phase.kind === "idle" && phase.error !== null && (
        <p role="alert" className="text-[13px] font-semibold text-urgent">
          {phase.error}
        </p>
      )}

      <div className="flex flex-wrap gap-2">
        {phase.kind === "unconfirmed" ? (
          phase.lost ? (
            <Button type="button" variant="ghost" className={btnAction} onClick={onResend}>
              같은 요청 다시 보내기
            </Button>
          ) : (
            <Button type="button" variant="ghost" className={btnNeutral} onClick={onRecheck}>
              다시 확인
            </Button>
          )
        ) : (
          <Button type="submit" variant="ghost" className={btnAction} disabled={locked}>
            <DownloadIcon className="size-[15px]" />
            한 번 받기
          </Button>
        )}
        {children}
      </div>
    </form>
  );
}
