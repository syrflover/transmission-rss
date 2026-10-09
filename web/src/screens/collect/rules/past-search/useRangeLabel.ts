import { useEffect, useRef, useState } from "react";

import { ApiError } from "@/lib/api";

import { rangeLabelOf } from "./api";

/** How long after the last keystroke the typed range is sent. */
const DEBOUNCE_MS = 300;

export interface RangeLabelState {
  /** The latest label: for the range as it is when `fresh`, for an earlier one while the next is asked. */
  label: string | null;
  /** The label is for the range as it is now. */
  fresh: boolean;
  /** Why the range could not be labelled, as a sentence. */
  failure: string | null;
}

/**
 * The server's label for the range of releases being typed (`S02E01–12`, `1–12화`), asked for when there is a
 * range to label. The first is asked at once and later ones a moment after the last keystroke; an answer for
 * an older range is dropped. The screen only shows it: the folder's episodes are the rule's conversion applied
 * by the server.
 */
export function useRangeLabel(ruleId: string, from: number | null, to: number | null): RangeLabelState {
  const [answer, setAnswer] = useState<{ key: string; label: string } | null>(null);
  const [failed, setFailed] = useState<{ key: string; message: string } | null>(null);
  const asked = useRef(false);
  const key = from === null || to === null ? null : `${ruleId}:${from}:${to}`;

  useEffect(() => {
    if (key === null || from === null || to === null) return;
    const controller = new AbortController();
    const delay = asked.current ? DEBOUNCE_MS : 0;
    asked.current = true;
    const timer = window.setTimeout(() => {
      rangeLabelOf(ruleId, from, to, controller.signal).then(
        ({ label }) => {
          if (controller.signal.aborted) return;
          setAnswer({ key, label });
          setFailed(null);
        },
        (e: unknown) => {
          if (controller.signal.aborted) return;
          setFailed({ key, message: e instanceof ApiError ? e.message : "회차 이름을 불러오지 못했어요." });
        },
      );
    }, delay);
    return () => {
      window.clearTimeout(timer);
      controller.abort();
    };
  }, [key]);

  return {
    label: answer?.label ?? null,
    fresh: key !== null && answer?.key === key,
    failure: key !== null && failed?.key === key ? failed.message : null,
  };
}
