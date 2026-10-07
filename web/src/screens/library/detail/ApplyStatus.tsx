import { useEffect, useRef, useState } from "react";
import { Link } from "react-router-dom";

import { POLL_MS } from "../../collect/history/useRetry";
import { fetchJob, jobPath } from "../../todo/api";
import { applyProgress, waitOver } from "./subtitles.ts";

/** What the status says once the wait stopped without the job having ended. */
type Stopped = { kind: "done" } | { kind: "waiting"; text: string } | { kind: "over" };

const OVER_TEXT = "아직 끝나지 않았어요.";

/**
 * The line a stored copy shows after its apply was handed to the job (`적용을 맡겼어요`). The job's answer only says
 * it took the apply, so this reads the job (`GET /api/subtitle-jobs/{id}`, as the job page does) every
 * `POLL_MS` until it is no longer on its way, the way the app's other waits for the worker do (`web-app.md`,
 * 웹 명령과 상태 갱신):
 *
 * - the next read is scheduled when the previous one has answered, nothing is read while the page is hidden, and it is
 *   read at once when the page is shown again; a read that fails is tried again;
 * - when the job ends (`done`, `failed`, `partial`) or stops for a person (`waiting`, `held`), `onEnded` reads the work
 *   again, so the row shows what the worker did. A failure then goes to `onFailed` with the job's own sentence;
 * - the wait stops when the row goes away, when the job ends, and after `APPLY_WAIT_MS` at the latest (a worker
 *   that is down leaves the job queued); the line then keeps the link to the job.
 */
export function ApplyStatus({
  job,
  onEnded,
  onFailed,
}: {
  job: string;
  /** Reads the work again. */
  onEnded: () => Promise<void>;
  /** The job failed: `text` is why, and the row shows it. */
  onFailed: (text: string) => void;
}) {
  const [stopped, setStopped] = useState<Stopped | null>(null);
  const endedRef = useRef(onEnded);
  endedRef.current = onEnded;
  const failedRef = useRef(onFailed);
  failedRef.current = onFailed;

  useEffect(() => {
    const startedAt = Date.now();
    let gone = false;
    let finished = false;
    let pending = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let controller: AbortController | undefined;

    const schedule = () => {
      clearTimeout(timer);
      timer = undefined;
      if (gone || finished || document.hidden) return;
      timer = setTimeout(run, POLL_MS);
    };

    const finish = async (then: Stopped | { kind: "failed"; text: string }) => {
      finished = true;
      clearTimeout(timer);
      try {
        await endedRef.current();
      } catch {
        // The row keeps its old look; the line below still says what the job did.
      }
      if (gone) return;
      if (then.kind === "failed") failedRef.current(then.text);
      else setStopped(then);
    };

    function run() {
      if (gone || finished || pending || document.hidden) return;
      clearTimeout(timer);
      timer = undefined;
      if (waitOver(startedAt, Date.now())) {
        finished = true;
        setStopped({ kind: "over" });
        return;
      }
      pending = true;
      const mine = new AbortController();
      controller = mine;
      fetchJob(job, mine.signal)
        .then(
          (detail) => {
            if (gone || finished) return;
            const progress = applyProgress(detail);
            if (progress.kind !== "running") void finish(progress);
          },
          () => {
            // Not read this time (a dropped connection, a restart): the job's state is unknown, not failed.
          },
        )
        .finally(() => {
          pending = false;
          schedule();
        });
    }

    const onVisibility = () => {
      if (document.hidden) {
        clearTimeout(timer);
        timer = undefined;
      } else {
        run();
      }
    };

    document.addEventListener("visibilitychange", onVisibility);
    run();
    return () => {
      gone = true;
      clearTimeout(timer);
      controller?.abort();
      document.removeEventListener("visibilitychange", onVisibility);
    };
  }, [job]);

  const text =
    stopped === null
      ? "적용을 맡겼어요."
      : stopped.kind === "done"
        ? "적용이 끝났어요."
        : stopped.kind === "waiting"
          ? stopped.text
          : OVER_TEXT;
  return (
    <span role="status" className="text-xs text-text-secondary">
      {text}{" "}
      <JobLink job={job} />
    </span>
  );
}

/** `작업 보기`: the job's page. */
export function JobLink({ job }: { job: string }) {
  return (
    <Link to={jobPath(job)} className="rounded-sm underline underline-offset-2 focus-visible:outline-2 focus-visible:outline-focus">
      작업 보기
    </Link>
  );
}
