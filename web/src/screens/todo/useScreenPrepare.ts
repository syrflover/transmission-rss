import { useCallback, useEffect, useRef, useState } from "react";

import { ApiError } from "@/lib/api";

import { prepareScreen, type JobDetail, type JobScreen } from "./api";
import { asksOnRead, openGate } from "./prepareGate";

const PREPARE_FAILED = "서버 브라우저를 준비해 달라는 요청을 보내지 못했어요. 잠시 뒤 다시 시도해 주세요.";

export interface ScreenPrepare {
  /** The job's remote screen: the newest answer of reading the job or of the last request to prepare it. */
  screen: JobScreen | null;
  /** A request to prepare the screen is on its way. */
  opening: boolean;
  /** Why the last request failed. */
  error: string | null;
  /** Asks again; resolves when the answer is in. For the person's own `다시 열기`. */
  open: () => Promise<void>;
}

/**
 * Prepares a job's remote screen when its page is opened: one request per
 * page, on the first read of the job that comes from the server (the job
 * cached from an earlier visit may be out of date), and only for a job that
 * has a screen ({@link asksOnRead}). Polling the job and reconnecting never
 * ask again.
 */
export function useScreenPrepare(jobId: string, job: JobDetail | undefined): ScreenPrepare {
  /** Decided by the first read from the server; the first render's job is possibly cached. */
  const gate = useRef(openGate(job));
  const inFlight = useRef(false);
  const latest = useRef(job);
  latest.current = job;
  const [opening, setOpening] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [answer, setAnswer] = useState<{ basis: JobDetail | undefined; screen: JobScreen } | null>(null);

  const open = useCallback(async () => {
    if (inFlight.current) return;
    inFlight.current = true;
    setOpening(true);
    setError(null);
    try {
      const screen = await prepareScreen(jobId);
      if (screen !== null) setAnswer({ basis: latest.current, screen });
    } catch (e) {
      setError(e instanceof ApiError ? e.message : PREPARE_FAILED);
    } finally {
      inFlight.current = false;
      setOpening(false);
    }
  }, [jobId]);

  useEffect(() => {
    if (asksOnRead(gate.current, job)) void open();
  }, [job, open]);

  // The answer stands until the job is read again, which is newer.
  const screen = answer !== null && answer.basis === job ? answer.screen : (job?.screen ?? null);
  return { screen, opening, error, open };
}
