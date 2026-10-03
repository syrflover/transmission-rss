import { useState } from "react";

import { Button } from "@/components/ui/button";
import { ApiError } from "@/lib/api";
import { patch } from "@/lib/cached";

import { btnAction } from "../collect/channels/styles";
import { finishJob, type JobDetail } from "./api";
import { KEYS } from "./poll";

const FINISH_FAILED = "받기를 끝내지 못했어요. 잠시 뒤 다시 시도해 주세요.";

/**
 * `받기 끝내기` of a find job (직접 찾기): the person is done browsing. The job
 * ends once no download of its server browser is on its way, so a file being
 * downloaded is received first; with no file kept it ends as `받은 파일 없음`.
 */
export function FindFinish({ job }: { job: JobDetail }) {
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const kept = job.items.reduce((sum, item) => sum + item.files.length, 0);

  const finish = async () => {
    setSending(true);
    setError(null);
    try {
      const { state } = await finishJob(job.id);
      // The worker ends it; the next read says how. Until then it is finishing.
      if (state === "finishing") {
        patch<JobDetail>(KEYS.job(job.id), (was) => ({ ...was, finishing: true }));
      }
    } catch (e) {
      setError(e instanceof ApiError ? e.message : FINISH_FAILED);
    } finally {
      setSending(false);
    }
  };

  return (
    <div className="mt-3 flex flex-wrap items-center gap-x-3 gap-y-2">
      <Button
        type="button"
        variant="ghost"
        className={btnAction}
        disabled={sending || job.finishing}
        onClick={() => void finish()}
      >
        {job.finishing ? "끝내는 중…" : "받기 끝내기"}
      </Button>
      <p className="m-0 min-w-0 flex-1 text-[13px] leading-relaxed text-text-secondary">
        {job.finishing
          ? "받는 중인 파일이 있으면 그 파일까지 받고 끝내요."
          : kept > 0
            ? `지금까지 받은 파일 ${kept}개로 작업을 끝내요. 받는 중인 파일이 있으면 그 파일까지 받아요.`
            : "아직 받은 파일이 없어요. 지금 끝내면 받은 파일 없음으로 끝나요."}
      </p>
      {error !== null && (
        <p role="alert" className="w-full text-[13px] font-semibold text-urgent">
          {error}
        </p>
      )}
    </div>
  );
}
