import { useId, type DragEvent } from "react";

import { cn } from "@/lib/utils";

import { AlertIcon, UploadIcon } from "../icons";
import { Banner, BTN, CARD } from "../parts";
import type { ImportFlow } from "./useImportFlow";

export function PickStep({ flow }: { flow: ImportFlow }) {
  const inputId = useId();

  const drop = (event: DragEvent<HTMLLabelElement>) => {
    event.preventDefault();
    const file = event.dataTransfer.files[0];
    if (file) void flow.chooseFile(file);
  };

  return (
    <div className="flex flex-col gap-4">
      <label
        htmlFor={inputId}
        onDragOver={(event) => event.preventDefault()}
        onDrop={drop}
        className={cn(
          CARD,
          "flex cursor-pointer flex-col items-center gap-3 border-dashed px-4 py-9 text-center has-[:focus-visible]:outline-2 has-[:focus-visible]:outline-offset-3 has-[:focus-visible]:outline-focus",
          flow.reading && "opacity-70",
        )}
      >
        <UploadIcon className="size-7 text-text-muted" />
        <span className="text-sm font-bold">기존 YAML 파일</span>
        <span className="max-w-[46ch] text-[13.5px] leading-relaxed text-text-secondary">
          transmission-rss에서 쓰던 채널 설정 파일을 선택해요. 파일은 이 브라우저에서 서버로만 보내고, 선택한 파일 자체는 바뀌지 않아요.
        </span>
        <span className={BTN.action} aria-hidden="true">
          {flow.reading ? "확인하는 중" : "파일 선택"}
        </span>
        <input
          id={inputId}
          type="file"
          accept=".yaml,.yml,text/yaml,application/yaml,text/plain"
          className="sr-only"
          disabled={flow.reading}
          aria-label="기존 YAML 파일 선택"
          onChange={(event) => {
            const file = event.target.files?.[0];
            if (file) void flow.chooseFile(file);
            // Choosing the same file again after a failure must fire again.
            event.target.value = "";
          }}
        />
      </label>

      {flow.pickError && (
        <Banner tone="fail" role="alert" title="가져올 수 없는 파일이에요">
          {flow.pickError} 설정은 하나도 바뀌지 않았어요. 다른 파일을 선택해 주세요.
        </Banner>
      )}

      <p className="flex items-start gap-2 text-[13px] leading-relaxed text-text-muted">
        <AlertIcon className="mt-0.5 size-4 flex-none" />
        <span>
          앱 YAML 가져오기는 아직 열리지 않았어요. 이 단계에서는 기존 YAML만 읽을 수 있어요.
        </span>
      </p>
    </div>
  );
}
