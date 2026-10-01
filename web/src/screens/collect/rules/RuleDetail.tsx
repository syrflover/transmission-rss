import { useEffect, useId, useMemo, useRef, useState, type FormEvent } from "react";

import { Link } from "react-router-dom";

import { ApiError } from "@/lib/api";
import { forget } from "@/lib/cached";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

import { ArchiveBanner } from "../archive/ArchiveBanner";
import { KEYS, subscriptionChanged, suggestionGone } from "../cache";
import { changeCreator } from "../subs/api";
import { useReceive } from "../subs/add/useReceive";
import { CreatorPicker } from "../subs/CreatorPicker";
import {
  btnDanger,
  btnDangerSolid,
  btnNeutral,
  btnPrimary,
  hintClass,
  inputClass,
  labelClass,
} from "../channels/styles";
import {
  channelName,
  createRule,
  deleteRule,
  reorderRules,
  saveRule,
  type ChannelBrief,
  type Rule,
} from "./api";
import { ArchiveMoveNotice } from "./ArchiveMoveNotice";
import { EpisodeGrounds } from "./EpisodeGrounds";
import { BLANK_DRAFT, draftOf, fieldsOf, parseEpisode, sameDraft, type Draft } from "./draft";
import { LinkToSchedule } from "./LinkToSchedule";
import { ChannelTag, StateBadge } from "./RuleList";
import { ConflictNotice, OrderRow, RuleSummary } from "./parts";
import { RulePreview, type PastReceive } from "./RulePreview";
import { SwitchRows } from "./SwitchRows";
import { WaitingBanner } from "./WaitingBanner";
import { useArchiveMove } from "./useArchiveMove";
import { usePreview } from "./usePreview";

interface RuleDetailProps {
  /** The stored rule to edit, or `null` for a rule that is not saved yet. */
  rule: Rule | null;
  channels: ChannelBrief[];
  /** The app's collect folder, `null` until chosen. */
  collect_folder: string | null;
  /** Every rule of every channel, as of the last load. */
  rules: Rule[];
  /** For a new rule: the channel to start in and the phrase to start with. */
  presetChannelId?: string | null;
  presetMatch?: string;
  /**
   * Called after a write that kept the rule (`saved` is the rule as stored, when
   * known), so the list can take it and be read again. A delete calls
   * `onDeleted` instead.
   */
  onChanged: (saved?: Rule) => void;
  onCreated: (rule: Rule) => void;
  onDeleted: (rule: Rule) => void;
  /** Leaves the detail (the phone's way back to the list). */
  onBack: () => void;
  onDirtyChange: (dirty: boolean) => void;
}

function messageOf(error: unknown): string {
  return error instanceof ApiError ? error.message : "저장하지 못했어요. 잠시 뒤 다시 시도해 주세요.";
}

const checkRow = "flex min-h-6 cursor-pointer items-center gap-2 text-[13.5px]";

/**
 * One rule, in place: the fields, its place in the channel's check order, the
 * preview of what it would do, and (only while something is changed) the save
 * bar. It keeps what was typed through failures; a conflict shows the server's
 * value beside the input and the next save goes against that version.
 */
export function RuleDetail({
  rule,
  channels,
  collect_folder,
  rules,
  presetChannelId,
  presetMatch,
  onChanged,
  onCreated,
  onDeleted,
  onBack,
  onDirtyChange,
}: RuleDetailProps) {
  const uid = useId();
  const isNew = rule === null;

  // What the server last told us about this rule.
  const [known, setKnown] = useState<Rule | null>(rule);
  const baseline = useMemo(() => (known ? draftOf(known) : BLANK_DRAFT), [known]);
  const [draft, setDraft] = useState<Draft>(() =>
    rule ? draftOf(rule) : { ...BLANK_DRAFT, match: presetMatch ?? "" },
  );
  const [channelId, setChannelId] = useState<string>(
    () => rule?.channel_id ?? channels.find((c) => c.id === presetChannelId)?.id ?? channels[0]?.id ?? "",
  );

  const channel = channels.find((c) => c.id === channelId);
  // The channel's rules in check order, this one included (a new one goes last).
  const siblings = useMemo(() => {
    const own = rules.filter((r) => r.channel_id === channelId).sort((a, b) => a.order - b.order);
    return own;
  }, [rules, channelId]);
  const startPosition = known ? Math.max(0, siblings.findIndex((r) => r.id === known.id)) : siblings.length;
  const [basePosition, setBasePosition] = useState(startPosition);
  const [position, setPosition] = useState(startPosition);

  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [conflict, setConflict] = useState<Rule | null>(null);
  const [orderConflict, setOrderConflict] = useState<Rule[] | null>(null);
  const [confirmingDelete, setConfirmingDelete] = useState(false);
  const [creatorOpen, setCreatorOpen] = useState(false);
  const [linkOpen, setLinkOpen] = useState(false);
  const confirmRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (confirmingDelete) confirmRef.current?.focus();
  }, [confirmingDelete]);

  const episodeValue = parseEpisode(draft.episode);
  const episodeOk = episodeValue !== null;
  const fields = fieldsOf(draft, known?.episode ?? 1);
  const fieldsDirty = !sameDraft(draft, baseline);
  const positionDirty = !isNew && position !== basePosition;
  const dirty = fieldsDirty || positionDirty;

  // The list was cached when this rule was opened and has been read again since
  // (the worker moves `episode` on its own): follow a newer version unless the
  // user has edits, in which case a save answers with the conflict.
  const storedVersion = rule?.version;
  useEffect(() => {
    if (!rule || !known || rule.version <= known.version || dirty) return;
    setKnown(rule);
    setDraft(draftOf(rule));
    // Only when the list brings a newer version.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [storedVersion]);

  // The state is the worker's to change (`보관`·`복원` below), never typed: the
  // draft follows the rule's stored state however the rule was last read.
  const knownState = known?.state;
  useEffect(() => {
    if (knownState) setDraft((d) => (d.state === knownState ? d : { ...d, state: knownState }));
  }, [knownState]);

  useEffect(() => {
    onDirtyChange(dirty);
  }, [dirty, onDirtyChange]);
  useEffect(() => () => onDirtyChange(false), [onDirtyChange]);

  // `받기` on a past item row receives it with the stored rule. Once an item is
  // added the preview is asked again, so its row reads as received.
  const received = useReceive(known?.id ?? null);
  const addedCount = received.entries.filter((e) => e.phase.kind === "added").length;
  const preview = usePreview(channelId, known?.id ?? null, fields, position, addedCount);
  const pastReceive: PastReceive | undefined = known
    ? {
        phaseOf: (itemId) => received.entries.find((e) => e.itemId === itemId)?.phase,
        onReceive: received.one,
        // The worker receives with the stored rule, so the row must be what is stored.
        blocked:
          known.state === "archived"
            ? "보관된 규칙이에요. 복원한 뒤에 받을 수 있어요."
            : known.state === "paused"
              ? "영상 받기를 켠 뒤에 받을 수 있어요."
              : dirty
                ? "저장하지 않은 변경이 있어요. 저장한 뒤에 받을 수 있어요."
                : null,
      }
    : undefined;
  const latest =
    preview.state === "ready" ? preview.preview : preview.state === "loading" ? preview.previous : null;
  const regexProblem = latest?.error ?? null;
  const canSave = dirty && !busy && episodeOk && channel !== undefined && regexProblem === null;

  const set = <K extends keyof Draft>(key: K, value: Draft[K]) => setDraft((d) => ({ ...d, [key]: value }));

  const fail = (e: unknown, after: "fields" | "order") => {
    if (e instanceof ApiError && e.code === "conflict" && e.current) {
      if (after === "fields") {
        const current = e.current as Rule;
        setConflict(current);
        setKnown(current);
      } else {
        const current = (e.current as { rules: Rule[] }).rules;
        setOrderConflict(current);
        const mine = current.find((r) => r.id === known?.id);
        if (mine) setKnown(mine);
      }
      setError(null);
    } else {
      setConflict(null);
      setError(messageOf(e));
    }
  };

  const save = async (event?: FormEvent) => {
    event?.preventDefault();
    if (!canSave) return;
    setBusy(true);
    setError(null);
    setOrderConflict(null);
    let current = known;
    let stage: "fields" | "order" = "fields";
    try {
      if (fieldsDirty) {
        if (known) {
          current = await saveRule(known, fields);
          setKnown(current);
        } else {
          const created = await createRule(channelId, fields);
          onCreated(created);
          return;
        }
        setConflict(null);
      }
      if (positionDirty && current) {
        stage = "order";
        const others = siblings.filter((r) => r.id !== current!.id);
        others.splice(position, 0, current);
        const updated = await reorderRules(
          channelId,
          others.map((r) => ({ id: r.id, version: r.id === current!.id ? current!.version : r.version })),
        );
        current = updated.find((r) => r.id === current!.id) ?? current;
        setKnown(current);
        setBasePosition(position);
      }
      onChanged(current ?? undefined);
      setBusy(false);
    } catch (e) {
      fail(e, stage);
      // A save of the fields that went through stays saved when the order fails.
      if (stage === "order") onChanged(current ?? undefined);
      setBusy(false);
    }
  };

  /**
   * Archive or restore through the worker, leaving the other unsaved edits in
   * place. The rule as the worker left it comes back when the command ends.
   */
  const move = useArchiveMove(known, (fresh) => {
    setKnown(fresh);
    setConflict(null);
    onChanged(fresh);
    if (fresh.state === "archived") {
      // An archived rule is no longer suggested, and the subscription cards no longer show it.
      suggestionGone(fresh.id);
      subscriptionChanged();
    }
  });
  const movingDirection =
    move.phase.kind === "sending" || move.phase.kind === "waiting" || move.phase.kind === "unconfirmed"
      ? move.phase.direction
      : null;
  const moving = movingDirection !== null;

  /** A switch, the creator or `편성표와 연결` changed the rule: take it, the list and the other screens. */
  const taken = (fresh: Rule) => {
    setKnown(fresh);
    setConflict(null);
    onChanged(fresh);
    subscriptionChanged();
  };

  const remove = async () => {
    if (!known || busy) return;
    setBusy(true);
    setError(null);
    try {
      await deleteRule(known);
      onDeleted(known);
    } catch (e) {
      setConfirmingDelete(false);
      fail(e, "fields");
      setBusy(false);
    }
  };

  const revert = () => {
    setDraft(baseline);
    setPosition(basePosition);
    setConflict(null);
    setOrderConflict(null);
    setError(null);
  };

  const matchTitle = draft.match !== "" ? draft.match : isNew ? "새 규칙" : "제목 대기";
  // A subscription is named by its anime.
  const anime = known?.subscription?.anime ?? null;
  const heading = anime?.subject ?? matchTitle;
  const collectFolder = collect_folder;
  const joined = `${collectFolder?.replace(/\/+$/, "") ?? ""}/${draft.directory.trim().replace(/^\/+/, "")}`.replace(/\/+$/, "");

  return (
    <div className="flex min-w-0 flex-col gap-5" data-testid="rule-detail">
      <Button
        type="button"
        variant="ghost"
        className={`${btnNeutral} self-start min-[721px]:hidden`}
        onClick={onBack}
      >
        ← 규칙 목록
      </Button>

      <div className="grid grid-cols-[104px_minmax(0,1fr)] gap-x-4 gap-y-3.5 max-[720px]:grid-cols-[88px_minmax(0,1fr)]">
        <div className="col-start-2 row-start-1 flex min-w-0 flex-col gap-2 max-[720px]:col-span-2 max-[720px]:col-start-1">
          <div className="flex min-w-0 flex-wrap items-center gap-1.5">
            <StateBadge rule={{ state: draft.state, match: draft.match }} />
            <ChannelTag channel={channel} />
            {known?.overlap && known.state === "active" && (
              <span className="inline-flex items-center rounded-full border border-hairline px-2 py-px text-[11.5px] font-semibold text-text-secondary">
                겹침
              </span>
            )}
          </div>
          <h2 className="min-w-0 text-xl leading-snug font-bold break-all" data-testid="rule-heading">
            {heading}
          </h2>
          {anime?.original_subject && (
            <p lang="ja" className="min-w-0 text-[13px] leading-snug break-words text-text-secondary">
              {anime.original_subject}
            </p>
          )}
        </div>
        <RuleSummary
          rule={known}
          creatorOpen={creatorOpen}
          linkOpen={linkOpen}
          onChangeCreator={() => {
            setLinkOpen(false);
            setCreatorOpen((open) => !open);
          }}
          onLink={() => {
            setCreatorOpen(false);
            setLinkOpen((open) => !open);
          }}
        />
      </div>

      {known && creatorOpen && known.subscription && (
        <CreatorPicker
          key={`${known.version}:${known.subscription.creator ?? ""}`}
          animeNo={known.subscription.anissia_anime_no}
          current={known.subscription.creator}
          onCancel={() => setCreatorOpen(false)}
          onApply={async (creator) => {
            try {
              taken(await changeCreator(known, creator));
              setCreatorOpen(false);
            } catch (e) {
              if (e instanceof ApiError && e.code === "conflict" && e.current) {
                taken(e.current as Rule);
                throw new Error("다른 곳에서 먼저 바꿨어요. 지금 상태를 보여드려요. 다시 골라 주세요.");
              }
              throw e;
            }
          }}
        />
      )}
      {known && linkOpen && known.subscription === null && (
        <LinkToSchedule
          rule={known}
          onCancel={() => setLinkOpen(false)}
          onStale={taken}
          onLinked={(linked) => {
            taken(linked);
            forget(KEYS.rules);
            setLinkOpen(false);
          }}
        />
      )}

      {known && (
        <SwitchRows
          rule={known}
          disabled={busy || moving}
          onChanged={(fresh) => {
            taken(fresh);
          }}
        />
      )}

      {known?.state === "paused" && (
        <p
          role="status"
          data-testid="paused-banner"
          className="rounded-xl border border-hairline bg-surface-2 px-3.5 py-3 text-[13px] leading-normal text-text-secondary"
        >
          영상 받기를 꺼서 멈춰 있어요. 새 항목을 받지 않고 작품 폴더는 그대로 두어요. 다시 켜면 다음 RSS 확인부터 새로 올라오는 항목을 받아요. 멈춘 동안 올라온 항목은 지난 회차로 남아서 직접 골라 받아요.
        </p>
      )}
      {known?.season_blocked && (
        <p
          role="status"
          data-testid="season-blocked"
          className="rounded-xl border border-hairline bg-surface-2 px-3.5 py-3 text-[13px] leading-normal text-text-secondary"
        >
          이 규칙이 받은 영상은 ‘{known.season_blocked.work_name ?? "작품"}’ 시즌 {known.season_blocked.number}에 있어요. 그 시즌은 이미{" "}
          {known.season_blocked.holder_subject ? `‘${known.season_blocked.holder_subject}’` : "다른 Anissia 작품"}에 이어져 있어서 이 규칙을
          잇지 않았어요. 그 구독이 사라지면 다음 확인에서 다시 이어 봐요.
        </p>
      )}

      {known?.state === "archived" && (
        <p className="rounded-xl border border-hairline bg-surface-2 px-3.5 py-3 text-[13px] leading-normal text-text-secondary">
          이 규칙은 보관했어요. 새 항목을 받지 않고, 수집 기록은 그대로예요. 복원하면 작품 폴더를 수집 폴더로 되돌린 뒤 다음 RSS 확인부터 새 항목을 받아요. 보관된 동안 올라온 항목은 지난 회차로 남아요.
        </p>
      )}
      {known && known.state !== "archived" && (
        <ArchiveBanner ruleId={known.id} busy={busy || moving} onArchive={() => move.submit("archive")} />
      )}
      {known && (
        <ArchiveMoveNotice
          rule={known}
          phase={move.phase}
          onMoveAgain={() => move.submit("archive")}
          onResend={move.resend}
          onRecheck={move.recheck}
        />
      )}
      {known && known.state !== "archived" && known.match === null && known.subscription !== null && (
        <WaitingBanner rule={known} />
      )}
      {known && known.state === "active" && known.match === null && known.subscription === null && (
        <p className="rounded-xl border border-hairline bg-surface-2 px-3.5 py-3 text-[13px] leading-normal text-text-secondary">
          일치 문구를 적기 전까지는 아무것도 받지 않아요.
        </p>
      )}

      <form onSubmit={save} className="flex min-w-0 flex-col gap-4" noValidate aria-label="규칙 편집">
        {isNew && (
          <div className="flex min-w-0 flex-col gap-1.5">
            <Label htmlFor={`${uid}-channel`} className={labelClass}>
              채널
            </Label>
            <select
              id={`${uid}-channel`}
              value={channelId}
              onChange={(e) => {
                setChannelId(e.target.value);
                setPosition(rules.filter((r) => r.channel_id === e.target.value).length);
              }}
              className={`${inputClass} w-full`}
            >
              {channels.map((c) => (
                <option key={c.id} value={c.id}>
                  {channelName(c)}
                </option>
              ))}
            </select>
          </div>
        )}

        <div className="flex min-w-0 flex-col gap-1.5">
          <Label htmlFor={`${uid}-match`} className={labelClass}>
            일치 문구
          </Label>
          <Input
            id={`${uid}-match`}
            value={draft.match}
            onChange={(e) => set("match", e.target.value)}
            className={`${inputClass} ${draft.regex ? "font-mono" : ""}`}
            placeholder={draft.regex ? "^\\[SubsPlease\\] 작품 제목 - \\d+" : "[SubsPlease] 작품 제목"}
            autoComplete="off"
            autoCapitalize="none"
            spellCheck={false}
            aria-invalid={regexProblem !== null}
            aria-describedby={`${uid}-match-hint`}
          />
          <p id={`${uid}-match-hint`} className={hintClass}>
            {draft.regex
              ? "정규식으로 읽어서 릴리스 제목에 맞는지 봐요."
              : "릴리스 제목에 이 문구가 들어 있으면 맞아요. 비워 두면 제목을 정하기 전까지 아무것도 받지 않아요."}
          </p>
        </div>

        <div className="flex flex-wrap gap-x-6 gap-y-2">
          <label className={checkRow}>
            <input
              type="checkbox"
              checked={draft.regex}
              onChange={(e) => set("regex", e.target.checked)}
              className="size-[18px] flex-none accent-focus"
            />
            <span className="font-semibold">정규식</span>
          </label>
          <label className={checkRow}>
            <input
              type="checkbox"
              checked={draft.case_insensitive}
              onChange={(e) => set("case_insensitive", e.target.checked)}
              className="size-[18px] flex-none accent-focus"
            />
            <span className="font-semibold">대소문자 무시</span>
          </label>
        </div>

        <div className="grid min-w-0 grid-cols-[minmax(0,1fr)_150px] gap-3.5 max-[720px]:grid-cols-1">
          <div className="flex min-w-0 flex-col gap-1.5">
            <Label htmlFor={`${uid}-dir`} className={labelClass}>
              저장 폴더
            </Label>
            <Input
              id={`${uid}-dir`}
              value={draft.directory}
              onChange={(e) => set("directory", e.target.value)}
              className={inputClass}
              placeholder="작품 제목/Season 01"
              autoComplete="off"
              spellCheck={false}
              aria-describedby={`${uid}-dir-hint`}
            />
            <p id={`${uid}-dir-hint`} className={hintClass}>
              앱의 수집 폴더 아래에 저장해요.{" "}
              {collectFolder ? (
                <span className="font-mono break-all">→ {joined}</span>
              ) : (
                <>
                  수집 폴더를 정하면 받기 시작해요.{" "}
                  <Link to="/settings/collection" className="font-semibold text-focus underline underline-offset-2">
                    수집 폴더 정하기
                  </Link>
                </>
              )}
            </p>
          </div>
          <div className="flex min-w-0 flex-col gap-1.5">
            <Label htmlFor={`${uid}-ep`} className={labelClass}>
              회차 변환
              {known?.episode_auto && parseEpisode(draft.episode) === known.episode && (
                <span className="ml-1.5 rounded-full border border-hairline px-1.5 py-px text-[11px] font-semibold text-text-secondary">
                  자동
                </span>
              )}
            </Label>
            <Input
              id={`${uid}-ep`}
              value={draft.episode}
              onChange={(e) => set("episode", e.target.value)}
              className={`${inputClass} font-mono`}
              inputMode="numeric"
              autoComplete="off"
              aria-invalid={!episodeOk}
              aria-describedby={`${uid}-ep-hint`}
            />
            <p id={`${uid}-ep-hint`} className={episodeOk ? hintClass : `${hintClass} font-semibold text-urgent`}>
              {episodeOk ? "릴리스 회차를 시즌 회차로 바꾸는 값이에요." : "정수로 적어 주세요."}
            </p>
          </div>
        </div>
        {known && (
          <EpisodeGrounds
            rule={known}
            shown={episodeValue === known.episode}
            disabled={busy || moving}
            onChanged={(fresh) => {
              taken(fresh);
              // The other edits stay; the field takes the stored offset.
              setDraft((d) => ({ ...d, episode: String(fresh.episode) }));
            }}
          />
        )}

        {!isNew && siblings.length > 1 && (
          <OrderRow siblings={siblings} position={position} onMove={setPosition} />
        )}

        {conflict && <ConflictNotice current={conflict} draft={draft} />}
        {orderConflict && (
          <section role="alert" className="flex min-w-0 flex-col gap-2 rounded-xl border border-hairline bg-surface-2 p-3.5">
            <p className="text-sm font-bold">다른 곳에서 먼저 순서를 바꿨어요.</p>
            <p className="text-[13px] leading-normal text-text-secondary">
              입력한 값은 그대로 두었어요. 지금 이 채널의 검사 순서는 아래와 같아요. 확인하고 다시 저장할 수 있어요.
            </p>
            <ol className="m-0 flex list-decimal flex-col gap-0.5 pl-5 text-[13px]">
              {orderConflict.map((r) => (
                <li key={r.id} className={r.id === known?.id ? "font-bold" : undefined}>
                  <span className="break-all">{r.match ?? "제목 대기"}</span>
                </li>
              ))}
            </ol>
          </section>
        )}
        {error && (
          <p role="alert" className="text-[13px] font-semibold text-urgent">
            {error}
          </p>
        )}

        <RulePreview state={preview} receive={pastReceive} />

        {dirty && (
          <div
            role="group"
            aria-label="저장"
            data-testid="save-bar"
            className="sticky bottom-0 z-30 -mx-1 flex flex-wrap items-center gap-2 rounded-xl border border-hairline bg-surface-1 px-3 py-2.5 shadow-[0_-6px_18px_rgba(0,0,0,0.12)] max-[720px]:bottom-[calc(var(--bnav-h)+env(safe-area-inset-bottom,0px)+8px)]"
          >
            <Button type="submit" variant="ghost" className={btnPrimary} disabled={!canSave}>
              {busy ? "저장 중" : isNew ? "규칙 만들기" : "저장"}
            </Button>
            <Button type="button" variant="ghost" className={btnNeutral} disabled={busy} onClick={revert}>
              되돌리기
            </Button>
            <span className="min-w-0 flex-1 basis-40 text-xs text-text-muted">
              {regexProblem
                ? "정규식을 고쳐야 저장할 수 있어요."
                : !episodeOk
                  ? "회차 변환을 정수로 적어야 저장할 수 있어요."
                  : "저장하지 않은 변경이 있어요."}
            </span>
          </div>
        )}
      </form>

      {known && (
        <div className="flex flex-col gap-2.5 border-t border-hairline-soft pt-4" data-testid="manage-row">
          {confirmingDelete ? (
            <div
              ref={confirmRef}
              tabIndex={-1}
              role="group"
              aria-labelledby={`${uid}-del`}
              className="flex flex-col gap-2.5 rounded-xl border border-[color-mix(in_srgb,var(--accent-urgent)_50%,transparent)] p-3.5 outline-offset-2"
            >
              <p id={`${uid}-del`} className="text-sm font-bold">
                이 규칙을 삭제할까요?
              </p>
              <p className="text-[13px] leading-normal text-text-secondary">
                이미 받은 파일과 수집 기록은 지우지 않아요. 규칙은 되돌릴 수 없어요. 새 항목을 받지 않기만 원하면 보관을 써요.
              </p>
              <div className="flex flex-wrap gap-2">
                <Button type="button" variant="ghost" className={btnDangerSolid} disabled={busy} onClick={remove}>
                  {busy ? "삭제 중" : "삭제"}
                </Button>
                <Button
                  type="button"
                  variant="ghost"
                  className={btnNeutral}
                  disabled={busy}
                  onClick={() => setConfirmingDelete(false)}
                >
                  취소
                </Button>
              </div>
            </div>
          ) : (
            <div className="flex flex-col gap-2">
              {move.phase.kind === "idle" && move.phase.error !== null && (
                <p role="alert" className="text-[13px] font-semibold text-urgent">
                  {move.phase.error}
                </p>
              )}
              <div className="flex flex-wrap items-center gap-2">
                <Button
                  type="button"
                  variant="ghost"
                  className={btnNeutral}
                  disabled={busy || moving}
                  onClick={() => move.submit(known.state === "archived" ? "restore" : "archive")}
                >
                  {movingDirection === "restore"
                    ? "복원하는 중"
                    : movingDirection === "archive"
                      ? "보관하는 중"
                      : known.state === "archived"
                        ? "복원"
                        : "보관"}
                </Button>
                {known.state !== "archived" && !moving && (
                  <span className="min-w-0 flex-1 basis-48 text-xs text-text-muted">
                    보관하면 새 항목을 받지 않고, 보관 폴더를 정해 두었으면 작품 폴더를 그리로 옮겨요.
                  </span>
                )}
                <Button
                  type="button"
                  variant="ghost"
                  className={`${btnDanger} ml-auto`}
                  disabled={busy || moving}
                  onClick={() => setConfirmingDelete(true)}
                >
                  삭제
                </Button>
              </div>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
