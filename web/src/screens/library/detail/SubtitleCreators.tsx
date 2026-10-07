import { useId, useState } from "react";

import { Button } from "@/components/ui/button";
import { ApiError } from "@/lib/api";
import { useCached } from "@/lib/cached";

import { btnAction, btnNeutral } from "../../collect/channels/styles";
import { changeCreator, fetchCreators, type Creators } from "../../collect/subs/api";
import { CreatorPicker } from "../../collect/subs/CreatorPicker";
import { subtitleChoice } from "../../collect/subs/format";
import { nameUnknownCreators, setFileCreator, type WorkSeason, type WorkSubscription, type WorkSubtitle } from "../api";
import { creatorOf, headNames, isNameable, unknownFiles, UNKNOWN } from "./fileCreators";
import { baseName } from "./model";

/**
 * The creator of a season's subtitle files (`library.md`, 머리와 시즌): a file found in a watch folder is by
 * `제작자 알 수 없음` until the user names one of the creators of the season's Anissia anime. The head names one for all
 * the season's unknown files at once; an expanded episode row changes one file's. Neither moves or receives a file.
 */

const radio = "mt-0.5 size-[18px] flex-none accent-focus";
const option =
  "flex min-w-0 cursor-pointer items-start gap-3 rounded-[10px] border border-hairline-soft bg-surface-2 px-3.5 py-2.5 has-[:checked]:border-focus has-[:focus-visible]:outline-2 has-[:focus-visible]:outline-offset-2 has-[:focus-visible]:outline-focus";
const textButton =
  "inline-flex min-h-6 items-center rounded-md px-1.5 text-[12.5px] font-semibold text-focus underline underline-offset-2 outline-offset-2 hover:bg-surface-2 focus-visible:outline-2 focus-visible:outline-focus max-[720px]:min-h-9";

/** The subtitle files of a season, in the episodes' order. */
function subtitlesOf(season: WorkSeason): WorkSubtitle[] {
  return season.episodes.flatMap((e) => e.subtitle);
}

/**
 * One of the creators Anissia lists for the anime. `current` is the creator now (`null`: `제작자 알 수 없음`;
 * `undefined`: none is chosen yet, as in the head). `onApply` sends the change and throws the sentence to show when
 * it fails.
 */
function SubtitleCreatorPicker({
  animeNo,
  legend,
  note,
  current,
  allowUnknown,
  applyLabel,
  onApply,
  onCancel,
}: {
  animeNo: number;
  legend: string;
  note?: string;
  current: string | null | undefined;
  allowUnknown: boolean;
  applyLabel: string;
  onApply: (creator: string | null) => Promise<void>;
  onCancel: () => void;
}) {
  const uid = useId();
  const list = useCached<Creators>(
    `collect:creators:${animeNo}`,
    (signal) => fetchCreators(animeNo, signal),
    "자막 제작자를 불러오지 못했어요.",
  );
  const [picked, setPicked] = useState<string | null | undefined>(current);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const names = list.data?.creators.map((c) => c.name) ?? [];
  // The creator now stays a choice even when Anissia no longer lists it.
  const choices = typeof current === "string" && !names.includes(current) ? [current, ...names] : names;

  const apply = async () => {
    if (picked === undefined) return;
    setBusy(true);
    setError(null);
    try {
      await onApply(picked);
    } catch (e) {
      setError(e instanceof Error ? e.message : "바꾸지 못했어요. 잠시 뒤 다시 시도해 주세요.");
      setBusy(false);
    }
  };

  return (
    <fieldset className="m-0 flex min-w-0 flex-col gap-2 border-0 p-0" data-testid="subtitle-creator-picker">
      <legend className="mb-1 p-0 text-[13px] font-bold">{legend}</legend>
      {note && <p className="m-0 text-xs leading-[1.45] text-text-muted">{note}</p>}

      {list.data === undefined && list.error === null && list.slow && (
        <p className="text-[13px] text-text-muted">자막 제작자를 불러오는 중이에요.</p>
      )}
      {list.error !== null && (
        <div className="flex flex-wrap items-center gap-2.5">
          <p role="alert" className="text-[13px] leading-normal font-semibold text-urgent">
            {list.error}
          </p>
          <Button type="button" variant="ghost" className={btnNeutral} onClick={list.reload}>
            재시도
          </Button>
        </div>
      )}
      {list.data !== undefined && choices.length === 0 && (
        <p className="text-[13px] leading-normal text-text-muted">Anissia에 등록된 자막 제작자가 없어요.</p>
      )}

      {choices.map((name) => (
        <label key={name} className={option}>
          <input
            type="radio"
            name={`${uid}-creator`}
            className={radio}
            checked={picked === name}
            onChange={() => setPicked(name)}
          />
          <span className="min-w-0 text-[14px] font-semibold break-words">{name}</span>
        </label>
      ))}
      {allowUnknown && (
        <label className={option}>
          <input
            type="radio"
            name={`${uid}-creator`}
            className={radio}
            checked={picked === null}
            onChange={() => setPicked(null)}
          />
          <span className="text-[14px] font-semibold">{UNKNOWN}</span>
        </label>
      )}

      {error && (
        <p role="alert" className="text-[13px] font-semibold text-urgent">
          {error}
        </p>
      )}
      <div className="flex flex-wrap gap-2">
        <Button
          type="button"
          variant="ghost"
          className={btnAction}
          disabled={busy || picked === undefined || picked === current}
          onClick={apply}
        >
          {busy ? "바꾸는 중" : applyLabel}
        </Button>
        <Button type="button" variant="ghost" className={btnNeutral} disabled={busy} onClick={onCancel}>
          취소
        </Button>
      </div>
    </fieldset>
  );
}

/**
 * The head's `자막 제작자` line for the chosen season: the subscription's choice (changed with `제작자 변경`), every
 * creator named for a subtitle file, and `제작자 알 수 없음` while some file has none. While files have none it offers
 * `제작자 지정`, which names one creator for all of them; a season with no Anissia link says to link it first.
 */
export function HeadCreators({
  workId,
  season,
  subscription,
  onSubscriptionChanged,
  onNamed,
}: {
  workId: string;
  season: WorkSeason;
  subscription: WorkSubscription | undefined;
  onSubscriptionChanged: () => void;
  /** The files got a creator: the work (and the candidates, which may be revisions now) are read again. */
  onNamed: () => void;
}) {
  const [creatorOpen, setCreatorOpen] = useState(false);
  const [namingOpen, setNamingOpen] = useState(false);
  const [named, setNamed] = useState<string | null>(null);
  const files = subtitlesOf(season);
  const unknown = unknownFiles(files).length;
  const animeNo = season.anissia.anime?.anime_no ?? null;
  const names = headNames(
    files,
    subscription && {
      text: subtitleChoice(subscription),
      followed: subscription.subtitles === "follow" ? subscription.creator : null,
    },
  );

  return (
    <div className="mt-3 flex min-w-0 flex-col gap-2.5" data-testid="head-creator">
      <p className="flex flex-wrap items-center gap-x-2 gap-y-1 text-[13px]">
        <span className="font-semibold text-text-muted">자막 제작자</span>
        <span className="font-medium text-text-secondary">{names.join(", ")}</span>
        {subscription && (
          <button
            type="button"
            aria-expanded={creatorOpen}
            disabled={subscription.subtitles === "none"}
            title={subscription.subtitles === "none" ? "자막 받기를 켠 뒤 바꿀 수 있어요" : undefined}
            onClick={() => setCreatorOpen((open) => !open)}
            className={`${textButton} disabled:cursor-not-allowed disabled:text-text-muted disabled:no-underline`}
          >
            제작자 변경
          </button>
        )}
        {unknown > 0 && animeNo !== null && (
          <button
            type="button"
            aria-expanded={namingOpen}
            onClick={() => {
              setNamed(null);
              setNamingOpen((open) => !open);
            }}
            className={textButton}
          >
            제작자 지정
          </button>
        )}
      </p>
      {named && (
        <p role="status" className="m-0 text-xs leading-[1.45] font-semibold text-text-secondary" data-testid="creator-named">
          {named}
        </p>
      )}
      {unknown > 0 && animeNo === null && (
        <p className="m-0 text-xs leading-[1.45] text-text-muted" data-testid="link-first">
          제작자를 모르는 자막이 {unknown}개 있어요. 아래에서 이 시즌에 Anissia 작품을 먼저 연결하면 그 작품의 제작자를 지정할 수 있어요.
        </p>
      )}
      {creatorOpen && subscription && (
        <CreatorPicker
          key={`${subscription.rule_version}:${subscription.creator ?? ""}`}
          animeNo={subscription.anime_no}
          current={subscription.creator}
          onCancel={() => setCreatorOpen(false)}
          onApply={async (creator) => {
            try {
              await changeCreator({ id: subscription.rule_id, version: subscription.rule_version }, creator);
            } catch (e) {
              if (e instanceof ApiError && e.code === "conflict") {
                onSubscriptionChanged();
                throw new Error("다른 곳에서 먼저 바꿨어요. 지금 상태를 보여드려요. 다시 골라 주세요.");
              }
              throw e;
            }
            onSubscriptionChanged();
            setCreatorOpen(false);
          }}
        />
      )}
      {namingOpen && unknown > 0 && animeNo !== null && (
        <SubtitleCreatorPicker
          animeNo={animeNo}
          legend="제작자를 모르는 자막의 제작자를 골라요"
          note={`제작자를 모르는 자막 ${unknown}개에 한 번에 붙여요. 이미 제작자가 있는 자막과 파일은 그대로 두고, 구독은 만들지 않아요.`}
          current={undefined}
          allowUnknown={false}
          applyLabel="지정하기"
          onCancel={() => setNamingOpen(false)}
          onApply={async (creator) => {
            if (creator === null) return;
            const result = await nameUnknownCreators(workId, season.number, creator);
            setNamed(
              result.attached > 0
                ? `자막 ${result.attached}개에 제작자를 붙였어요. 제작자: ${result.creator.name}`
                : "붙일 자막이 없었어요. 제작자를 모르던 자막에 다른 곳에서 이미 제작자를 붙였어요. 지금 상태를 보여드려요.",
            );
            setNamingOpen(false);
            onNamed();
          }}
        />
      )}
    </div>
  );
}

/** One subtitle file of an expanded episode row: its name, its creator and, with a linked anime, `제작자 바꾸기`. */
function SubtitleFile({
  workId,
  season,
  animeNo,
  file,
  onChanged,
}: {
  workId: string;
  season: number;
  animeNo: number | null;
  file: WorkSubtitle;
  onChanged: () => void;
}) {
  const [open, setOpen] = useState(false);
  const [note, setNote] = useState<string | null>(null);
  const name = baseName(file.path);
  const creator = creatorOf(file);
  return (
    <div className="flex min-w-0 flex-col gap-1" data-testid="subtitle-file">
      <span title={file.path} className="font-mono text-[12px] break-all">
        {name}
      </span>
      <span className="flex flex-wrap items-center gap-x-2 gap-y-0.5 text-xs">
        <span className="text-text-muted">제작자</span>
        <span className={creator !== null ? "font-semibold text-text-secondary" : "text-text-muted"}>
          {creator ?? UNKNOWN}
        </span>
        {animeNo !== null && isNameable(file) && (
          <button
            type="button"
            aria-expanded={open}
            onClick={() => {
              setNote(null);
              setOpen((o) => !o);
            }}
            className={textButton}
          >
            제작자 바꾸기<span className="sr-only"> ({name})</span>
          </button>
        )}
      </span>
      {note && (
        <p role="status" className="m-0 text-xs leading-[1.45] font-semibold text-urgent">
          {note}
        </p>
      )}
      {open && animeNo !== null && isNameable(file) && (
        <SubtitleCreatorPicker
          key={file.creator_version}
          animeNo={animeNo}
          legend={`${name}의 제작자를 골라요`}
          current={file.creator ? file.creator.name : null}
          allowUnknown
          applyLabel="바꾸기"
          onCancel={() => setOpen(false)}
          onApply={async (creator) => {
            try {
              await setFileCreator(workId, season, { path: file.path, version: file.creator_version }, creator);
            } catch (e) {
              if (e instanceof ApiError && e.code === "conflict") {
                // Another screen changed this file first: show what it is now and let the user choose again.
                setOpen(false);
                setNote("다른 곳에서 먼저 이 자막의 제작자를 바꿨어요. 지금 상태를 보여드려요. 다시 고르려면 제작자 바꾸기를 눌러 주세요.");
                onChanged();
                return;
              }
              throw e;
            }
            setOpen(false);
            onChanged();
          }}
        />
      )}
    </div>
  );
}

/** The `자막 파일` cell of an expanded episode row: each file with its creator. */
export function SubtitleFiles({
  workId,
  season,
  animeNo,
  files,
  onChanged,
}: {
  workId: string;
  season: number;
  animeNo: number | null;
  files: readonly WorkSubtitle[];
  onChanged: () => void;
}) {
  if (files.length === 0) return <span className="text-text-muted">없음</span>;
  return files.map((file) => (
    <SubtitleFile key={file.path} workId={workId} season={season} animeNo={animeNo} file={file} onChanged={onChanged} />
  ));
}
