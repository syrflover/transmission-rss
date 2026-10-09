import { useEffect, useId, useRef, useState, type KeyboardEvent } from "react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { cn } from "@/lib/utils";

import { btnAction, btnNeutral } from "../../collect/channels/styles";
import type { PageDialog } from "./protocol";

const LEAVE_TEXT = "이 페이지를 떠날까요? 입력한 내용이 저장되지 않을 수 있어요.";

/**
 * A dialog the remote page shows (`alert`, `confirm`, `prompt`, the ask before leaving it), which the frames do not
 * draw: its text and its choices over the screen, as a browser would show them. The answer goes to the page through
 * the server; the card stays until the server says the dialog closed, and its buttons wait meanwhile.
 *
 * Escape answers as the cancel button does (an `alert` has only `확인`, which Escape also is).
 */
export function PageDialogCard({
  dialog,
  maxText,
  onAnswer,
}: {
  dialog: PageDialog;
  /** The most characters of a prompt's answer the server takes. */
  maxText: number;
  onAnswer: (accept: boolean, text?: string) => void;
}) {
  const titleId = useId();
  const textId = useId();
  const [text, setText] = useState(dialog.prompt);
  const [answered, setAnswered] = useState(false);
  const first = useRef<HTMLElement | null>(null);

  // Another dialog is a new question; the focus goes to it, away from the remote page's keyboard field.
  useEffect(() => {
    setText(dialog.prompt);
    setAnswered(false);
    first.current?.focus();
  }, [dialog.id, dialog.prompt]);

  const answer = (accept: boolean) => {
    if (answered) return;
    setAnswered(true);
    onAnswer(accept, accept && dialog.kind === "prompt" ? text : undefined);
  };
  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key === "Escape") {
      e.preventDefault();
      answer(dialog.kind === "alert");
    }
  };

  const title = dialog.host === null ? "이 페이지의 메시지" : `${dialog.host}의 메시지`;
  const leaving = dialog.kind === "beforeunload";
  const [cancel, accept] = leaving ? ["머무르기", "떠나기"] : ["취소", "확인"];

  return (
    <div className="absolute inset-0 flex items-center justify-center bg-surface-1/60 p-3">
      <div
        role="alertdialog"
        aria-modal="true"
        aria-labelledby={titleId}
        aria-describedby={textId}
        className="flex max-h-full w-full max-w-[400px] flex-col gap-3 rounded-card border border-hairline bg-surface-1 p-4 shadow-(--card-shadow)"
        onKeyDown={onKeyDown}
      >
        <h3 id={titleId} className="truncate text-[15px] font-bold" title={title}>
          {title}
        </h3>
        <p
          id={textId}
          className={cn(
            "min-h-0 overflow-y-auto text-[14px] leading-relaxed break-words whitespace-pre-wrap text-text-primary",
            "max-h-[40vh]",
          )}
        >
          {leaving ? LEAVE_TEXT : dialog.message}
        </p>
        {dialog.kind === "prompt" && (
          <Input
            ref={(node) => {
              first.current = node;
            }}
            aria-label="답"
            value={text}
            maxLength={maxText}
            disabled={answered}
            onChange={(e) => setText(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.nativeEvent.isComposing) {
                e.preventDefault();
                answer(true);
              }
            }}
          />
        )}
        <div className="flex flex-wrap justify-end gap-2">
          {dialog.kind !== "alert" && (
            <Button type="button" variant="ghost" className={btnNeutral} disabled={answered} onClick={() => answer(false)}>
              {cancel}
            </Button>
          )}
          <Button
            ref={(node) => {
              if (dialog.kind !== "prompt") first.current = node;
            }}
            type="button"
            variant="ghost"
            className={btnAction}
            disabled={answered}
            onClick={() => answer(true)}
          >
            {accept}
          </Button>
        </div>
      </div>
    </div>
  );
}
