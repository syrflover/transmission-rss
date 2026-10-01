import { useId, useState } from "react";
import { Link } from "react-router-dom";

import { Button } from "@/components/ui/button";
import { ApiError } from "@/lib/api";
import { useCached } from "@/lib/cached";
import { dateTime } from "@/lib/time";

import { candidatesChanged, KEYS } from "../cache";
import { btnAction, btnNeutral } from "../channels/styles";
import { fetchCandidates, rejectCandidate, type TitleCandidate } from "./api";

/** The page that gives a work to a waiting subscription (`NameTitle`). */
export function nameTitleLink(channelId: string, work: string, folder: string | null): string {
  const params = new URLSearchParams({ channel: channelId, work });
  if (folder !== null) params.set("folder", folder);
  return `/collect/subs/title?${params}`;
}

/** The ids a candidate is told apart by on the screen. */
const idOf = (c: TitleCandidate) => `${c.channel_id}\n${c.key}`;

/**
 * The title candidates, at the top of the 구독 tab: new works that appeared in
 * a channel with a subscription waiting for its title. It is calm information
 * (blue, not a warning); nothing here changes until the user picks `정하기` or
 * turns a candidate down with `거절`. Renders nothing while there are none.
 */
export function Candidates() {
  const headingId = useId();
  const list = useCached<TitleCandidate[]>(KEYS.candidates, fetchCandidates, "제목 후보를 불러오지 못했어요.");
  const candidates = list.data;
  const [rejecting, setRejecting] = useState<string | null>(null);
  const [error, setError] = useState<{ id: string; message: string } | null>(null);

  const reject = async (candidate: TitleCandidate) => {
    const id = idOf(candidate);
    setRejecting(id);
    setError(null);
    try {
      await rejectCandidate(candidate.channel_id, candidate.work);
      candidatesChanged();
    } catch (e) {
      setError({ id, message: e instanceof ApiError ? e.message : "거절하지 못했어요. 다시 시도해 주세요." });
    } finally {
      setRejecting(null);
    }
  };

  if (candidates === undefined || candidates.length === 0) {
    // A failed first load is said once; a quiet "none" says nothing.
    if (candidates === undefined && list.error !== null) {
      return (
        <p role="alert" className="text-[13px] font-semibold text-urgent">
          {list.error}
        </p>
      );
    }
    return null;
  }

  return (
    <section
      aria-labelledby={headingId}
      className="flex min-w-0 flex-col gap-3 rounded-card border border-[color-mix(in_srgb,var(--focus-ring)_45%,transparent)] bg-[color-mix(in_srgb,var(--focus-ring)_7%,transparent)] p-4 max-[480px]:p-3.5"
    >
      <div className="flex min-w-0 flex-col gap-1">
        <h2 id={headingId} className="text-[15px] font-bold text-focus">
          제목 후보 {candidates.length}개
        </h2>
        <p className="min-w-0 text-[13px] leading-normal text-text-secondary">
          제목을 기다리는 구독이 있는 채널에 새 작품이 올라왔어요. 어느 구독의 제목인지 골라 주면 그때부터 받을 수 있어요.
        </p>
      </div>

      <ul className="m-0 flex list-none flex-col gap-2.5 p-0">
        {candidates.map((candidate) => {
          const id = idOf(candidate);
          return (
            <li
              key={id}
              data-testid="title-candidate"
              className="flex min-w-0 flex-col gap-2 rounded-[10px] border border-hairline-soft bg-surface-1 px-3.5 py-3"
            >
              <div className="min-w-0">
                <p className="min-w-0 text-[15px] leading-snug font-bold break-all">{candidate.work}</p>
                <p className="mt-0.5 min-w-0 text-xs leading-snug break-all text-text-muted">{candidate.latest_title}</p>
              </div>
              <p className="min-w-0 text-xs leading-normal break-words text-text-secondary">
                {candidate.channel_name ?? candidate.channel_host} · 항목 {candidate.items}개 · {dateTime(candidate.first_seen_at)} 처음 기록
              </p>
              <div className="flex flex-wrap items-center gap-2">
                <Button asChild type="button" variant="ghost" className={btnAction}>
                  <Link to={nameTitleLink(candidate.channel_id, candidate.work, candidate.folder)}>정하기</Link>
                </Button>
                <Button
                  type="button"
                  variant="ghost"
                  className={btnNeutral}
                  disabled={rejecting !== null}
                  onClick={() => void reject(candidate)}
                >
                  {rejecting === id ? "거절하는 중" : "거절"}
                </Button>
              </div>
              {error?.id === id && (
                <p role="alert" className="text-xs leading-normal font-semibold text-urgent">
                  {error.message}
                </p>
              )}
            </li>
          );
        })}
      </ul>
    </section>
  );
}
