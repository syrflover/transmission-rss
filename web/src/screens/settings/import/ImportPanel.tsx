import { useEffect, useRef } from "react";

import { Steps } from "../parts";
import { PickStep } from "./PickStep";
import { ResultStep } from "./ResultStep";
import { ReviewStep } from "./ReviewStep";
import type { ImportFlow } from "./useImportFlow";

const STEP_NUMBER = { pick: 1, review: 2, result: 3 } as const;

/** `가져오기`: `파일 선택` → `검토` → `결과`. */
export function ImportPanel({ flow }: { flow: ImportFlow }) {
  const title = useRef<HTMLHeadingElement>(null);
  const firstRender = useRef(true);

  // A new step starts at its top with the panel title focused, as a dialog would.
  useEffect(() => {
    if (firstRender.current) {
      firstRender.current = false;
      return;
    }
    title.current?.focus({ preventScroll: true });
    window.scrollTo({ top: 0 });
  }, [flow.step]);

  return (
    <div className="flex flex-col gap-5 p-5 max-[720px]:p-4">
      <header className="flex flex-col gap-1.5">
        <h2 ref={title} id="panel-title" tabIndex={-1} className="m-0 text-xl font-bold outline-none">
          가져오기
        </h2>
        {flow.step === "pick" && (
          <p className="max-w-[62ch] text-[13.5px] leading-relaxed text-text-secondary">
            기존 YAML 파일을 선택해 채널과 규칙을 가져와요. 가져오기는 다운로드를 시작하거나 자막을 적용하거나 파일을
            정리하지 않아요.
          </p>
        )}
      </header>
      <Steps current={STEP_NUMBER[flow.step]} labels={["파일 선택", "검토", "결과"]} />
      {flow.step === "pick" && <PickStep flow={flow} />}
      {flow.step === "review" && <ReviewStep flow={flow} />}
      {flow.step === "result" && <ResultStep flow={flow} />}
    </div>
  );
}
