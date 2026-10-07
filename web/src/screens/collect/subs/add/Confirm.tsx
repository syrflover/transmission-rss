import { useId, useMemo, useState } from "react";
import { Link } from "react-router-dom";

import { Button } from "@/components/ui/button";
import { ApiError } from "@/lib/api";
import { useAfterDelay } from "@/lib/cached";

import { subscriptionAdded } from "../../cache";
import { btnAction, btnNeutral, btnPrimary, hintClass } from "../../channels/styles";
import type { RuleFields } from "../../rules/api";
import { ArchivedWorkNotice } from "../../rules/ArchivedWorkNotice";
import { receivesPastItemsNow } from "../../rules/archivedWork";
import { useArchivedWork } from "../../rules/useArchivedWork";
import { usePreview } from "../../rules/usePreview";
import type { Channel } from "../../channels/api";
import { subscribe, type ScheduleEntry, type TitleGroup } from "../api";
import { subtitleChoice } from "../format";
import type { Draft } from "./draft";
import { listed, PastChecklist, ReceiveProgress, receivable } from "./PastItems";
import { useReceive } from "./useReceive";

/**
 * The last step: the past items of the channel that the new rule would pick,
 * unticked until the user ticks them, and the button that creates the rule.
 * Creating the rule receives nothing; only the ticked items are received, each
 * with its own command, and their progress replaces the list.
 *
 * A subscription made before the first episode (`work` is `null`) has no
 * phrase and so no items to list: it only says what happens next.
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
  /** The release title picked, or `null` for `아직 첫 화 전이에요`. */
  work: TitleGroup | null;
  onCreated: () => void;
}) {
  const uid = useId();
  const waiting = work === null;
  const fields = useMemo<RuleFields>(
    () => ({
      match: work?.work ?? "",
      regex: false,
      case_insensitive: false,
      directory: draft.directory.trim(),
      episode: 1,
      state: "active",
    }),
    [work, draft.directory],
  );
  // A new rule is checked last in its channel. A waiting one has nothing to compare, so it asks for nothing.
  const preview = usePreview(channel.id, null, fields, channel.rule_count, 0, !waiting);
  const shown = preview.state === "ready" ? preview.preview : preview.state === "loading" ? preview.previous : null;
  const slow = useAfterDelay(shown === null && preview.state === "loading");

  const [ticked, setTicked] = useState<ReadonlySet<number>>(new Set());
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // The rule that already follows this anime in this channel (a `409`), to open.
  const [existing, setExisting] = useState<string | null>(null);
  const [ruleId, setRuleId] = useState<string | null>(null);
  // The rule was made paused: its work folder is coming over from the archive folder.
  const [held, setHeld] = useState(false);
  // How many ticked items the server receives after the move, for a held rule.
  const [heldTicked, setHeldTicked] = useState(0);
  const archivedWork = useArchivedWork(draft.directory);
  // The titles of the items asked for, kept because the preview moves on.
  const [titles, setTitles] = useState<ReadonlyMap<number, string>>(new Map());
  const receive = useReceive(ruleId);

  const items = shown ? listed(shown.items) : [];
  const tickedNow = items.filter((i) => receivable(i) && ticked.has(i.id));
  const blocked = !waiting && shown?.error != null;

  const submit = async () => {
    setBusy(true);
    setError(null);
    setExisting(null);
    try {
      const rule = await subscribe({
        channel_id: channel.id,
        anissia_anime_no: anime.anime_no,
        week: anime.week,
        work: work?.work ?? null,
        subtitles: draft.subtitles,
        // The creator is only stored with a followed one.
        creator: draft.subtitles === "follow" ? draft.creator : null,
        directory: draft.directory.trim(),
        receive: tickedNow.map((i) => i.id),
      });
      subscriptionAdded(channel.id);
      setTitles(new Map(tickedNow.map((i) => [i.id, i.title])));
      setRuleId(rule.id);
      // The server decided from the disk, not from the notice above (the library's records can be
      // stale): a rule that came back paused waits for its work folder, and the server receives
      // what was ticked once the folder came over; one that came back collecting receives it here.
      const receivesNow = receivesPastItemsNow(rule.state);
      setHeld(!receivesNow);
      setHeldTicked(receivesNow ? 0 : tickedNow.length);
      onCreated();
      if (tickedNow.length > 0 && receivesNow) receive.start(rule.id, tickedNow.map((i) => i.id));
    } catch (e) {
      setError(e instanceof ApiError ? e.message : "구독하지 못했어요. 다시 시도해 주세요.");
      // Subscribed already, from another tab or by an earlier press whose
      // answer was lost: its rule is where the past items are received.
      const current = e instanceof ApiError && e.code === "conflict" ? (e.current as { rule_id?: unknown } | null) : null;
      if (typeof current?.rule_id === "string") setExisting(current.rule_id);
    } finally {
      setBusy(false);
    }
  };

  if (ruleId !== null) {
    const titleOf = (id: number) => titles.get(id) ?? `항목 ${id}`;
    return (
      <section aria-labelledby={`${uid}-h`} className="flex min-w-0 flex-col gap-3.5">
        <h3 id={`${uid}-h`} className="text-[15px] font-bold">
          구독했어요
        </h3>
        <p className="min-w-0 text-[13px] leading-normal break-words text-text-secondary">
          {waiting ? (
            <>
              {anime.subject} 구독은 제목을 정하기 전까지 아무것도 받지 않아요. {channel.name ?? channel.host} 채널에 새 작품 제목이
              처음 나타나면 구독 탭에서 제목 후보로 알려요.
            </>
          ) : (
            <>
              {held
                ? `${anime.subject}의 작품 폴더를 보관 폴더에서 수집 폴더로 옮기는 중이에요. 옮기기가 끝나면 다음 RSS 확인부터 ${channel.name ?? channel.host} 채널에서 새 회차를 받아요. 그동안 규칙은 멈춰 있어요. ${
                    heldTicked > 0
                      ? `체크한 지난 항목 ${heldTicked}개는 옮긴 뒤 받아요. 옮기지 못하면 받지 않고, 규칙 화면에 까닭이 나와요.`
                      : "지난 항목은 받지 않았어요. 옮긴 뒤 규칙 화면에서 골라 받을 수 있어요. 옮기지 못하면 규칙 화면에 까닭이 나와요."
                  }`
                : `${anime.subject}의 새 회차는 다음 RSS 확인부터 ${channel.name ?? channel.host} 채널에서 받아요.`}
              {!held && receive.entries.length === 0 && " 지난 항목은 받지 않았어요. 규칙 화면에서 골라 받을 수 있어요."}
            </>
          )}
        </p>

        <ReceiveProgress entries={receive.entries} titleOf={titleOf} onRetry={receive.retry} />

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
        <dd className="m-0 min-w-0 break-all">
          {work ? work.work : <span className="text-text-secondary">첫 화가 올라오면 정해요 (제목 대기)</span>}
        </dd>
        <dt className="font-semibold text-text-muted">자막</dt>
        <dd className="m-0 min-w-0 break-words">
          {subtitleChoice({ subtitles: draft.subtitles, creator: draft.creator })}
        </dd>
        <dt className="font-semibold text-text-muted">저장 폴더</dt>
        <dd className="m-0 min-w-0 break-all">{draft.directory.trim()}</dd>
      </dl>

      {archivedWork && <ArchivedWorkNotice archived={archivedWork} />}

      {waiting && (
        <p className="min-w-0 rounded-xl border border-hairline bg-surface-2 px-3.5 py-3 text-[13px] leading-normal text-text-secondary">
          첫 화가 나오기 전이라 일치 문구가 아직 없어요. 구독해 두면 아무것도 받지 않고, 이 채널에 새 작품 제목이 처음 나타날 때
          구독 탭에서 제목 후보로 알려요. 후보를 고르면 그때 지난 항목을 확인하고 받아요.
        </p>
      )}

      {!waiting && (
        <div className="flex min-w-0 flex-col gap-2.5">
          <h4 className="text-[14px] font-bold">이미 기록된 항목</h4>
          <p className={hintClass}>
            구독만으로는 아무것도 받지 않아요. 지난 항목은 체크한 것만 받아요. ‘[Batch]’처럼 원하지 않는 항목은 체크하지 않으면 돼요.
            {archivedWork &&
              " 작품 폴더를 옮기는 동안에는 규칙이 받지 못해요. 체크한 항목은 옮기기가 끝난 뒤 받아요."}
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

          {shown && !shown.error && items.length === 0 && (
            <p className="text-[13px] leading-normal text-text-secondary">
              이 규칙이 받을 지난 항목이 없어요. 구독하면 다음 RSS 확인부터 받아요.
            </p>
          )}

          <PastChecklist items={items} ticked={ticked} onTicked={setTicked} label="이미 기록된 항목" />
          {shown?.truncated && <p className={hintClass}>항목이 많아서 최근 {shown.items.length}개만 보여줘요.</p>}
        </div>
      )}

      {error && (
        <p role="alert" className="text-[13px] leading-normal font-semibold text-urgent">
          {error}
        </p>
      )}
      {existing !== null && (
        <div className="flex">
          <Button asChild type="button" variant="ghost" className={btnAction}>
            <Link to={`/collect/rules?rule=${encodeURIComponent(existing)}`}>그 구독 규칙 열기</Link>
          </Button>
        </div>
      )}

      <div className="flex">
        <Button
          type="button"
          variant="ghost"
          className={btnPrimary}
          disabled={busy || blocked || (!waiting && shown === null)}
          onClick={() => void submit()}
        >
          {busy ? "구독하는 중" : tickedNow.length > 0 ? `구독하고 ${tickedNow.length}개 받기` : "구독"}
        </Button>
      </div>
    </section>
  );
}
