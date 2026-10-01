import { useEffect, useId, useState } from "react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { ApiError } from "@/lib/api";
import { ago } from "@/lib/time";
import { cn } from "@/lib/utils";

import { channelTitle, type Channel } from "../../channels/api";
import { btnNeutral, hintClass, inputClass, labelClass } from "../../channels/styles";
import { fetchTitles, type TitleGroup, type Titles } from "../api";
import { optionClass } from "./draft";

type State =
  | { state: "loading"; previous: Titles | null }
  | { state: "ready"; titles: Titles }
  | { state: "failed"; message: string };

const DEBOUNCE_MS = 250;

/**
 * Step 3: the release title, chosen from the works the channel's history
 * records. There is no free text field: the match phrase is always one a real
 * item of this channel carries.
 */
export function PickTitle({
  channel,
  selected,
  onPick,
}: {
  channel: Channel;
  selected: TitleGroup | null;
  onPick: (work: TitleGroup) => void;
}) {
  const uid = useId();
  const [filter, setFilter] = useState("");
  const [round, setRound] = useState(0);
  const [result, setResult] = useState<State>({ state: "loading", previous: null });
  const query = filter.trim();

  useEffect(() => {
    const controller = new AbortController();
    setResult((prev) => ({
      state: "loading",
      previous: prev.state === "ready" ? prev.titles : prev.state === "loading" ? prev.previous : null,
    }));
    const timer = window.setTimeout(
      () => {
        fetchTitles(channel.id, query, controller.signal).then(
          (titles) => {
            if (!controller.signal.aborted) setResult({ state: "ready", titles });
          },
          (e: unknown) => {
            if (controller.signal.aborted) return;
            setResult({
              state: "failed",
              message: e instanceof ApiError ? e.message : "작품 목록을 불러오지 못했어요.",
            });
          },
        );
      },
      query === "" ? 0 : DEBOUNCE_MS,
    );
    return () => {
      window.clearTimeout(timer);
      controller.abort();
    };
  }, [channel.id, query, round]);

  const titles = result.state === "ready" ? result.titles : result.state === "loading" ? result.previous : null;
  const stale = result.state === "loading";
  // With nothing recorded there is nothing to filter, so the box goes.
  const empty = titles !== null && titles.recorded_items === 0;
  const nothingFits = titles !== null && titles.recorded_items > 0 && titles.total === 0 && query === "";

  return (
    <section aria-labelledby={`${uid}-h`} className="flex min-w-0 flex-col gap-3.5">
      <div className="flex min-w-0 flex-col gap-1">
        <h3 id={`${uid}-h`} className="text-[15px] font-bold">
          릴리스 제목을 골라요
        </h3>
        <p className="min-w-0 text-[13px] leading-normal text-text-muted">
          {channelTitle(channel)}에 기록된 항목에서 골라요. 고른 제목의 작품 부분이 규칙의 일치 문구가 돼요.
        </p>
      </div>

      {titles !== null && !empty && !nothingFits && (
        <div className="flex min-w-0 flex-col gap-1.5">
          <Label htmlFor={`${uid}-filter`} className={labelClass}>
            제목 찾기
          </Label>
          <Input
            id={`${uid}-filter`}
            type="search"
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
            placeholder="작품 이름 일부"
            autoComplete="off"
            className={inputClass}
          />
        </div>
      )}

      {result.state === "failed" && (
        <div className="flex flex-col items-start gap-2.5">
          <p role="alert" className="text-[13px] font-semibold text-urgent">
            {result.message}
          </p>
          <Button type="button" variant="ghost" className={btnNeutral} onClick={() => setRound((n) => n + 1)}>
            재시도
          </Button>
        </div>
      )}

      {titles === null && result.state === "loading" && (
        <p className="text-[13px] text-text-muted">기록을 불러오는 중이에요.</p>
      )}

      {empty && (
        <p role="status" className="text-[13px] leading-normal font-semibold text-text-secondary">
          이 채널에는 수집 기록이 아직 없어서 릴리스 제목을 고를 수 없어요. worker가 채널을 한 번 읽은 뒤에 다시 와요.
        </p>
      )}

      {nothingFits && (
        <p role="status" className="text-[13px] leading-normal font-semibold text-text-secondary">
          이 채널의 기록에서 작품 제목을 찾지 못해서 릴리스 제목을 고를 수 없어요. 다른 채널을 골라요.
        </p>
      )}

      {titles !== null && !empty && !nothingFits && titles.total === 0 && (
        <p role="status" className="text-[13px] leading-normal font-semibold text-text-secondary">
          ‘{query}’에 맞는 릴리스 제목이 이 채널의 기록에 없어요.
        </p>
      )}

      {titles !== null && titles.titles.length > 0 && (
        <div className={cn("flex min-w-0 flex-col gap-2", stale && "opacity-60")}>
          <ul className="m-0 flex list-none flex-col gap-2 p-0">
            {titles.titles.map((group) => {
              const on = selected?.work === group.work;
              return (
                <li key={group.work}>
                  <button
                    type="button"
                    aria-pressed={on}
                    onClick={() => onPick(group)}
                    className={cn(optionClass, on ? "border-focus" : "border-hairline-soft")}
                  >
                    <span className="min-w-0 text-[14.5px] leading-snug font-semibold break-all">{group.work}</span>
                    <span className="min-w-0 text-xs leading-snug break-all text-text-muted">
                      {group.latest_title}
                    </span>
                    <span className="text-xs text-text-secondary">
                      항목 {group.items}개 · 마지막 {ago(group.latest_seen_at)}
                    </span>
                  </button>
                </li>
              );
            })}
          </ul>
          {titles.truncated && (
            <p className={hintClass}>작품이 많아서 {titles.titles.length}편만 보여줘요. 이름을 입력하면 좁힐 수 있어요.</p>
          )}
        </div>
      )}
    </section>
  );
}
