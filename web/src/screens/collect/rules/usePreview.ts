import { useEffect, useState } from "react";

import { ApiError } from "@/lib/api";

import { previewRule, type Preview, type RuleFields } from "./api";

export type PreviewState =
  | { state: "loading"; previous: Preview | null }
  | { state: "ready"; preview: Preview }
  | { state: "failed"; message: string };

const DEBOUNCE_MS = 300;

/**
 * The server's preview of the rule as edited. It is asked again a moment after
 * the last change; an answer for an older edit is dropped. Nothing here judges
 * a title: the server runs the worker's own evaluation.
 */
export function usePreview(
  channelId: string,
  ruleId: string | null,
  fields: RuleFields,
  position: number,
): PreviewState {
  const [result, setResult] = useState<PreviewState>({ state: "loading", previous: null });
  // The edited fields as one string, so the effect depends on their content.
  const key = JSON.stringify([channelId, ruleId, fields, position]);

  useEffect(() => {
    const controller = new AbortController();
    setResult((prev) => ({
      state: "loading",
      previous: prev.state === "ready" ? prev.preview : prev.state === "loading" ? prev.previous : null,
    }));
    const timer = window.setTimeout(() => {
      previewRule(channelId, ruleId, fields, position, controller.signal).then(
        (preview) => {
          if (!controller.signal.aborted) setResult({ state: "ready", preview });
        },
        (e: unknown) => {
          if (controller.signal.aborted) return;
          setResult({
            state: "failed",
            message: e instanceof ApiError ? e.message : "미리보기를 불러오지 못했어요.",
          });
        },
      );
    }, DEBOUNCE_MS);
    return () => {
      window.clearTimeout(timer);
      controller.abort();
    };
    // `key` carries every input; `fields` is a new object on each render.
  }, [key]);

  return result;
}
