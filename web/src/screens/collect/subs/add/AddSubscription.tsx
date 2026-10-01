import { useState } from "react";
import { Link, useSearchParams } from "react-router-dom";

import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

import { btnAction, btnNeutral } from "../../channels/styles";
import { todayWeek } from "../format";
import { Confirm } from "./Confirm";
import { PickAnime } from "./PickAnime";
import { PickChannel } from "./PickChannel";
import { PickFolder } from "./PickFolder";
import { PickSubtitles } from "./PickSubtitles";
import { PickTitle } from "./PickTitle";
import { folderProblem, type Draft } from "./draft";

const STEPS = ["작품", "채널", "릴리스 제목", "자막", "저장 폴더", "확인"] as const;
const LAST = STEPS.length - 1;

/** What a step needs before the next one opens. */
function ready(step: number, draft: Draft): boolean {
  switch (step) {
    case 0:
      return draft.anime !== null;
    case 1:
      return draft.channel !== null;
    case 2:
      return draft.work !== null || draft.waiting;
    case 3:
      return draft.subtitles !== "follow" || (draft.creator !== null && draft.creator !== "");
    case 4:
      return folderProblem(draft.directory) === null;
    default:
      return false;
  }
}

/** The schedule tab to open on: the one the link names, else today's weekday. */
function startWeek(param: string | null): number {
  const week = param === null || param === "" ? NaN : Number(param);
  return Number.isInteger(week) && week >= 0 && week <= 8 ? week : todayWeek();
}

/**
 * The subscribe flow at `/collect/subs/add`: the anime from the schedule, the
 * channel, the release title from that channel's history, the subtitle choice,
 * the save folder, then the past items to receive. Nothing is created before
 * the last step's button; going back keeps what was chosen, and a change
 * clears only the choices that depended on it.
 */
export function AddSubscription() {
  const [step, setStep] = useState(0);
  // `?week=8` opens the schedule on `신작`, where the next quarter's works are.
  const [params] = useSearchParams();
  const [draft, setDraft] = useState<Draft>(() => ({
    week: startWeek(params.get("week")),
    anime: null,
    channel: null,
    work: null,
    waiting: false,
    subtitles: "undecided",
    creator: null,
    directory: "",
  }));
  // Once the rule exists the flow cannot be walked back: the rest is about it.
  const [created, setCreated] = useState(false);

  const change = (next: Partial<Draft>) => setDraft((prev) => ({ ...prev, ...next }));
  const go = (to: number) => {
    setStep(to);
    window.scrollTo({ top: 0 });
  };

  return (
    <div className="flex min-w-0 flex-col gap-4">
      <div className="flex flex-wrap items-center justify-between gap-x-4 gap-y-2.5">
        <h2 className="text-[17px] font-bold">편성표에서 구독</h2>
        {!created && (
          <Button asChild type="button" variant="ghost" className={btnNeutral}>
            <Link to="/collect/subs">취소</Link>
          </Button>
        )}
      </div>

      <nav aria-label="구독 단계">
        <ol className="m-0 flex list-none items-center gap-1.5 p-0 text-[12.5px]">
          {STEPS.map((name, index) => (
            <li
              key={name}
              aria-current={index === step ? "step" : undefined}
              className={cn(
                "flex min-w-0 items-center gap-1.5 rounded-full border px-2.5 py-1",
                index === step
                  ? "border-focus font-bold text-text-primary"
                  : "border-hairline-soft text-text-muted",
                index !== step && "max-[600px]:hidden",
              )}
            >
              <span className="font-mono">{index + 1}</span>
              <span className="min-w-0 truncate">{name}</span>
            </li>
          ))}
          <li className="ml-auto hidden text-text-muted max-[600px]:block" aria-hidden="true">
            {step + 1} / {STEPS.length}
          </li>
        </ol>
      </nav>

      <div className="min-w-0 rounded-card border border-hairline-soft bg-surface-1 p-[18px] shadow-(--card-shadow) max-[480px]:p-4">
        {step === 0 && (
          <PickAnime
            week={draft.week}
            selected={draft.anime}
            onWeek={(week) => change({ week })}
            onPick={(anime) => {
              // The subtitle choices come from the anime's captions.
              if (anime.anime_no !== draft.anime?.anime_no) {
                // A channel that already follows this anime cannot be kept.
                const taken = anime.subscribed_rules.some((r) => r.channel_id === draft.channel?.id);
                change({
                  anime,
                  subtitles: "undecided",
                  creator: null,
                  ...(taken ? { channel: null, work: null, waiting: false, directory: "" } : {}),
                });
              }
              go(1);
            }}
          />
        )}
        {step === 1 && draft.anime && (
          <PickChannel
            anime={draft.anime}
            selected={draft.channel}
            onPick={(channel) => {
              if (channel.id !== draft.channel?.id) change({ channel, work: null, waiting: false, directory: "" });
              go(2);
            }}
          />
        )}
        {step === 2 && draft.channel && (
          <PickTitle
            channel={draft.channel}
            selected={draft.work}
            waiting={draft.waiting}
            onPick={(work) => {
              if (draft.waiting || work.work !== draft.work?.work) {
                change({ work, waiting: false, directory: work.folder ?? "" });
              }
              go(3);
            }}
            onPickWaiting={() => {
              // No release title, so no suggested folder: a folder made for another pick is dropped.
              if (!draft.waiting) change({ work: null, waiting: true, directory: "" });
              go(3);
            }}
          />
        )}
        {step === 3 && draft.anime && (
          <PickSubtitles
            anime={draft.anime}
            subtitles={draft.subtitles}
            creator={draft.creator}
            onChange={(subtitles, creator) => change({ subtitles, creator })}
          />
        )}
        {step === 4 && (
          <PickFolder
            directory={draft.directory}
            waiting={draft.waiting}
            onChange={(directory) => change({ directory })}
          />
        )}
        {step === LAST && draft.anime && draft.channel && (draft.work || draft.waiting) && (
          <Confirm draft={draft} anime={draft.anime} channel={draft.channel} work={draft.work} onCreated={() => setCreated(true)} />
        )}
      </div>

      {step < LAST && (
        <div className="flex flex-wrap items-center justify-between gap-2.5">
          <Button
            type="button"
            variant="ghost"
            className={btnNeutral}
            disabled={step === 0}
            onClick={() => go(step - 1)}
          >
            이전
          </Button>
          {/* The first three steps move on when something is picked; this is for coming back to them. */}
          <Button
            type="button"
            variant="ghost"
            className={btnAction}
            disabled={!ready(step, draft)}
            onClick={() => go(step + 1)}
          >
            다음
          </Button>
        </div>
      )}
      {step === LAST && !created && (
        <div className="flex">
          <Button type="button" variant="ghost" className={btnNeutral} onClick={() => go(step - 1)}>
            이전
          </Button>
        </div>
      )}
    </div>
  );
}
