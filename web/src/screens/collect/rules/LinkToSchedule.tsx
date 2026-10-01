import { useState } from "react";

import { Button } from "@/components/ui/button";
import { ApiError } from "@/lib/api";

import { btnNeutral, btnPrimary } from "../channels/styles";
import {
  linkToSchedule,
  type ScheduleEntry,
  type SubtitleMode,
} from "../subs/api";
import { PickAnime } from "../subs/add/PickAnime";
import { PickSubtitles } from "../subs/add/PickSubtitles";
import { todayWeek } from "../subs/format";
import type { Rule } from "./api";

/**
 * `편성표와 연결`: makes an existing rule a subscription. The anime comes from
 * Anissia's schedule and the subtitle choice from the same step the subscribe
 * flow has. The rule's match phrase, save folder and order are not touched.
 * `onStale` gets the rule as the server has it when someone else changed it
 * first.
 */
export function LinkToSchedule({
  rule,
  onLinked,
  onStale,
  onCancel,
}: {
  rule: Rule;
  onLinked: (rule: Rule) => void;
  onStale: (rule: Rule) => void;
  onCancel: () => void;
}) {
  const [week, setWeek] = useState(() => todayWeek());
  const [anime, setAnime] = useState<ScheduleEntry | null>(null);
  const [subtitles, setSubtitles] = useState<SubtitleMode>("undecided");
  const [creator, setCreator] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const ready = anime !== null && (subtitles !== "follow" || creator !== null);

  const link = async () => {
    if (anime === null || !ready) return;
    setBusy(true);
    setError(null);
    try {
      const linked = await linkToSchedule(rule, {
        anissia_anime_no: anime.anime_no,
        week: anime.week,
        subtitles,
        // The creator is only stored with a followed one.
        creator: subtitles === "follow" ? creator : null,
      });
      onLinked(linked);
    } catch (e) {
      if (e instanceof ApiError && e.code === "conflict" && e.current) {
        onStale(e.current as Rule);
        setError("다른 곳에서 먼저 바꿨어요. 지금 상태를 보여드려요. 다시 골라 연결해 주세요.");
      } else {
        setError(e instanceof ApiError ? e.message : "연결하지 못했어요. 잠시 뒤 다시 시도해 주세요.");
      }
      setBusy(false);
    }
  };

  return (
    <section
      aria-label="편성표와 연결"
      className="flex min-w-0 flex-col gap-4 rounded-xl border border-hairline bg-surface-1 p-4"
      data-testid="link-to-schedule"
    >
      <p className="text-[13px] leading-normal text-text-secondary">
        편성표에서 작품과 자막 제작자를 고르면 이 규칙이 그 작품의 구독이 돼요. 일치 문구와 저장 폴더, 검사 순서는 그대로예요.
      </p>
      <PickAnime
        week={week}
        selected={anime}
        onWeek={(next) => {
          setWeek(next);
          setAnime(null);
        }}
        onPick={(entry) => {
          setAnime(entry);
          setSubtitles("undecided");
          setCreator(null);
        }}
      />
      {anime && (
        <PickSubtitles
          key={anime.anime_no}
          anime={anime}
          subtitles={subtitles}
          creator={creator}
          onChange={(nextSubtitles, nextCreator) => {
            setSubtitles(nextSubtitles);
            setCreator(nextCreator);
          }}
        />
      )}
      {error && (
        <p role="alert" className="text-[13px] leading-normal font-semibold text-urgent">
          {error}
        </p>
      )}
      <div className="flex flex-wrap gap-2">
        <Button type="button" variant="ghost" className={btnPrimary} disabled={!ready || busy} onClick={link}>
          {busy ? "연결하는 중" : "연결"}
        </Button>
        <Button type="button" variant="ghost" className={btnNeutral} disabled={busy} onClick={onCancel}>
          취소
        </Button>
      </div>
    </section>
  );
}
