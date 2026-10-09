import { Button } from "@/components/ui/button";

import { btnAction } from "../../collect/channels/styles";
import type { Candidate, CandidateList } from "../api";
import { CreateStatus, JobStatusLink, PostLink, UpdatedAt } from "./CandidateParts";
import { isHeld, useCreateJob } from "./candidates";

/** What an episode list needs to name an episode's subtitle candidates and to take one. */
export interface EpisodeCandidateSource {
  list: CandidateList;
  workId: string;
  /** The subscription's creator: an episode this creator posted is received on its own and names no candidate. */
  subscribed: string | null;
  /** A job took these candidates (shows them as taken). */
  onMade: (candidates: readonly number[], jobId: string) => void;
}

/** The quiet note on an episode's row: a creator, or how many creators, have a subtitle for it. */
export function candidateNote(candidates: readonly Candidate[]): string {
  return candidates.length === 1 ? `자막 후보 · ${candidates[0].creator}` : `자막 후보 ${candidates.length}명`;
}

/** One creator's candidate for the episode with a `받기` that makes a job of this one candidate, the same as the section's. */
function EpisodePick({ candidate: c, season, source }: { candidate: Candidate; season: number; source: EpisodeCandidateSource }) {
  const { phase, create, resend } = useCreateJob(source.workId, season, source.onMade, source.list.max_job_candidates);
  return (
    <li className="flex flex-col gap-1.5">
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1.5">
        <span className="font-semibold [overflow-wrap:anywhere]">{c.creator}</span>
        <span className="text-xs text-text-muted">
          <UpdatedAt candidate={c} />
        </span>
        <span className="text-xs">
          <PostLink url={c.post_url} />
        </span>
        <JobStatusLink job={c.job} />
        <Button
          type="button"
          variant="ghost"
          className={btnAction}
          disabled={isHeld(c) || phase.kind === "sending"}
          onClick={() => create([c.id])}
        >
          받기<span className="sr-only"> ({c.creator})</span>
        </Button>
      </div>
      <CreateStatus phase={phase} onResend={resend} />
    </li>
  );
}

/** The `자막 후보` cell of an expanded episode row. */
export function EpisodePicks({ candidates, season, source }: { candidates: readonly Candidate[]; season: number; source: EpisodeCandidateSource }) {
  return (
    <div className="col-span-full min-w-0" data-testid="episode-candidates">
      <dt className="text-[12px] font-semibold text-text-muted">자막 후보</dt>
      <dd className="m-0 mt-1.5">
        <ul className="m-0 flex list-none flex-col gap-2.5 p-0 text-[12.5px] leading-snug text-text-primary">
          {candidates.map((c) => (
            <EpisodePick key={c.id} candidate={c} season={season} source={source} />
          ))}
        </ul>
      </dd>
    </div>
  );
}
