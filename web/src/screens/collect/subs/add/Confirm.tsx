import { useId, useMemo, useState } from "react";
import { Link } from "react-router-dom";

import { Button } from "@/components/ui/button";
import { ApiError } from "@/lib/api";
import { useAfterDelay } from "@/lib/cached";
import { dateTime } from "@/lib/time";

import { subscriptionAdded } from "../../cache";
import { btnNeutral, btnPrimary, hintClass } from "../../channels/styles";
import type { PreviewItem, RuleFields } from "../../rules/api";
import { usePreview } from "../../rules/usePreview";
import type { Channel } from "../../channels/api";
import { subscribe, type ScheduleEntry, type TitleGroup } from "../api";
import { subtitleChoice } from "../format";
import type { Draft } from "./draft";
import { useReceive, type ReceivePhase } from "./useReceive";

const check = "mt-0.5 size-[18px] flex-none accent-focus";

/** A past item the rule picks that no rule has received: the ones the user may tick. */
function receivable(item: PreviewItem): boolean {
  return item.kind === "mine" && item.stored_result === "no_match";
}

const PHASE_TEXT: Record<ReceivePhase["kind"], string> = {
  sending: "요청하는 중",
  waiting: "추가하는 중",
  added: "추가함",
  failed: "추가하지 못함",
};

/**
 * The last step: the past items of the channel that the new rule would pick,
 * unticked until the user ticks them, and the button that creates the rule.
 * Creating the rule receives nothing; only the ticked items are received, each
 * with its own command, and their progress replaces the list.
 */
export function Confirm({
  draft,
  anime,
  channel,
  work,
  onCreated,
}: {
  draft: Draft;
  anime: ScheduleEntry;
  channel: Channel;
  work: TitleGroup;
  onCreated: () => void;
}) {
  const uid = useId();
  const fields = useMemo<RuleFields>(
    () => ({
      match: work.work,
      regex: false,
      case_insensitive: false,
      directory: draft.directory.trim(),
      episode: 1,
      state: "active",
    }),
    [work.work, draft.directory],
  );
  // A new rule is checked last in its channel.
  const preview = usePreview(channel.id, null, fields, channel.rule_count);
  const shown = preview.state === "ready" ? preview.preview : preview.state === "loading" ? preview.previous : null;
  const slow = useAfterDelay(shown === null && preview.state === "loading");

  const [ticked, setTicked] = useState<ReadonlySet<number>>(new Set());
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [ruleId, setRuleId] = useState<string | null>(null);
  // The titles of the items asked for, kept because the preview moves on.
  const [titles, setTitles] = useState<ReadonlyMap<number, string>>(new Map());
  const receive = useReceive(ruleId);

  const mine = shown?.items.filter((i) => i.kind === "mine") ?? [];
  const pickable = mine.filter(receivable);
  const tickedNow = pickable.filter((i) => ticked.has(i.id));
  const blocked = shown?.error != null;

  const toggle = (id: number) =>
    setTicked((prev) => {
      const next = new Set(prev);
      if (!next.delete(id)) next.add(id);
      return next;
    });

  const submit = async () => {
    setBusy(true);
    setError(null);
    try {
      const rule = await subscribe({
        channel_id: channel.id,
        anissia_anime_no: anime.anime_no,
        week: anime.week,
        work: work.work,
        subtitles: draft.subtitles,
        // The creator is only stored with a followed one.
        creator: draft.subtitles === "follow" ? draft.creator : null,
        directory: draft.directory.trim(),
      });
      subscriptionAdded(channel.id);
      setTitles(new Map(tickedNow.map((i) => [i.id, i.title])));
      setRuleId(rule.id);
      onCreated();
      if (tickedNow.length > 0) receive.start(rule.id, tickedNow.map((i) => i.id));
    } catch (e) {
      setError(e instanceof ApiError ? e.message : "구독하지 못했어요. 다시 시도해 주세요.");
    } finally {
      setBusy(false);
    }
  };

  if (ruleId !== null) {
    const titleOf = (id: number) => titles.get(id) ?? `항목 ${id}`;
    const settled = receive.entries.every((e) => e.phase.kind === "added" || e.phase.kind === "failed");
    return (
      <section aria-labelledby={`${uid}-h`} className="flex min-w-0 flex-col gap-3.5">
        <h3 id={`${uid}-h`} className="text-[15px] font-bold">
          구독했어요
        </h3>
        <p className="min-w-0 text-[13px] leading-normal break-words text-text-secondary">
          {anime.subject}의 새 회차는 다음 RSS 확인부터 {channel.name ?? channel.host} 채널에서 받아요.
          {receive.entries.length === 0 && " 지난 항목은 받지 않았어요."}
        </p>

        {receive.entries.length > 0 && (
          <ul className="m-0 flex list-none flex-col gap-2 p-0" aria-label="지난 항목 받기">
            {receive.entries.map((entry) => (
              <li
                key={entry.itemId}
                className="flex min-w-0 flex-col gap-1 rounded-[10px] border border-hairline-soft bg-surface-2 px-3 py-2.5"
              >
                <p className="min-w-0 text-[13px] leading-snug break-all">{titleOf(entry.itemId)}</p>
                <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
                  <span
                    role="status"
                    className={
                      entry.phase.kind === "failed"
                        ? "text-xs font-semibold text-urgent"
                        : entry.phase.kind === "added"
                          ? "text-xs font-semibold text-ok"
                          : "text-xs font-semibold text-text-secondary"
                    }
                  >
                    {PHASE_TEXT[entry.phase.kind]}
                  </span>
                  {entry.phase.kind === "failed" && (
                    <>
                      <span className="min-w-0 text-xs break-words text-text-secondary">{entry.phase.message}</span>
                      <Button
                        type="button"
                        variant="ghost"
                        className={btnNeutral}
                        onClick={() => receive.retry(entry.itemId)}
                      >
                        다시 받기
                      </Button>
                    </>
                  )}
                </div>
              </li>
            ))}
          </ul>
        )}
        {receive.entries.length > 0 && !settled && (
          <p className={hintClass}>worker가 하나씩 추가해요. 이 화면을 떠나도 계속돼요.</p>
        )}

        <div className="flex flex-wrap gap-2.5">
          <Button asChild type="button" variant="ghost" className={btnPrimary}>
            <Link to={`/collect/rules?rule=${encodeURIComponent(ruleId)}`}>규칙 열기</Link>
          </Button>
          <Button asChild type="button" variant="ghost" className={btnNeutral}>
            <Link to="/collect/subs">구독 목록</Link>
          </Button>
        </div>
      </section>
    );
  }

  return (
    <section aria-labelledby={`${uid}-h`} className="flex min-w-0 flex-col gap-4">
      <h3 id={`${uid}-h`} className="text-[15px] font-bold">
        확인하고 구독해요
      </h3>

      <dl className="m-0 grid grid-cols-[84px_minmax(0,1fr)] gap-x-3 gap-y-1.5 text-[13px]">
        <dt className="font-semibold text-text-muted">작품</dt>
        <dd className="m-0 min-w-0 break-words">{anime.subject}</dd>
        <dt className="font-semibold text-text-muted">채널</dt>
        <dd className="m-0 min-w-0 break-all">{channel.name ?? channel.host}</dd>
        <dt className="font-semibold text-text-muted">일치 문구</dt>
        <dd className="m-0 min-w-0 break-all">{work.work}</dd>
        <dt className="font-semibold text-text-muted">자막</dt>
        <dd className="m-0 min-w-0 break-words">
          {subtitleChoice({ subtitles: draft.subtitles, creator: draft.creator })}
        </dd>
        <dt className="font-semibold text-text-muted">저장 폴더</dt>
        <dd className="m-0 min-w-0 break-all">{draft.directory.trim()}</dd>
      </dl>

      <div className="flex min-w-0 flex-col gap-2.5">
        <h4 className="text-[14px] font-bold">이미 기록된 항목</h4>
        <p className={hintClass}>
          구독만으로는 아무것도 받지 않아요. 지난 항목은 체크한 것만 받아요. ‘[Batch]’처럼 원하지 않는 항목은 체크하지 않으면 돼요.
        </p>

        {preview.state === "failed" && (
          <p role="alert" className="text-[13px] font-semibold text-urgent">
            {preview.message}
          </p>
        )}
        {shown?.error && (
          <p role="alert" className="text-[13px] font-semibold text-urgent">
            {shown.error.message}
          </p>
        )}
        {slow && <p className="text-[13px] text-text-muted">기록을 확인하는 중이에요.</p>}

        {shown && !shown.error && mine.length === 0 && (
          <p className="text-[13px] leading-normal text-text-secondary">
            이 규칙이 받을 지난 항목이 없어요. 구독하면 다음 RSS 확인부터 받아요.
          </p>
        )}

        {pickable.length > 0 && (
          <div className="flex flex-wrap items-center gap-2">
            <Button
              type="button"
              variant="ghost"
              className={btnNeutral}
              onClick={() => setTicked(new Set(pickable.map((i) => i.id)))}
            >
              모두 선택
            </Button>
            <Button type="button" variant="ghost" className={btnNeutral} onClick={() => setTicked(new Set())}>
              선택 해제
            </Button>
            <span className="text-xs text-text-muted" role="status">
              {tickedNow.length}개 선택
            </span>
          </div>
        )}

        {mine.length > 0 && (
          <ul className="m-0 flex list-none flex-col gap-2 p-0" aria-label="이미 기록된 항목">
            {mine.map((item) => {
              const can = receivable(item);
              return (
                <li key={item.id}>
                  <label
                    className={
                      can
                        ? "flex min-w-0 cursor-pointer items-start gap-3 rounded-[10px] border border-hairline-soft bg-surface-2 px-3 py-2.5 has-[:checked]:border-focus"
                        : "flex min-w-0 items-start gap-3 rounded-[10px] border border-hairline-soft bg-surface-2 px-3 py-2.5 opacity-70"
                    }
                  >
                    <input
                      type="checkbox"
                      className={check}
                      disabled={!can}
                      checked={can && ticked.has(item.id)}
                      onChange={() => toggle(item.id)}
                    />
                    <span className="flex min-w-0 flex-col gap-0.5">
                      <span className="min-w-0 text-[13px] leading-snug break-all">{item.title}</span>
                      <span className="text-xs text-text-muted">
                        {dateTime(item.first_seen_at)} 기록
                        {!can && " · 이미 처리된 항목이라 받을 수 없어요"}
                      </span>
                    </span>
                  </label>
                </li>
              );
            })}
          </ul>
        )}
        {shown?.truncated && (
          <p className={hintClass}>항목이 많아서 최근 {shown.items.length}개만 보여줘요.</p>
        )}
      </div>

      {error && (
        <p role="alert" className="text-[13px] leading-normal font-semibold text-urgent">
          {error}
        </p>
      )}

      <div className="flex">
        <Button
          type="button"
          variant="ghost"
          className={btnPrimary}
          disabled={busy || blocked || shown === null}
          onClick={() => void submit()}
        >
          {busy ? "구독하는 중" : tickedNow.length > 0 ? `구독하고 ${tickedNow.length}개 받기` : "구독"}
        </Button>
      </div>
    </section>
  );
}
