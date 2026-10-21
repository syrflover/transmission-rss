import { api } from "@/lib/api";
import type { Board } from "@/lib/api-types";

export function loadBoard(signal?: AbortSignal): Promise<Board> {
  // Minutes east of UTC decide where a day starts for the bars.
  const offset = -new Date().getTimezoneOffset();
  return api<Board>(`/collect/status?tz_offset=${offset}`, { signal });
}

/** What the status says in place of a past next-check time when the checks have stopped. */
export const STALLED = "RSS 확인이 멈췄어요. 컨테이너 로그를 확인해 주세요.";
/** The same on a phone's one-line summary. */
export const STALLED_SHORT = "RSS 멈춤";
