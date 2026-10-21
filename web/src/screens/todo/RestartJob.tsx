import { useState } from "react";

import { Button } from "@/components/ui/button";
import { ApiError } from "@/lib/api";

import { btnAction } from "../collect/channels/styles";
import { restartJob } from "./api";

const RESTART_FAILED = "다시 시작하지 못했어요. 잠시 뒤 다시 시도해 주세요.";

/**
 * `다시 시작` of an `interrupted` job (`중단됨`): the job goes on from the step the process cut it in. `reload` reads
 * what the screen shows again; it runs after the request answers, also for a `conflict` (the job is not interrupted
 * any more), so the screen shows what the server has now.
 */
export function RestartJob({ jobId, reload }: { jobId: string; reload: () => Promise<void> }) {
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const restart = async () => {
    setSending(true);
    setError(null);
    try {
      await restartJob(jobId);
    } catch (e) {
      setError(e instanceof ApiError ? e.message : RESTART_FAILED);
      if (!(e instanceof ApiError && e.code === "conflict")) {
        setSending(false);
        return;
      }
    }
    try {
      await reload();
    } catch {
      // The job's change event reads it again.
    }
    setSending(false);
  };

  return (
    <>
      <Button type="button" variant="ghost" className={btnAction} disabled={sending} onClick={() => void restart()}>
        다시 시작
      </Button>
      {error !== null && (
        <p role="alert" className="w-full text-[13px] font-semibold text-urgent">
          {error}
        </p>
      )}
    </>
  );
}
