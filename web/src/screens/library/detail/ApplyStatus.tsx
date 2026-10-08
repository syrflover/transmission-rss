import { useEffect, useRef, useState } from "react";
import { Link } from "react-router-dom";

import { POLL_MS } from "../../collect/history/useRetry";
import { fetchJob, jobPath } from "../../todo/api";
import { loadWork } from "../api";
import { applyProgress, appliedShown, waitOver } from "./subtitles.ts";

/**
 * What the status says once the wait stopped: the job ended and the work shows it, the job stops for a person, or the
 * wait ran out while the job was on its way (`over`) or while the work did not show the finished apply yet (`unseen`).
 */
type Stopped = { kind: "done" } | { kind: "waiting"; text: string } | { kind: "over" } | { kind: "unseen" };

const OVER_TEXT = "아직 끝나지 않았어요.";
const SHOWING_TEXT = "적용했어요. 라이브러리에 보이기를 기다려요.";
const UNSEEN_TEXT = "적용했어요. 라이브러리에는 아직 보이지 않아요.";

/**
 * The line a stored copy shows after its apply was handed to the job (`적용을 맡겼어요`). The job's answer only says
 * it took the apply, so this reads the job (`GET /api/subtitle-jobs/{id}`, as the job page does) every
 * `POLL_MS` until it is no longer on its way, the way the app's other waits for the worker do (`web-app.md`,
 * 웹 명령과 상태 갱신):
 *
 * - the next read is scheduled when the previous one has answered, nothing is read while the page is hidden, and it is
 *   read at once when the page is shown again; a read that fails is tried again;
 * - when the job ends `done`, the work is read the same way until it shows the apply (`appliedShown`): the job's
 *   record has the copy applied at once, but the episodes list the file only once the worker's reading of the watch
 *   folder found it, a few seconds later. Reading the work for the screen before that would take the stored copy off
 *   the row and leave the subtitle missing;
 * - then, and when the job fails (`failed`, `partial`) or stops for a person (`waiting`, `held`), `onEnded` reads the
 *   work again, so the row shows what the worker did. A failure then goes to `onFailed` with the job's own sentence;
 * - the wait stops when the row goes away, when the job ends and the work shows it, and after `APPLY_WAIT_MS` at the
 *   latest (a worker that is down leaves the job queued, a watch folder that cannot be read leaves the file unlisted);
 *   the line then keeps the link to the job.
 */
export function ApplyStatus({
  job,
  workId,
  storedId,
  onEnded,
  onFailed,
}: {
  job: string;
  /** The work and the stored copy whose apply the job was asked for. */
  workId: string;
  storedId: string;
  /** Reads the work again. */
  onEnded: () => Promise<void>;
  /** The job failed: `text` is why, and the row shows it. */
  onFailed: (text: string) => void;
}) {
  const [stopped, setStopped] = useState<Stopped | null>(null);
  // The job ended `done`; the wait is for the work to show it.
  const [applied, setApplied] = useState(false);
  const endedRef = useRef(onEnded);
  endedRef.current = onEnded;
  const failedRef = useRef(onFailed);
  failedRef.current = onFailed;

  useEffect(() => {
    const startedAt = Date.now();
    let gone = false;
    let finished = false;
    let pending = false;
    let jobDone = false;
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
        setStopped({ kind: jobDone ? "unseen" : "over" });
        return;
      }
      pending = true;
      const mine = new AbortController();
      controller = mine;
      const read = jobDone
        ? loadWork(workId, mine.signal).then((work) => {
            if (gone || finished) return;
            if (work === null || appliedShown(work, storedId)) void finish({ kind: "done" });
          })
        : fetchJob(job, mine.signal).then((detail) => {
            if (gone || finished) return;
            const progress = applyProgress(detail);
            if (progress.kind === "done") {
              jobDone = true;
              setApplied(true);
            } else if (progress.kind !== "running") {
              void finish(progress);
            }
          });
      read
        .then(undefined, () => {
          // Not read this time (a dropped connection, a restart): the state is unknown, not failed.
        })
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
  }, [job, workId, storedId]);

  const text =
    stopped === null
      ? applied
        ? SHOWING_TEXT
        : "적용을 맡겼어요."
      : stopped.kind === "done"
        ? "적용이 끝났어요."
        : stopped.kind === "waiting"
          ? stopped.text
          : stopped.kind === "unseen"
            ? UNSEEN_TEXT
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
