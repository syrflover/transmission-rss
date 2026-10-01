import { useId } from "react";
import { Link } from "react-router-dom";

import type { SubscriptionItem } from "./api";
import { airing, startDate, subtitleChoice } from "./format";

const tag =
  "inline-flex max-w-full items-center rounded-full border border-hairline px-[9px] py-[3px] text-[11.5px] leading-tight font-semibold text-text-secondary";

/**
 * One subscription. The weekday and time are the stored snapshot, so the card
 * reads the same while Anissia does not answer. The title opens the rule.
 */
export function SubscriptionCard({ item }: { item: SubscriptionItem }) {
  const headingId = useId();
  const { subscription } = item;
  const anime = subscription.anime;
  const title = anime?.subject ?? item.title ?? "이름 없는 작품";
  const waitingForTitle = item.title === null;
  const start = anime?.week === 8 || item.upcoming ? startDate(anime?.start_date ?? null) : null;

  return (
    <li
      aria-labelledby={headingId}
      className="flex min-w-0 flex-col gap-2.5 rounded-card border border-hairline-soft bg-surface-1 p-4 shadow-(--card-shadow)"
    >
      <div className="min-w-0">
        <h3 id={headingId} className="min-w-0 text-[16px] leading-snug font-bold break-words">
          <Link
            to={`/collect/rules?rule=${encodeURIComponent(item.rule_id)}`}
            className="rounded-sm underline-offset-4 outline-offset-2 hover:underline focus-visible:outline-2 focus-visible:outline-focus"
          >
            {title}
          </Link>
        </h3>
        {anime?.original_subject && (
          <p className="mt-0.5 min-w-0 text-xs leading-snug break-words text-text-muted">{anime.original_subject}</p>
        )}
      </div>

      <div className="flex min-w-0 flex-wrap items-center gap-1.5">
        {anime && <span className={tag}>{airing(anime)}</span>}
        {start && <span className="text-xs text-text-muted">{start} 시작</span>}
        {waitingForTitle && <span className={`${tag} border-focus text-focus`}>제목 대기</span>}
        {item.state === "paused" && <span className={`${tag} text-text-primary`}>멈춤</span>}
        <span className={`${tag} break-all`}>{item.channel_name ?? item.channel_host}</span>
      </div>

      <dl className="m-0 grid grid-cols-[64px_minmax(0,1fr)] gap-x-3 gap-y-1.5 text-[13px]">
        <dt className="font-semibold text-text-muted">자막</dt>
        <dd className="m-0 min-w-0 break-words text-text-secondary">{subtitleChoice(subscription)}</dd>
        <dt className="font-semibold text-text-muted">저장 폴더</dt>
        <dd className="m-0 min-w-0 break-all text-text-secondary">{item.directory || "수집 폴더"}</dd>
      </dl>

      {waitingForTitle && (
        <p className="min-w-0 text-xs leading-normal text-text-secondary">
          {item.state === "paused"
            ? "영상 받기가 꺼져 있어서 제목 후보도 알리지 않아요."
            : "일치 문구가 없어서 아직 아무것도 받지 않아요. 이 채널에 새 작품 제목이 나타나면 제목 후보로 알려요."}
        </p>
      )}
    </li>
  );
}

