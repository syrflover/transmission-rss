import { useCallback, useState } from "react";

import { api, ApiError } from "@/lib/api";
import { everythingChanged } from "@/screens/collect/cache";

import type { ApplyResult, ChoiceRequest, Decision, Preview } from "./types";

/** The server refuses a request body over 2 MB; say so before sending. */
const MAX_FILE_BYTES = 2 * 1024 * 1024;

export type ImportStep = "pick" | "review" | "result";

export interface ImportFlow {
  step: ImportStep;
  fileName: string | null;
  preview: Preview | null;
  result: ApplyResult | null;
  /** Choice per file channel index, for channels that already exist. */
  choices: Record<number, Decision>;
  /** Reading and checking the file. */
  reading: boolean;
  applying: boolean;
  /** Why the file cannot be imported, shown on the file-selection step. */
  pickError: string | null;
  /** Why applying failed, shown on the review step. */
  applyError: { message: string; stale: boolean } | null;
  /** How many channels that already exist still need a choice. */
  undecided: number;
  chooseFile(file: File): Promise<void>;
  choose(index: number, decision: Decision): void;
  apply(): Promise<void>;
  /** Reviews the same file again after a stale review. */
  reviewAgain(): Promise<void>;
  restart(): void;
}

/**
 * State of the import: `파일 선택` → `검토` → `결과`.
 *
 * The file text is kept in memory only, and is sent again with the apply: the
 * server keeps no preview and never trusts the masked data it showed. It is
 * dropped as soon as the import is applied or the flow restarts.
 */
export function useImportFlow(): ImportFlow {
  const [step, setStep] = useState<ImportStep>("pick");
  const [content, setContent] = useState<string | null>(null);
  const [fileName, setFileName] = useState<string | null>(null);
  const [preview, setPreview] = useState<Preview | null>(null);
  const [result, setResult] = useState<ApplyResult | null>(null);
  const [choices, setChoices] = useState<Record<number, Decision>>({});
  const [reading, setReading] = useState(false);
  const [applying, setApplying] = useState(false);
  const [pickError, setPickError] = useState<string | null>(null);
  const [applyError, setApplyError] = useState<ImportFlow["applyError"]>(null);

  const undecided = preview
    ? preview.channels.filter((channel) => channel.existing && !choices[channel.index]).length
    : 0;

  const review = useCallback(async (text: string, name: string) => {
    setReading(true);
    setPickError(null);
    setApplyError(null);
    try {
      const next = await api<Preview>("/import/legacy/preview", {
        method: "POST",
        body: { content: text },
      });
      setContent(text);
      setFileName(name);
      setPreview(next);
      setChoices({});
      setStep("review");
    } catch (error) {
      setPickError(error instanceof ApiError ? error.message : "파일을 확인하지 못했어요.");
      setStep("pick");
    } finally {
      setReading(false);
    }
  }, []);

  const chooseFile = useCallback(
    async (file: File) => {
      if (file.size > MAX_FILE_BYTES) {
        setPickError("파일이 너무 커요. 2MB 이하의 기존 YAML 파일을 선택해 주세요.");
        return;
      }
      let text: string;
      try {
        text = await file.text();
      } catch {
        setPickError("파일을 읽지 못했어요. 다른 파일을 선택해 주세요.");
        return;
      }
      await review(text, file.name);
    },
    [review],
  );

  const choose = useCallback((index: number, decision: Decision) => {
    setChoices((previous) => ({ ...previous, [index]: decision }));
  }, []);

  const apply = useCallback(async () => {
    if (!preview || content === null || undecided > 0) return;
    const body: ChoiceRequest[] = preview.channels.flatMap((channel) =>
      channel.existing
        ? [
            {
              index: channel.index,
              existing_id: channel.existing.id,
              existing_version: channel.existing.version,
              decision: choices[channel.index],
            },
          ]
        : [],
    );
    setApplying(true);
    setApplyError(null);
    try {
      const done = await api<ApplyResult>("/import/legacy/apply", {
        method: "POST",
        body: { content, choices: body },
      });
      setResult(done);
      setContent(null);
      setStep("result");
    } catch (error) {
      if (error instanceof ApiError) {
        setApplyError({ message: error.message, stale: error.code === "conflict" });
      } else {
        setApplyError({ message: "가져오지 못했어요. 다시 시도해 주세요.", stale: false });
      }
    } finally {
      setApplying(false);
      // The import adds or changes channels and rules, whether or not the answer arrived.
      everythingChanged();
    }
  }, [preview, content, choices, undecided]);

  const reviewAgain = useCallback(async () => {
    if (content !== null) await review(content, fileName ?? "");
  }, [content, fileName, review]);

  const restart = useCallback(() => {
    setStep("pick");
    setContent(null);
    setFileName(null);
    setPreview(null);
    setResult(null);
    setChoices({});
    setPickError(null);
    setApplyError(null);
  }, []);

  return {
    step,
    fileName,
    preview,
    result,
    choices,
    reading,
    applying,
    pickError,
    applyError,
    undecided,
    chooseFile,
    choose,
    apply,
    reviewAgain,
    restart,
  };
}
