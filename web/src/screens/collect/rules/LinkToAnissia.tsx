import { useState } from "react";

import { Button } from "@/components/ui/button";
import { ApiError } from "@/lib/api";
import type { Rule, ScheduleEntry, SubtitleMode } from "@/lib/api-types";

import { btnNeutral, btnPrimary, hintClass } from "../channels/styles";
import { linkToAnissia, type PickedFrom } from "../subs/api";
import { AnissiaListTabs, AnissiaSearch, type AnissiaList } from "../subs/add/AnissiaSearch";
import { PickAnime } from "../subs/add/PickAnime";
import { PickSubtitles } from "../subs/add/PickSubtitles";
import { todayWeek } from "../subs/format";

/**
 * `Anissia 연결`: makes an existing rule a subscription. The anime comes from
 * Anissia's full list, finished anime included (the user writes the query;
 * nothing is searched on open), or from this quarter's schedule (a weekday, or
 * `작품 찾기` in every week), and the subtitle choice from the same step the
 * subscribe flow has. The rule's match phrase, save folder and order are not
 * touched. `onStale` gets the rule as the server has it when someone else
 * changed it first.
 */
export function LinkToAnissia({
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
  const [tab, setTab] = useState<AnissiaList>("list");
  const [query, setQuery] = useState("");
  const [week, setWeek] = useState(() => todayWeek());
  const [anime, setAnime] = useState<ScheduleEntry | null>(null);
  const [from, setFrom] = useState<PickedFrom | null>(null);
  const [subtitles, setSubtitles] = useState<SubtitleMode>("undecided");
  const [creator, setCreator] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const ready = anime !== null && from !== null && (subtitles !== "follow" || creator !== null);

  // The pick keeps where it was made, so it stays while the other tab or another weekday is looked at.
  const pick = (entry: ScheduleEntry, picked: PickedFrom) => {
    setAnime(entry);
    setFrom(picked);
    setSubtitles("undecided");
    setCreator(null);
  };

  const link = async () => {
    if (anime === null || from === null || !ready) return;
    setBusy(true);
    setError(null);
    try {
      const linked = await linkToAnissia(rule, {
        anissia_anime_no: anime.anime_no,
        ...from,
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
      aria-label="Anissia 연결"
      className="flex min-w-0 flex-col gap-4 rounded-xl border border-hairline bg-surface-1 p-4"
      data-testid="link-to-anissia"
    >
      <p className="text-[13px] leading-normal text-text-secondary">
        Anissia에서 작품과 자막 제작자를 고르면 이 규칙이 그 작품의 구독이 돼요. 일치 문구와 저장 폴더, 검사 순서는 그대로예요.
      </p>
      <AnissiaListTabs tab={tab} onTab={setTab} />
      {/* Both tabs stay mounted, so a search and its place survive switching. */}
      <div hidden={tab !== "list"}>
        <div className="flex min-w-0 flex-col gap-3.5">
          <AnissiaSearch
            query={query}
            onQuery={setQuery}
            ended
            hint="방영이 끝난 작품도 찾아요. Anissia는 한국어 제목으로 찾아져서 일본어나 영어 제목은 거의 맞지 않아요."
            selected={anime}
            onPick={pick}
          />
        </div>
      </div>
      <div hidden={tab !== "schedule"}>
        <PickAnime heading="이번 분기 편성표에서 골라요" week={week} selected={anime} onWeek={setWeek} onPick={pick} />
      </div>
      <p className={hintClass} role="status">
        {anime ? `고른 작품: ${anime.subject}` : "연결할 작품을 골라 주세요."}
      </p>
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
