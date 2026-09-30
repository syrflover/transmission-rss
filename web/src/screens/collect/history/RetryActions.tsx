import type { ReactNode } from "react";

import { Button } from "@/components/ui/button";

import { btnAction, btnNeutral } from "../channels/styles";
import { DownloadIcon } from "./icons";
import type { RetryPhase } from "./useRetry";

interface RetryActionsProps {
  phase: RetryPhase;
  onSubmit: () => void;
  onResend: () => void;
  onRecheck: () => void;
  /** More buttons for the row of actions after the retry button. */
  children?: ReactNode;
}

/**
 * The `다시 받기` button of an expanded row. There is nothing to fill in: the
 * rule that picked the item decides the save folder and the episode
 * conversion, and the server decides whether the request is acceptable.
 */
export function RetryActions({ phase, onSubmit, onResend, onRecheck, children }: RetryActionsProps) {
  // While a request is in flight or unconfirmed the button is replaced or off.
  const locked = phase.kind === "sending" || phase.kind === "waiting" || phase.kind === "unconfirmed";

  return (
    <div className="flex flex-col gap-2.5">
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
          <Button type="button" variant="ghost" className={btnAction} disabled={locked} onClick={onSubmit}>
            <DownloadIcon className="size-[15px]" />
            다시 받기
          </Button>
        )}
        {children}
      </div>
    </div>
  );
}
