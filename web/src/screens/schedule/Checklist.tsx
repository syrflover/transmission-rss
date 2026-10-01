import { useState } from "react";
import { Link } from "react-router-dom";

import { ApiError } from "@/lib/api";
import { cn } from "@/lib/utils";

import { CheckIcon } from "../library/icons";
import { BTN, CARD } from "../settings/parts";
import { setSkipped, type FirstRun, type Step } from "./api";

interface StepInfo {
  title: string;
  /** What the step does, and what it leads to. */
  text: string;
  /** The settings page the step's button opens. */
  to: string;
  go: string;
}

const STEPS: Record<Step, StepInfo> = {
  folder: {
    title: "감시 폴더 등록",
    text: "영상이 들어 있는 폴더를 등록하면 라이브러리가 작품을 찾아요. 폴더를 하나 추가하면 이 단계가 끝나요.",
    to: "/settings/folders",
    go: "감시 폴더 등록",
  },
  import: {
    title: "기존 설정 가져오기",
    text: "이전에 쓰던 채널 YAML이 있으면 모두 추가해요. YAML이 없으면 건너뛰고, 나중에 수집 화면에서 구독을 추가하면 돼요.",
    to: "/settings/import",
    go: "가져오기",
  },
};

/**
 * The first run's checklist, in the place of the weekly schedule. Each step
 * leads to its settings page and can be skipped; a skip can be taken back.
 * `onChange` gets the checklist as the server has it after a skip or its undo.
 */
export function Checklist({
  firstRun,
  onChange,
}: {
  firstRun: FirstRun;
  onChange: (next: FirstRun, step: Step, skipped: boolean) => void;
}) {
  const [busy, setBusy] = useState<Step | null>(null);
  const [error, setError] = useState<string | null>(null);

  const skip = async (step: Step, skipped: boolean) => {
    setBusy(step);
    setError(null);
    try {
      onChange(await setSkipped(step, skipped), step, skipped);
    } catch (e) {
      setError(e instanceof ApiError ? e.message : "건너뛰기를 저장하지 못했어요.");
    } finally {
      setBusy(null);
    }
  };

  return (
    <section aria-labelledby="first-run-title" data-testid="first-run" className="max-w-[720px]">
      <h2 id="first-run-title" className="text-lg font-bold">
        처음 설정
      </h2>
      <p className="mt-1 mb-4 text-[13.5px] leading-relaxed text-text-secondary">
        두 단계를 마치거나 건너뛰면 이번 주 편성이 나와요. 건너뛴 단계는 되돌릴 수 있어요.
      </p>
      <ol className="m-0 flex list-none flex-col gap-3 p-0">
        {firstRun.steps.map(({ step, done, skipped }, i) => {
          const info = STEPS[step];
          const idle = !done && !skipped;
          return (
            <li key={step} className={cn(CARD, "flex min-w-0 flex-wrap items-start gap-x-3.5 gap-y-3 p-4")}>
              <span
                aria-hidden="true"
                className={cn(
                  "mt-0.5 flex size-6 flex-none items-center justify-center rounded-full border text-xs font-bold",
                  done ? "border-ok text-ok" : "border-hairline text-text-muted",
                )}
              >
                {done ? <CheckIcon className="size-3.5" /> : i + 1}
              </span>
              <div className="min-w-0 flex-1 basis-60">
                <h3 className="flex flex-wrap items-center gap-x-2 text-[15px] font-bold">
                  {info.title}
                  {done && <span className="text-xs font-semibold text-ok">완료</span>}
                  {skipped && !done && <span className="text-xs font-medium text-text-muted">건너뜀</span>}
                </h3>
                <p className="mt-1 text-[13px] leading-relaxed text-text-secondary">{info.text}</p>
              </div>
              {!done && (
                <div className="flex flex-none flex-wrap gap-2">
                  {idle && (
                    <>
                      <Link to={info.to} className={BTN.main}>
                        {info.go}
                      </Link>
                      <button type="button" className={BTN.plain} disabled={busy !== null} onClick={() => skip(step, true)}>
                        건너뛰기
                      </button>
                    </>
                  )}
                  {skipped && (
                    <button type="button" className={BTN.plain} disabled={busy !== null} onClick={() => skip(step, false)}>
                      건너뛰기 취소
                    </button>
                  )}
                </div>
              )}
            </li>
          );
        })}
      </ol>
      {error && (
        <p role="alert" className="mt-3 text-[13px] font-semibold text-urgent">
          {error}
        </p>
      )}
    </section>
  );
}
