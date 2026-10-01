import { useEffect, useRef, useState } from "react";

import { ApiError } from "@/lib/api";
import { peek, store } from "@/lib/cached";

import { previewRule, type Preview, type RuleFields } from "./api";

export type PreviewState =
  | { state: "loading"; previous: Preview | null }
  | { state: "ready"; preview: Preview }
  | { state: "failed"; message: string };

const DEBOUNCE_MS = 300;

/** The last answer for a stored rule, whatever was edited, to show while the next one is asked. */
const lastKey = (ruleId: string) => `collect:preview:${ruleId}`;

/**
 * The server's preview of the rule as edited. The first answer is asked for at
 * once, and shows the rule's last answer (dimmed) until it comes, so picking a
 * rule again does not start from an empty panel. After an edit it is asked
 * again a moment after the last change; an answer for an older edit is dropped.
 * Nothing here judges a title: the server runs the worker's own evaluation.
 */
export function usePreview(
  channelId: string,
  ruleId: string | null,
  fields: RuleFields,
  position: number,
  /** Changes when the recorded items did (an item was received), to ask again. */
  refresh = 0,
): PreviewState {
  const [result, setResult] = useState<PreviewState>(() => ({
    state: "loading",
    previous: ruleId ? (peek<Preview>(lastKey(ruleId)) ?? null) : null,
  }));
  const asked = useRef(false);
  // The edited fields as one string, so the effect depends on their content.
  const key = JSON.stringify([channelId, ruleId, fields, position, refresh]);

  useEffect(() => {
    const controller = new AbortController();
    setResult((prev) => ({
      state: "loading",
      previous: prev.state === "ready" ? prev.preview : prev.state === "loading" ? prev.previous : null,
    }));
    const delay = asked.current ? DEBOUNCE_MS : 0;
    asked.current = true;
    const timer = window.setTimeout(() => {
      previewRule(channelId, ruleId, fields, position, controller.signal).then(
        (preview) => {
          if (controller.signal.aborted) return;
          if (ruleId) store(lastKey(ruleId), preview);
          setResult({ state: "ready", preview });
        },
        (e: unknown) => {
          if (controller.signal.aborted) return;
          setResult({
            state: "failed",
            message: e instanceof ApiError ? e.message : "미리보기를 불러오지 못했어요.",
          });
        },
      );
    }, delay);
    return () => {
      window.clearTimeout(timer);
      controller.abort();
    };
    // `key` carries every input; `fields` is a new object on each render.
  }, [key]);

  return result;
}
