import { Link } from "react-router-dom";

import { Button } from "@/components/ui/button";
import { when } from "@/lib/time";

import type { ArchiveSuggestion, Ground } from "../collect/archive/api";
import { suggestionTitle } from "../collect/archive/ground";
import { btnNeutral } from "../collect/channels/styles";
import { nameTitleLink } from "../collect/subs/Candidates";
import { startDate } from "../collect/subs/format";
import type { TitleCandidate } from "../collect/subs/api";
import { Tag } from "./badges";

/** One row of `제안`: a title candidate or an archive suggestion, in the shape the list shows. */
export interface Suggestion {
  id: string;
  kind: "candidate" | "archive";
  title: string;
  /** One quiet line. */
  detail: string;
  /** When it came up (Unix ms); `null` when the source does not say. */
  at: number | null;
  /** What orders the list, newest first. */
  order: number;
  to: string;
  /** Short: where the button goes. */
  action: string;
}

/** The moment a ground started to hold, as far as the suggestion says. */
function groundTime(ground: Ground, lastReceived: number | null): number | null {
  switch (ground.kind) {
    case "quiet":
      return ground.since;
    case "ended": {
      const match = ground.end_date === null ? null : /^(\d{4})-(\d{2})(?:-(\d{2}))?$/.exec(ground.end_date);
      return match ? Date.UTC(Number(match[1]), Number(match[2]) - 1, Number(match[3] ?? 1)) : lastReceived;
    }
    case "unlisted":
      return lastReceived;
  }
}

function groundShort(ground: Ground): string {
  switch (ground.kind) {
    case "ended": {
      const date = startDate(ground.end_date);
      return date ? `${date} 방영 종료` : "방영 종료";
    }
    case "unlisted":
      return "편성표에서 빠짐";
    case "quiet":
      return "4주 넘게 새 항목 없음";
  }
}

export function fromCandidate(c: TitleCandidate): Suggestion {
  return {
    id: `candidate:${c.channel_id}:${c.key}`,
    kind: "candidate",
    title: c.work,
    detail: `${c.channel_name ?? c.channel_host} · 항목 ${c.items}개`,
    at: c.first_seen_at,
    order: c.first_seen_at,
    to: nameTitleLink(c.channel_id, c.work, c.folder),
    action: "제목 정하기",
  };
}

export function fromArchive(s: ArchiveSuggestion): Suggestion {
  const times = s.grounds
    .map((g) => groundTime(g, s.last_received_at))
    .filter((t): t is number => t !== null);
  return {
    id: `archive:${s.rule_id}`,
    kind: "archive",
    title: suggestionTitle(s),
    detail: s.grounds.length > 0 ? groundShort(s.grounds[0]) : (s.channel_name ?? s.channel_host),
    // The suggestion has no moment of its own; the list is ordered by the ground it stands on.
    at: null,
    order: times.length > 0 ? Math.max(...times) : (s.last_received_at ?? 0),
    to: `/collect/rules?rule=${encodeURIComponent(s.rule_id)}`,
    action: "규칙 보기",
  };
}

/** The rows of `제안`, newest first across both sources. */
export function suggestionRows(candidates: readonly TitleCandidate[], archives: readonly ArchiveSuggestion[]): Suggestion[] {
  return [...candidates.map(fromCandidate), ...archives.map(fromArchive)].sort((a, b) => b.order - a.order);
}

const KIND_LABEL = { candidate: "제목 후보", archive: "보관 제안" } as const;

/** Thin rows: a small kind tag, the title, one quiet line, the time, and one neutral button. */
export function SuggestionRows({ rows }: { rows: readonly Suggestion[] }) {
  return (
    <ul
      aria-label="제안"
      className="m-0 list-none overflow-hidden rounded-card border border-hairline-soft bg-surface-1 p-0 shadow-(--card-shadow)"
    >
      {rows.map((row) => (
        <li
          key={row.id}
          className="grid grid-cols-[84px_minmax(0,1fr)_auto_auto] items-center gap-x-3.5 gap-y-1 border-t border-hairline-soft px-3.5 py-2.5 first:border-t-0 max-[720px]:grid-cols-[minmax(0,1fr)_auto] max-[720px]:gap-y-1.5 max-[720px]:px-3"
        >
          <span className="justify-self-start max-[720px]:col-start-1 max-[720px]:row-start-1">
            <Tag>{KIND_LABEL[row.kind]}</Tag>
          </span>
          <div className="min-w-0 max-[720px]:col-start-1 max-[720px]:row-start-2">
            <p className="line-clamp-2 text-[14px] leading-snug font-semibold">{row.title}</p>
            <p className="mt-0.5 min-w-0 text-xs leading-snug text-text-muted">{row.detail}</p>
          </div>
          <time className="min-w-[5.5rem] text-right text-xs whitespace-nowrap text-text-muted max-[720px]:col-start-2 max-[720px]:row-start-1 max-[720px]:min-w-0">
            {row.at === null ? null : when(row.at)}
          </time>
          <Button
            asChild
            variant="ghost"
            className={`${btnNeutral} max-[720px]:col-start-2 max-[720px]:row-start-2`}
          >
            <Link to={row.to}>{row.action}</Link>
          </Button>
        </li>
      ))}
    </ul>
  );
}
