import { useId, useState, type ReactNode } from "react";
import { Link } from "react-router-dom";

import { cn } from "@/lib/utils";

import { channelName, ruleTitle } from "../../collect/rules/api";
import type { SeasonInfo, WorkDetail, WorkRule } from "../api";
import { ChevronIcon } from "../icons";
import { baseName, episodeLabel } from "./model";

/**
 * A card of the right column. On a wide screen it is open and always there; on
 * a narrow one (`collapsible`) it is a section under the episode list that
 * starts folded, with a line of what it holds on its button.
 */
function Card({
  title,
  summary,
  collapsible,
  children,
}: {
  title: string;
  summary: string;
  collapsible: boolean;
  children: ReactNode;
}) {
  const id = useId();
  const [open, setOpen] = useState(false);
  const shown = !collapsible || open;
  const box = "rounded-card border border-hairline-soft bg-surface-1 shadow-(--card-shadow)";
  return (
    <section className={box} aria-labelledby={`${id}-title`}>
      {collapsible ? (
        <h2 className="m-0">
          <button
            type="button"
            aria-expanded={open}
            aria-controls={`${id}-body`}
            onClick={() => setOpen(!open)}
            className="flex min-h-14 w-full items-center gap-3 rounded-card px-4 py-2.5 text-left hover:bg-surface-2"
          >
            <span className="min-w-0 flex-1">
              <span id={`${id}-title`} className="block text-[15px] font-bold">
                {title}
              </span>
              <span className="block text-xs font-medium text-text-muted">{summary}</span>
            </span>
            <ChevronIcon className={cn("size-4 flex-none text-text-muted transition-transform", open && "rotate-90")} />
          </button>
        </h2>
      ) : (
        <h2 id={`${id}-title`} className="m-0 px-4 pt-3.5 pb-1 text-[15px] font-bold">
          {title}
        </h2>
      )}
      {shown && (
        <div id={`${id}-body`} className="flex flex-col gap-3 px-4 pt-2 pb-4">
          {children}
        </div>
      )}
    </section>
  );
}

function RuleLink({ rule }: { rule: WorkRule }) {
  return (
    <Link
      to={`/collect/rules?rule=${encodeURIComponent(rule.id)}`}
      className="flex min-w-0 flex-col gap-1 rounded-[10px] border border-hairline-soft bg-surface-2 px-3 py-2.5 hover:border-text-muted dark:hover:bg-surface-3"
    >
      <span className="flex flex-wrap items-center gap-x-2 gap-y-1">
        <span className={cn("text-[13.5px] font-bold", rule.match === null && "text-text-secondary")}>{ruleTitle(rule)}</span>
        {rule.state === "archived" && (
          <span className="rounded-full border border-hairline px-1.5 py-px text-[11.5px] font-semibold text-text-secondary">보관</span>
        )}
        {rule.state === "paused" && (
          <span className="rounded-full border border-hairline px-1.5 py-px text-[11.5px] font-semibold text-text-secondary">멈춤</span>
        )}
      </span>
      <span className="text-xs text-text-muted">{channelName(rule.channel)}</span>
      <span className="font-mono text-[12px] break-all text-text-secondary">{rule.directory}</span>
    </Link>
  );
}

/** `수집`: the rules that save into this work's folder, each a link to the rule in the collect screen. */
export function CollectCard({ work, collapsible }: { work: WorkDetail; collapsible: boolean }) {
  const count = work.rules.length;
  return (
    <Card title="수집" summary={count === 0 ? "받는 규칙 없음" : `규칙 ${count}개`} collapsible={collapsible}>
      {count === 0 ? (
        <p className="text-[13px] leading-relaxed text-text-muted">이 작품 폴더에 받는 규칙이 없어요.</p>
      ) : (
        <>
          <p className="text-[12.5px] leading-relaxed text-text-muted">이 작품 폴더에 받는 규칙이에요. 누르면 수집에서 규칙을 열어요.</p>
          <ul className="m-0 flex list-none flex-col gap-2 p-0" aria-label="이 작품 폴더에 받는 규칙">
            {work.rules.map((rule) => (
              <li key={rule.id} className="min-w-0">
                <RuleLink rule={rule} />
              </li>
            ))}
          </ul>
        </>
      )}
    </Card>
  );
}

/**
 * `작품 정보`: the synopsis of the chosen season (its first linked AniList
 * entry's description), as the paragraphs the server cut it into. They are put
 * on the page as text only, so nothing in the description can become markup.
 */
export function InfoCard({ info, collapsible }: { info: SeasonInfo; collapsible: boolean }) {
  const paragraphs = info.synopsis ?? [];
  return (
    <Card title="작품 정보" summary={paragraphs.length === 0 ? "줄거리 없음" : "줄거리"} collapsible={collapsible}>
      <div className="flex flex-col gap-1.5">
        <h3 className={subHeading}>작품 줄거리</h3>
        {paragraphs.length === 0 ? (
          <p className="text-[13px] text-text-muted">{info.entries.length === 0 ? "연결한 AniList 항목이 없어서 줄거리를 몰라요." : "AniList에 줄거리가 없어요."}</p>
        ) : (
          paragraphs.map((paragraph, index) => (
            <p key={index} className="text-[13px] leading-relaxed whitespace-pre-line break-words text-text-primary">
              {paragraph}
            </p>
          ))
        )}
      </div>
    </Card>
  );
}

const subHeading = "text-xs font-bold text-text-muted";

/** The folder, how its files are matched to episodes, and the files that could not be. */
export function FilesCard({ work, collapsible }: { work: WorkDetail; collapsible: boolean }) {
  const [mapOpen, setMapOpen] = useState(false);
  const mapId = useId();
  const episodes = work.seasons.reduce((sum, s) => sum + s.episodes.length, 0);
  const left = work.unrecognized.length;
  const summary = work.missing ? "폴더 없음" : left === 0 ? "작품 폴더와 파일 대응" : `확인하지 못한 파일 ${left}개`;
  return (
    <Card title="파일" summary={summary} collapsible={collapsible}>
      <div className="flex flex-col gap-1">
        <h3 className={subHeading}>작품 폴더</h3>
        <p className="font-mono text-[12.5px] leading-snug break-all">{work.folder_path}</p>
        {work.missing && <p className="text-xs text-text-muted">지금은 이 폴더를 찾지 못해요. 아래는 마지막으로 확인한 기록이에요.</p>}
      </div>

      <div className="flex flex-col gap-1.5 border-t border-hairline-soft pt-3">
        <h3 className={subHeading}>회차별 파일 대응</h3>
        {episodes === 0 ? (
          <p className="text-[13px] text-text-muted">회차로 읽은 파일이 없어요.</p>
        ) : (
          <>
            <button
              type="button"
              aria-expanded={mapOpen}
              aria-controls={mapId}
              onClick={() => setMapOpen(!mapOpen)}
              className="flex min-h-9 items-center gap-1.5 self-start text-[13px] font-semibold text-text-primary"
            >
              <ChevronIcon className={cn("size-3.5 text-text-muted transition-transform", mapOpen && "rotate-90")} />
              {mapOpen ? "접기" : `회차 ${episodes}개의 파일 보기`}
            </button>
            {mapOpen && (
              <div id={mapId} className="flex flex-col gap-3">
                {work.seasons
                  .filter((s) => s.episodes.length > 0)
                  .map((season) => (
                    <div key={season.number} className="flex flex-col gap-1">
                      <h4 className="text-[12.5px] font-bold text-text-secondary">시즌 {season.number}</h4>
                      <ul className="m-0 flex list-none flex-col gap-2 p-0">
                        {season.episodes.map((episode) => (
                          <li key={episode.episode} className="grid grid-cols-[3.6em_minmax(0,1fr)] gap-x-2 text-[12px] leading-snug">
                            <span className="font-bold">{episodeLabel(episode.episode)}</span>
                            <span className="flex min-w-0 flex-col gap-0.5 font-mono break-all text-text-secondary">
                              {episode.video.map((f) => (
                                <span key={f.path}>영상 {baseName(f.path)}</span>
                              ))}
                              {episode.subtitle.map((f) => (
                                <span key={f.path}>자막 {baseName(f.path)}</span>
                              ))}
                            </span>
                          </li>
                        ))}
                      </ul>
                    </div>
                  ))}
              </div>
            )}
          </>
        )}
      </div>

      <div className="flex flex-col gap-1.5 border-t border-hairline-soft pt-3">
        <h3 className={subHeading}>회차를 확인하지 못한 파일</h3>
        {left === 0 ? (
          <p className="text-[13px] text-text-muted">회차를 확인하지 못한 파일이 없어요.</p>
        ) : (
          <>
            <p className="text-xs leading-relaxed text-text-muted">이 파일들은 어느 회차에도 붙이지 않았어요.</p>
            <ul className="m-0 flex list-none flex-col gap-2.5 p-0">
              {work.unrecognized.map((file) => (
                <li key={file.path} className="flex min-w-0 flex-col gap-0.5">
                  <span className="font-mono text-[12px] leading-snug break-all">{file.path}</span>
                  <span className="text-xs leading-snug text-text-secondary">{file.message}</span>
                </li>
              ))}
            </ul>
          </>
        )}
      </div>
    </Card>
  );
}
