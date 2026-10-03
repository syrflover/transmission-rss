import { useId, useRef, useState } from "react";
import { useNavigate } from "react-router-dom";

import { Button } from "@/components/ui/button";
import { ApiError } from "@/lib/api";
import { newCommandId } from "@/lib/commands";
import { cn } from "@/lib/utils";

import { btnAction, hintClass, inputClass } from "../../collect/channels/styles";
import { createFindJob, jobPath } from "../../todo/api";
import type { CandidateGroup } from "./candidates";

/** The one request the user has not seen the answer to: its ID is kept so a resend makes no second job. */
interface Sent {
  id: string;
  creator: string;
}

/**
 * `직접 찾기`, at the end of the season's 자막 후보: the user picks one of the creators the candidates name, and a
 * job opens that creator's most recently observed post in the server browser. The job's page shows the remote
 * screen, where the user browses the creator's past posts and downloads attachments; every download becomes a file
 * of the job. Shown only while the season has candidates (so an Anissia link and a creator).
 */
export function FindSection({
  workId,
  season,
  groups,
}: {
  workId: string;
  season: number;
  groups: readonly Pick<CandidateGroup, "sourceId" | "creator" | "host">[];
}) {
  const navigate = useNavigate();
  const selectId = useId();
  const [creator, setCreator] = useState(groups[0]?.sourceId ?? "");
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const sent = useRef<Sent | null>(null);

  // A creator the list no longer has (the candidates were read again) is not kept as a choice.
  const chosen = groups.some((g) => g.sourceId === creator) ? creator : (groups[0]?.sourceId ?? "");
  const host = groups.find((g) => g.sourceId === chosen)?.host ?? null;

  const find = async () => {
    if (sending || chosen === "") return;
    const kept = sent.current;
    const request = kept && kept.creator === chosen ? kept : (sent.current = { id: newCommandId(), creator: chosen });
    setSending(true);
    setError(null);
    try {
      const made = await createFindJob({ id: request.id, work_id: workId, season, creator: chosen });
      sent.current = null;
      navigate(jobPath(made.id));
    } catch (e) {
      // The server answered and said no: nothing was made, and a next try is a new request.
      if (e instanceof ApiError && (e.code === "invalid" || e.code === "conflict" || e.code === "not_found")) {
        sent.current = null;
      }
      setError(e instanceof ApiError ? e.message : "직접 찾기 작업을 만들지 못했어요.");
      setSending(false);
    }
  };

  return (
    <div className="mt-4 border-t border-hairline-soft pt-3" data-testid="find-section">
      <h3 className="m-0 text-[14.5px] font-bold">직접 찾기</h3>
      <p className={cn(hintClass, "m-0 mt-1 text-[12.5px]")}>
        후보에 없는 지난 회차는 제작자의 게시물을 서버 브라우저로 직접 찾아 받아요. 제작자의 가장 최근 게시물이 작업 화면에
        열리고, 거기서 받은 파일은 모두 한 작업에 담겨요. 자막, 폰트, 압축 파일만 남겨요.
      </p>
      <div className="mt-2 flex flex-wrap items-center gap-x-3 gap-y-2">
        <label htmlFor={selectId} className="text-[12.5px] font-semibold text-text-secondary">
          제작자
        </label>
        <select
          id={selectId}
          value={chosen}
          disabled={sending}
          onChange={(e) => setCreator(e.target.value)}
          className={cn(inputClass, "w-[240px] max-w-full border py-1")}
        >
          {groups.map((g) => (
            <option key={g.sourceId} value={g.sourceId}>
              {g.creator}
            </option>
          ))}
        </select>
        {host !== null && <span className="text-xs text-text-muted [overflow-wrap:anywhere]">{host}</span>}
        <Button type="button" variant="ghost" className={btnAction} disabled={sending || chosen === ""} onClick={() => void find()}>
          {sending ? "만드는 중…" : "직접 찾기"}
        </Button>
      </div>
      {error !== null && (
        <p role="alert" className="m-0 mt-2 text-[13px] font-semibold text-urgent">
          {error}
        </p>
      )}
    </div>
  );
}
