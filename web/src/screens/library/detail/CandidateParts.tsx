import { Link } from "react-router-dom";

import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

import { btnAction } from "../../collect/channels/styles";
import { jobPath } from "../../todo/api";
import type { Candidate, CandidateJob } from "../api";
import { CheckIcon, ExternalIcon } from "../icons";
import { jobStatus, KIND_TEXT, updatedText, type CreatePhase, type Kind } from "./candidates";

const linkClass = "text-focus underline underline-offset-2 hover:text-text-primary";

/**
 * The kind of a candidate as a small tag: a word, never colour alone. A received
 * candidate's tag is the way to the job that received it.
 */
export function KindTag({ kind, job }: { kind: Kind; job: CandidateJob | null }) {
  const tag = "inline-flex items-center gap-1 rounded-full border border-hairline px-2 py-px text-[11.5px] font-semibold whitespace-nowrap text-text-secondary";
  if (kind === "received" && job) {
    return (
      <Link to={jobPath(job.id)} className={cn(tag, "relative min-h-6 hover:border-text-secondary hover:text-text-primary max-[720px]:after:absolute max-[720px]:after:inset-x-0 max-[720px]:after:-inset-y-2 max-[720px]:after:content-['']")}>
        <CheckIcon className="size-3 text-ok" />
        {KIND_TEXT.received}
        <span className="sr-only">, 작업 보기</span>
      </Link>
    );
  }
  return <span className={tag}>{KIND_TEXT[kind]}</span>;
}

/** What a job's item says about its candidate, as the way to the job's page. */
export function JobStatusLink({ job }: { job: CandidateJob | null }) {
  const text = jobStatus(job);
  if (!job || text === null) return null;
  return (
    <Link to={jobPath(job.id)} className={cn("inline-flex min-h-6 items-center text-xs font-semibold max-[720px]:min-h-9", linkClass)}>
      {text}
      <span className="sr-only">, 작업 보기</span>
    </Link>
  );
}

/** `Anissia 갱신 9월 30일 23:06`: when Anissia's line was updated, which is neither the post's nor the episode's date. */
export function UpdatedAt({ candidate }: { candidate: Candidate }) {
  return <span>Anissia 갱신 {updatedText(candidate)}</span>;
}

/** A small link to the creator's post in a new window. */
export function PostLink({ url }: { url: string }) {
  return (
    <a
      href={url}
      target="_blank"
      rel="noopener noreferrer"
      className={cn("inline-flex min-h-6 items-center gap-1 font-semibold max-[720px]:min-h-9", linkClass)}
    >
      게시물
      <ExternalIcon className="size-3" />
      <span className="sr-only"> 보기 (새 창)</span>
    </a>
  );
}

/**
 * What became of the last `받기`: it only says the job was accepted, never that
 * the subtitle was received. After a lost answer `다시 보내기` sends the same
 * request again.
 */
export function CreateStatus({ phase, onResend }: { phase: CreatePhase; onResend: () => void }) {
  switch (phase.kind) {
    case "idle":
      return null;
    case "sending":
      return (
        <p role="status" className="m-0 text-xs text-text-muted">
          작업을 만드는 중이에요.
        </p>
      );
    case "made":
      return (
        <p role="status" className="m-0 text-[12.5px] text-text-secondary">
          작업을 만들었어요.{" "}
          <Link to={jobPath(phase.jobId)} className={cn("font-semibold", linkClass)}>
            작업 보기
          </Link>
        </p>
      );
    case "unconfirmed":
      return (
        <div role="alert" className="flex flex-wrap items-center gap-x-3 gap-y-1.5">
          <p className="m-0 min-w-0 text-[12.5px] leading-snug text-text-secondary [overflow-wrap:anywhere]">
            {phase.message} 작업을 만들었는지 알 수 없어요. 같은 요청을 다시 보내도 작업은 하나만 생겨요.
          </p>
          <Button type="button" variant="ghost" className={btnAction} onClick={onResend}>
            다시 보내기
          </Button>
        </div>
      );
    case "refused":
      return (
        <p role="alert" className="m-0 text-[12.5px] leading-snug font-semibold text-urgent [overflow-wrap:anywhere]">
          {phase.message}
        </p>
      );
  }
}
