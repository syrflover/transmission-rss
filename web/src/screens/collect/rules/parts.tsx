import { Link } from "react-router-dom";

import { Button } from "@/components/ui/button";
import { ago, dateTime } from "@/lib/time";
import { cn } from "@/lib/utils";
import { workPath } from "@/screens/library/api";
import { coverOf } from "@/screens/library/model";
import { Cover } from "@/screens/library/WorkItem";

import { btnNeutral } from "../channels/styles";
import { airing, subtitleChoice } from "../subs/format";
import type { Rule, RuleSeason } from "./api";
import { parseEpisode, type Draft } from "./draft";

/** The empty box that keeps the poster's size for a rule with no cover. */
const posterBox =
  "col-start-1 row-span-2 row-start-1 aspect-[2/3] w-full self-start overflow-hidden rounded-xl max-[720px]:row-span-1 max-[720px]:row-start-2";

/** `9 / 12화 받음`, with a bar whose rest is dotted while the episode count is unknown. */
function Progress({ season }: { season: RuleSeason }) {
  const total = season.episodes;
  // With no total the bar only shows that there is more to come.
  const share =
    total === null ? season.videos / (season.videos + 4) : total === 0 ? 0 : Math.min(1, season.videos / total);
  const text = `${season.videos} / ${total ?? "?"}화 받음`;
  return (
    <div className="flex min-w-0 flex-col gap-1.5" data-testid="progress">
      <span>{text}</span>
      <div
        {...(total === null
          ? { "aria-hidden": true }
          : { role: "progressbar", "aria-valuemin": 0, "aria-valuemax": total, "aria-valuenow": season.videos })}
        className={cn("flex h-1.5 w-full max-w-60 overflow-hidden rounded-full", total !== null && "bg-surface-3")}
      >
        <span className="h-full flex-none bg-ok" style={{ width: `${Math.round(share * 100)}%` }} />
        {total === null && (
          <span
            data-testid="progress-rest"
            className="h-full min-w-0 flex-1 border-t-[3px] border-dotted border-text-muted"
          />
        )}
      </div>
    </div>
  );
}

const linkButton =
  "ml-2 inline-flex min-h-6 items-center rounded-md px-1.5 text-[12.5px] font-semibold text-focus underline underline-offset-2 outline-offset-2 hover:bg-surface-2 focus-visible:outline-2 focus-visible:outline-focus disabled:cursor-not-allowed disabled:text-text-muted disabled:no-underline max-[720px]:min-h-9";

/**
 * The four lines at the top of every rule, filled the same way for each so the
 * poster keeps one size: broadcast day and quarter, subtitle creator with its
 * `변경`, progress, and the last collection. What is not known is `미정` or
 * `아직 없음`. A rule that follows no anime has `편성표와 연결` beside the
 * broadcast line. The poster is the connected work's cover, or an empty box.
 */
export function RuleSummary({
  rule,
  onChangeCreator,
  onLink,
  creatorOpen,
  linkOpen,
}: {
  /** The stored rule; `null` for one not saved yet. */
  rule: Rule | null;
  onChangeCreator: () => void;
  onLink: () => void;
  creatorOpen: boolean;
  linkOpen: boolean;
}) {
  const subscription = rule?.subscription ?? null;
  const anime = subscription?.anime ?? null;
  const season = rule?.season ?? null;
  const lastReceivedAt = rule?.last_received_at ?? null;

  const quarter = subscription ? `${subscription.quarter.year}년 ${subscription.quarter.number}분기` : null;
  const broadcast = quarter ? (anime ? `${airing(anime)} · ${quarter}` : quarter) : "미정";

  return (
    <>
      {season ? (
        <Link
          to={`${workPath(season.work_id)}?season=${season.number}`}
          aria-label={`${season.work_name} 작품 보기`}
          className={cn(posterBox, "block outline-offset-2 focus-visible:outline-2 focus-visible:outline-focus")}
          data-testid="rule-poster"
        >
          <Cover
            work={coverOf(season.work_name)}
            imageUrl={season.cover_url}
            className="size-full rounded-xl"
            letterClass="text-3xl"
          />
        </Link>
      ) : (
        <div
          aria-hidden="true"
          className={cn(posterBox, "border border-hairline-soft bg-surface-3")}
          data-testid="rule-poster"
        />
      )}
      <dl
        aria-label="규칙 요약"
        className="col-start-2 row-start-2 m-0 grid min-w-0 grid-cols-[76px_minmax(0,1fr)] content-start gap-x-3 gap-y-2 text-[13px] max-[720px]:row-start-2"
      >
        <dt className="font-semibold text-text-muted">방영</dt>
        <dd className="m-0 min-w-0 text-text-secondary">
          {broadcast}
          {rule && subscription === null && (
            <button type="button" className={linkButton} aria-expanded={linkOpen} onClick={onLink}>
              편성표와 연결
            </button>
          )}
        </dd>
        <dt className="font-semibold text-text-muted">자막 제작자</dt>
        <dd className="m-0 min-w-0 text-text-secondary">
          {subscription ? subtitleChoice(subscription) : "미정"}
          {subscription && (
            <button
              type="button"
              className={linkButton}
              aria-expanded={creatorOpen}
              disabled={subscription.subtitles === "none"}
              title={subscription.subtitles === "none" ? "자막 받기를 켠 뒤 바꿀 수 있어요" : undefined}
              onClick={onChangeCreator}
            >
              변경
            </button>
          )}
        </dd>
        <dt className="font-semibold text-text-muted">진행</dt>
        <dd className="m-0 min-w-0 text-text-secondary">{season ? <Progress season={season} /> : "아직 없음"}</dd>
        <dt className="font-semibold text-text-muted">마지막 수집</dt>
        <dd className="m-0 min-w-0 text-text-secondary">
          {lastReceivedAt === null ? "아직 없음" : `${ago(lastReceivedAt)} (${dateTime(lastReceivedAt)})`}
        </dd>
      </dl>
    </>
  );
}

function stateLabel(state: Rule["state"]): string {
  return state === "archived" ? "보관됨" : state === "paused" ? "멈춤" : "수집 중";
}

const CHANGED_FIELDS: { label: string; server: (r: Rule) => string; mine: (d: Draft) => string }[] = [
  { label: "일치 문구", server: (r) => r.match ?? "(비어 있음)", mine: (d) => d.match || "(비어 있음)" },
  { label: "정규식", server: (r) => (r.regex ? "켬" : "끔"), mine: (d) => (d.regex ? "켬" : "끔") },
  {
    label: "대소문자",
    server: (r) => (r.case_insensitive ? "무시" : "구분"),
    mine: (d) => (d.case_insensitive ? "무시" : "구분"),
  },
  { label: "저장 폴더", server: (r) => r.directory || "(비어 있음)", mine: (d) => d.directory.trim() || "(비어 있음)" },
  {
    label: "회차 변환",
    server: (r) => String(r.episode),
    mine: (d) => (parseEpisode(d.episode) === null ? d.episode : String(parseEpisode(d.episode))),
  },
  {
    label: "상태",
    server: (r) => stateLabel(r.state),
    mine: (d) => stateLabel(d.state),
  },
];

/** The server's rule after someone else saved first, beside the untouched input. */
export function ConflictNotice({ current, draft }: { current: Rule; draft: Draft }) {
  const differing = CHANGED_FIELDS.filter((f) => f.server(current) !== f.mine(draft));
  return (
    <section role="alert" className="flex min-w-0 flex-col gap-2 rounded-xl border border-hairline bg-surface-2 p-3.5">
      <p className="text-sm font-bold">다른 곳에서 먼저 저장했어요.</p>
      <p className="text-[13px] leading-normal text-text-secondary">
        입력한 값은 그대로 두었어요. 지금 저장된 값과 다른 항목은 아래와 같아요. 확인하고 다시 저장할 수 있어요.
      </p>
      {differing.length > 0 ? (
        <dl className="m-0 grid grid-cols-[92px_minmax(0,1fr)] gap-x-3 gap-y-2 text-[13px] max-[480px]:grid-cols-1 max-[480px]:gap-y-0.5">
          {differing.map((f) => (
            <div key={f.label} className="contents">
              <dt className="font-semibold text-text-muted">{f.label}</dt>
              <dd className="m-0 min-w-0 break-all max-[480px]:mb-1.5">
                <span className="block">
                  <span className="text-text-muted">저장된 값 </span>
                  <span data-testid="conflict-server">{f.server(current)}</span>
                </span>
                <span className="block">
                  <span className="text-text-muted">내 입력 </span>
                  <span className="font-semibold">{f.mine(draft)}</span>
                </span>
              </dd>
            </div>
          ))}
        </dl>
      ) : (
        <p className="text-[13px] text-text-secondary">저장된 값이 입력과 같아요. 저장하면 이 화면이 최신 버전이 돼요.</p>
      )}
    </section>
  );
}

/** Moves the rule within its channel's check order (applied when saved). */
export function OrderRow({
  siblings,
  position,
  onMove,
}: {
  /** The channel's rules in check order, this one included. */
  siblings: Rule[];
  /** Where the edited rule stands among them, from 0. */
  position: number;
  onMove: (to: number) => void;
}) {
  const others = siblings.length;
  return (
    <div className="flex min-w-0 flex-col gap-2">
      <div className="flex flex-wrap items-center justify-between gap-x-3 gap-y-2">
        <p className="text-[13px] font-semibold" data-testid="order-position">
          이 채널에서 {position + 1}번째로 검사해요 <span className="font-normal text-text-muted">({others}개 중)</span>
        </p>
        <div className="flex gap-2">
          <Button
            type="button"
            variant="ghost"
            className={btnNeutral}
            disabled={position <= 0}
            onClick={() => onMove(position - 1)}
          >
            앞으로
          </Button>
          <Button
            type="button"
            variant="ghost"
            className={btnNeutral}
            disabled={position >= others - 1}
            onClick={() => onMove(position + 1)}
          >
            뒤로
          </Button>
        </div>
      </div>
      <p className="text-xs leading-normal text-text-muted">
        두 규칙이 같은 항목에 맞으면 앞 규칙이 가져가요. 순서는 저장할 때 바뀌어요.
      </p>
    </div>
  );
}
