import { useEffect, useRef, useState, type ComponentType, type ReactNode } from "react";

import { Button } from "@/components/ui/button";
import { ApiError } from "@/lib/api";
import { cn } from "@/lib/utils";

import { btnAction, btnNeutral, hintClass } from "../../collect/channels/styles";
import { loadSeasonInfo, setSeasonLinks, type SeasonInfo, type SeasonSuggestion } from "../api";
import { CalendarIcon, ExternalIcon, FilmIcon, StudioIcon, TagIcon } from "../icons";
import { SeasonLinksDialog } from "./SeasonLinksDialog";
import { airingBadge, airingText, episodesText, fuzzyDate } from "./model";

/** How often the info is read again while the app still has the season's search to do. */
const PENDING_POLL_MS = 4000;

function Cell({
  icon: Icon,
  label,
  children,
  unknown,
}: {
  icon: ComponentType<{ className?: string }>;
  label: string;
  children: ReactNode;
  unknown: boolean;
}) {
  return (
    <div className="min-w-0">
      <dt className="flex items-center gap-1.5 text-[12px] font-semibold text-text-muted">
        <Icon className="size-3.5 flex-none" />
        {label}
      </dt>
      <dd className={cn("m-0 mt-1 text-[13.5px] leading-snug break-words", unknown ? "text-text-muted" : "text-text-primary")}>{children}</dd>
    </div>
  );
}

const facts = (s: SeasonSuggestion) => [s.format, fuzzyDate(s.start)].filter(Boolean).join(" · ");

/**
 * The season's AniList info: the `방영`, `분량`, `제작사` and `장르` of the linked
 * entries taken together, the link to the first entry's AniList page, and the
 * ways to change the link. A season with no link says `미상`, never what another
 * season or the files would suggest. The sequels of the previous season's last
 * entry are offered quietly, and link nothing until one is confirmed.
 */
export function SeasonInfoSection({
  workId,
  workName,
  info,
  seasonCount,
  onChanged,
}: {
  workId: string;
  workName: string;
  info: SeasonInfo;
  seasonCount: number;
  /** Called with the info after any change (and when the app's search finished), to show it and update the caches. */
  onChanged: (info: SeasonInfo) => void;
}) {
  const [editing, setEditing] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const onChangedRef = useRef(onChanged);
  onChangedRef.current = onChanged;

  // While the app's search for this season is pending, read the info now and then.
  const pending = info.pending !== null;
  const { season, version } = info;
  useEffect(() => {
    if (!pending) return;
    const controller = new AbortController();
    const timer = setInterval(() => {
      loadSeasonInfo(workId, season, controller.signal).then(
        (next) => {
          if (next.version !== version || next.pending === null) onChangedRef.current(next);
        },
        () => {},
      );
    }, PENDING_POLL_MS);
    return () => {
      clearInterval(timer);
      controller.abort();
    };
  }, [workId, season, version, pending]);

  // Another season's message never stays.
  useEffect(() => setError(null), [season]);

  const confirm = async (suggestion: SeasonSuggestion) => {
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      onChanged(await setSeasonLinks(workId, season, info.version, [suggestion.id]));
    } catch (e) {
      if (e instanceof ApiError && e.code === "conflict" && e.current) onChanged(e.current as SeasonInfo);
      setError(e instanceof ApiError ? e.message : "시즌 정보를 연결하지 못했어요.");
    } finally {
      setBusy(false);
    }
  };

  const linked = info.entries.length > 0;
  const badge = airingBadge(info);
  const title = seasonCount > 1 ? `시즌 ${season} 정보` : "시즌 정보";
  const waiting = info.pending === "search" && !linked;
  const suggestions = linked ? [] : info.suggestions;

  return (
    <section aria-labelledby={`season-info-${season}`} className="@container rounded-card border border-hairline-soft bg-surface-1 px-4 py-3.5 shadow-(--card-shadow) max-[720px]:px-3">
      <div className="flex flex-wrap items-center gap-x-2.5 gap-y-1.5">
        <h2 id={`season-info-${season}`} className="m-0 text-[15px] font-bold">
          {title}
        </h2>
        {linked && info.origin === "auto" && (
          <span className="rounded-full border border-hairline px-2 py-px text-[11.5px] font-semibold text-text-secondary">자동</span>
        )}
        {badge && <span className="rounded-full bg-surface-2 px-2 py-px text-[11.5px] font-bold text-text-secondary">{badge}</span>}
        <span className="ml-auto flex flex-wrap items-center gap-x-1.5 gap-y-1">
          {info.anilist_url && (
            <a
              href={info.anilist_url}
              target="_blank"
              rel="noopener noreferrer"
              className="inline-flex min-h-9 items-center gap-1 rounded-full px-2.5 text-[13px] font-semibold text-focus underline-offset-4 hover:underline max-[720px]:min-h-10"
            >
              AniList
              <ExternalIcon className="size-3.5" />
              <span className="sr-only">에서 시즌 {season} 보기 (새 창)</span>
            </a>
          )}
          <Button type="button" variant="ghost" className={btnNeutral} aria-haspopup="dialog" onClick={() => setEditing(true)}>
            연결 바꾸기
          </Button>
        </span>
      </div>

      <dl className="m-0 mt-3 grid grid-cols-2 gap-x-4 gap-y-3 @[560px]:grid-cols-4">
        <Cell icon={CalendarIcon} label="방영" unknown={!info.airing || airingText(info) === "미상"}>
          {airingText(info)}
        </Cell>
        <Cell icon={FilmIcon} label="분량" unknown={info.episodes === null}>
          {episodesText(info.episodes)}
        </Cell>
        <Cell icon={StudioIcon} label="제작사" unknown={info.studios.length === 0}>
          {info.studios.length === 0 ? "미상" : info.studios.join(", ")}
        </Cell>
        <Cell icon={TagIcon} label="장르" unknown={info.genres.length === 0}>
          {info.genres.length === 0 ? "미상" : info.genres.join(", ")}
        </Cell>
      </dl>

      {info.entries.length > 1 && (
        <p className={cn(hintClass, "mt-3")}>
          AniList 항목 {info.entries.length}개를 이어서 보여줘요: {info.entries.map((e) => e.title).join(" → ")}
        </p>
      )}
      {waiting && (
        <p className={cn(hintClass, "mt-3")} role="status">
          AniList에서 제목이 같은 작품을 찾고 있어요.
        </p>
      )}
      {!linked && !waiting && info.note && (
        <p className={cn(hintClass, "mt-3")}>
          {info.note.message} 연결 바꾸기에서 직접 고를 수 있어요.
        </p>
      )}

      {suggestions.length > 0 && (
        <div className="mt-3 flex flex-col gap-2 border-t border-hairline-soft pt-3" aria-label="앞 시즌의 속편 제안">
          <p className={hintClass}>앞 시즌 마지막 항목의 속편이에요. 맞는 항목을 확인하면 연결해요.</p>
          <ul className="m-0 flex list-none flex-col gap-2 p-0">
            {suggestions.map((s) => (
              <li
                key={s.id}
                className="flex flex-wrap items-center gap-x-3 gap-y-1.5 rounded-[10px] bg-surface-2 px-3 py-2 dark:bg-surface-2"
              >
                <span className="flex min-w-0 flex-1 basis-48 flex-col">
                  <span className="text-[13px] leading-snug font-semibold break-words">{s.title || `AniList #${s.id}`}</span>
                  <span className="text-xs text-text-muted">
                    {facts(s) || "정보 없음"} ·{" "}
                    <a
                      href={s.url}
                      target="_blank"
                      rel="noopener noreferrer"
                      className="text-focus underline underline-offset-2"
                    >
                      AniList<span className="sr-only"> {s.title} 보기 (새 창)</span>
                    </a>
                  </span>
                </span>
                <Button type="button" variant="ghost" className={btnAction} disabled={busy} onClick={() => void confirm(s)}>
                  <span aria-hidden="true">이 항목이 맞아요</span>
                  <span className="sr-only">{s.title} 항목이 맞아요</span>
                </Button>
              </li>
            ))}
          </ul>
          <Button type="button" variant="ghost" className={cn(btnNeutral, "self-start")} disabled={busy} onClick={() => setEditing(true)}>
            다른 항목 고르기
          </Button>
        </div>
      )}
      {error && (
        <p role="alert" className="mt-3 text-[13px] leading-relaxed font-semibold text-urgent">
          {error}
        </p>
      )}

      <SeasonLinksDialog workId={workId} workName={workName} info={info} open={editing} onOpenChange={setEditing} onChanged={onChanged} />
    </section>
  );
}
