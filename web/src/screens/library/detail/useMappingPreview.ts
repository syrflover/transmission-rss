import { useEffect, useRef, useState } from "react";

import { ApiError } from "@/lib/api";

import { previewMapping, type MappingInput, type MappingPreview } from "../api";

/** How long after the last change the input is sent again. */
const DEBOUNCE_MS = 300;

export interface MappingPreviewState {
  /**
   * The latest answer: for the input shown when `fresh`, for an earlier input while the next one is asked
   * (`null` until the first comes).
   */
  preview: MappingPreview | null;
  /** The answer is for the input as it is now, so what it says can be saved. */
  fresh: boolean;
  /** Why the input could not be previewed, as a sentence; `null` when it could. */
  failure: string | null;
}

/**
 * What `회차 대응 정하기` shows for its input, written by the server (`trss_library::mapping::preview`). The first
 * answer is asked for at once; after a change it is asked again a moment after the last one, an answer for an
 * older input is dropped, and the last answer stays on view until the next comes. Nothing here reads an
 * episode or an offset: the server does, with the rules the save and the receipt use.
 */
export function useMappingPreview(
  workId: string,
  season: number,
  sourceId: string,
  input: MappingInput,
): MappingPreviewState {
  const [answer, setAnswer] = useState<{ key: string; preview: MappingPreview } | null>(null);
  const [failed, setFailed] = useState<{ key: string; message: string } | null>(null);
  const asked = useRef(false);
  // The input as one string, so the effect depends on its content.
  const key = JSON.stringify([workId, season, sourceId, input]);

  useEffect(() => {
    const controller = new AbortController();
    const delay = asked.current ? DEBOUNCE_MS : 0;
    asked.current = true;
    const timer = window.setTimeout(() => {
      previewMapping(workId, season, sourceId, input, controller.signal).then(
        (preview) => {
          if (controller.signal.aborted) return;
          setAnswer({ key, preview });
          setFailed(null);
        },
        (e: unknown) => {
          if (controller.signal.aborted) return;
          setFailed({ key, message: e instanceof ApiError ? e.message : "미리보기를 불러오지 못했어요." });
        },
      );
    }, delay);
    return () => {
      window.clearTimeout(timer);
      controller.abort();
    };
    // `key` carries every input; `input` is a new object on each render.
  }, [key]);

  return {
    preview: answer?.preview ?? null,
    fresh: answer?.key === key,
    failure: failed?.key === key ? failed.message : null,
  };
}
