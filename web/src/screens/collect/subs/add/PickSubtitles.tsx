import { useId } from "react";

import { Button } from "@/components/ui/button";
import { useCached } from "@/lib/cached";

import { btnNeutral } from "../../channels/styles";
import { fetchCreators, type Creators, type ScheduleEntry, type SubtitleMode } from "../api";
import { SUBTITLE_NOTES } from "../format";

const radio = "mt-0.5 size-[18px] flex-none accent-focus";
const option =
  "flex min-w-0 cursor-pointer items-start gap-3 rounded-[10px] border border-hairline-soft bg-surface-2 px-3.5 py-3 has-[:checked]:border-focus has-[:focus-visible]:outline-2 has-[:focus-visible]:outline-offset-2 has-[:focus-visible]:outline-focus";

/**
 * Step 4: how subtitles come. One choice per creator Anissia lists for the
 * anime, `제작자 미정` and `받지 않음`. The choice is stored with the
 * subscription; receiving subtitles is another feature. When Anissia does not
 * answer the creators, the two choices that need none stay open.
 */
export function PickSubtitles({
  anime,
  subtitles,
  creator,
  onChange,
}: {
  anime: ScheduleEntry;
  subtitles: SubtitleMode;
  creator: string | null;
  onChange: (subtitles: SubtitleMode, creator: string | null) => void;
}) {
  const uid = useId();
  const list = useCached<Creators>(
    `collect:creators:${anime.anime_no}`,
    (signal) => fetchCreators(anime.anime_no, signal),
    "자막 제작자를 불러오지 못했어요.",
  );
  const creators = list.data?.creators;

  return (
    <fieldset className="m-0 flex min-w-0 flex-col gap-3 border-0 p-0">
      <legend className="mb-1 p-0 text-[15px] font-bold">자막을 어떻게 받을지 골라요</legend>

      {creators === undefined && list.error === null && list.slow && (
        <p className="text-[13px] text-text-muted">자막 제작자를 불러오는 중이에요.</p>
      )}

      {list.error !== null && (
        <div className="flex flex-col items-start gap-2.5">
          <p role="alert" className="text-[13px] leading-normal font-semibold text-urgent">
            {list.error}
          </p>
          <p className="text-[13px] leading-normal text-text-muted">
            제작자 목록 없이도 ‘제작자 미정’이나 ‘받지 않음’으로 구독할 수 있어요.
          </p>
          <Button type="button" variant="ghost" className={btnNeutral} onClick={list.reload}>
            재시도
          </Button>
        </div>
      )}

      {creators !== undefined && creators.length === 0 && (
        <p className="text-[13px] leading-normal text-text-muted">이 작품은 Anissia에 등록된 자막 제작자가 없어요.</p>
      )}

      {creators?.map((c) => (
        <label key={c.name} className={option}>
          <input
            type="radio"
            name={`${uid}-subtitles`}
            className={radio}
            checked={subtitles === "follow" && creator === c.name}
            onChange={() => onChange("follow", c.name)}
          />
          <span className="flex min-w-0 flex-col gap-0.5">
            <span className="min-w-0 text-[14.5px] font-semibold break-words">{c.name} 따라 받기</span>
            <span className="text-[13px] leading-normal text-text-secondary">{SUBTITLE_NOTES.follow(c.name)}</span>
            <span className="text-xs text-text-muted">
              자막 {c.captions}개{c.last_updated_at ? ` · 마지막 ${c.last_updated_at.slice(0, 10)}` : ""}
            </span>
          </span>
        </label>
      ))}

      <label className={option}>
        <input
          type="radio"
          name={`${uid}-subtitles`}
          className={radio}
          checked={subtitles === "undecided"}
          onChange={() => onChange("undecided", null)}
        />
        <span className="flex min-w-0 flex-col gap-0.5">
          <span className="text-[14.5px] font-semibold">제작자 미정</span>
          <span className="text-[13px] leading-normal text-text-secondary">{SUBTITLE_NOTES.undecided}</span>
        </span>
      </label>

      <label className={option}>
        <input
          type="radio"
          name={`${uid}-subtitles`}
          className={radio}
          checked={subtitles === "none"}
          onChange={() => onChange("none", creator)}
        />
        <span className="flex min-w-0 flex-col gap-0.5">
          <span className="text-[14.5px] font-semibold">받지 않음</span>
          <span className="text-[13px] leading-normal text-text-secondary">{SUBTITLE_NOTES.none}</span>
        </span>
      </label>
    </fieldset>
  );
}
