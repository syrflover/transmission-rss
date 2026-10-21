import { useEffect, useRef, useState } from "react";
import { Link } from "react-router-dom";

import { createRereader } from "@/lib/rereads";
import { sharedEvents } from "@/lib/sharedEvents";

import { fetchJob, jobPath } from "../../todo/api";
import { loadWork } from "../api";
import { applyChanged } from "./listens";
import { APPLY_WAIT_MS, applyProgress, appliedShown, waitOver } from "./subtitles.ts";

/**
 * What the status says once the wait stopped: the job ended and the work shows it, the job stops for a person, or the
 * wait ran out while the job was on its way (`over`) or while the work did not show the finished apply yet (`unseen`).
 */
type Stopped = { kind: "done" } | { kind: "waiting"; text: string } | { kind: "over" } | { kind: "unseen" };

/** How long to wait before reading the job again after a read that failed, in milliseconds. */
const RETRY_MS = 2000;

const OVER_TEXT = "아직 끝나지 않았어요.";
const SHOWING_TEXT = "적용했어요. 라이브러리에 보이기를 기다려요.";
const UNSEEN_TEXT = "적용했어요. 라이브러리에는 아직 보이지 않아요.";

/**
 * The line a stored copy shows after its apply was handed to the job (`적용을 맡겼어요`). The job's answer only says
 * it took the apply, so this reads the job (`GET /api/subtitle-jobs/{id}`, as the job page does) when the server says
 * the job changed, until it is no longer on its way, the way the app's other waits for trss do (`web-app.md`,
 * 웹 명령과 상태 갱신):
 *
 * - changes that come while a read is under way make one more read after it, nothing is read while the page is
 *   hidden, and everything is read when it is shown again; a read that fails is tried again after `RETRY_MS`;
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
    let jobDone = false;
    let timer: ReturnType<typeof setTimeout> | undefined;

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

    // The wait ran out while the job was on its way, or while the work did not show the finished apply yet.
    const expire = () => {
      finished = true;
      clearTimeout(timer);
      rereader.stop();
      setStopped({ kind: jobDone ? "unseen" : "over" });
    };

    const rereader = createRereader({
      read: async (signal) => {
        if (gone || finished) return;
        if (waitOver(startedAt, Date.now())) {
          expire();
          return;
        }
        if (!jobDone) {
          const detail = await fetchJob(job, signal);
          if (gone || finished) return;
          const progress = applyProgress(detail);
          if (progress.kind === "running") return;
          if (progress.kind !== "done") {
            void finish(progress);
            return;
          }
          jobDone = true;
          setApplied(true);
        }
        const work = await loadWork(workId, signal);
        if (gone || finished) return;
        if (work === null || appliedShown(work, storedId)) void finish({ kind: "done" });
      },
      // Not read this time (a dropped connection, a restart): the state is unknown, not failed.
      retryMs: () => RETRY_MS,
      hidden: () => document.hidden,
    });

    // Listening starts before the first read, so a change during it is heard.
    const stopListening = sharedEvents().subscribe({
      match: (change) => applyChanged(change, job, workId),
      reread: () => rereader.request(),
    });
    // The wait is a clock of the page, not a read of the server: it ends when `APPLY_WAIT_MS` has passed whatever is heard.
    timer = setTimeout(() => {
      if (!gone && !finished) expire();
    }, APPLY_WAIT_MS);
    rereader.start();
    return () => {
      gone = true;
      clearTimeout(timer);
      stopListening();
      rereader.stop();
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
