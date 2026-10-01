import { useEffect, useRef, useState, type FormEvent } from "react";
import { Dialog } from "radix-ui";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { ApiError } from "@/lib/api";
import { cn } from "@/lib/utils";

import { btnAction, btnDanger, btnNeutral, btnPrimary, hintClass, inputClass } from "../../collect/channels/styles";
import {
  refreshSeason,
  restartSeasonAuto,
  searchSeason,
  setSeasonLinks,
  type AnilistCandidate,
  type SeasonEntry,
  type SeasonInfo,
} from "../api";
import { fuzzyDate } from "./model";

/** The most entries one season can link (the server's limit). */
const MAX_ENTRIES = 8;

/** An entry of the draft: what is shown of it in the list. */
interface Item {
  id: number;
  title: string;
  facts: string;
}

const itemOfEntry = (e: SeasonEntry): Item => ({
  id: e.id,
  title: e.title,
  facts: [e.format, fuzzyDate(e.start), e.episodes === null ? null : `${e.episodes}화`].filter(Boolean).join(" · "),
});

const itemOfCandidate = (c: AnilistCandidate): Item => ({
  id: c.id,
  title: c.title,
  facts: [c.format, c.season_year?.toString()].filter(Boolean).join(" · "),
});

const sameIds = (a: readonly Item[], b: readonly SeasonEntry[]) => a.length === b.length && a.every((item, i) => item.id === b[i].id);

/**
 * Edits which AniList entries a season links: search and add, remove, put in
 * order, unlink, or (for the first season) go back to the automatic search. The
 * draft is saved with the version the screen shows; when someone changed the
 * link first, the draft is replaced by the current link and the dialog says so.
 */
export function SeasonLinksDialog({
  workId,
  workName,
  info,
  open,
  onOpenChange,
  onChanged,
}: {
  workId: string;
  workName: string;
  info: SeasonInfo;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onChanged: (info: SeasonInfo) => void;
}) {
  const [draft, setDraft] = useState<Item[]>([]);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const infoRef = useRef(info);
  infoRef.current = info;

  // The draft starts as the season's current link each time the dialog opens.
  useEffect(() => {
    if (!open) return;
    setDraft(infoRef.current.entries.map(itemOfEntry));
    setError(null);
    setNotice(null);
  }, [open, info.season]);

  const run = async (label: string, action: () => Promise<SeasonInfo>, done: string | null, close: boolean) => {
    if (busy) return;
    setBusy(label);
    setError(null);
    setNotice(null);
    try {
      const next = await action();
      onChanged(next);
      setDraft(next.entries.map(itemOfEntry));
      if (close) onOpenChange(false);
      else setNotice(done);
    } catch (e) {
      if (e instanceof ApiError && e.code === "conflict" && e.current) {
        const current = e.current as SeasonInfo;
        onChanged(current);
        setDraft(current.entries.map(itemOfEntry));
      }
      setError(e instanceof ApiError ? e.message : "시즌 정보를 바꾸지 못했어요.");
    } finally {
      setBusy(null);
    }
  };

  const move = (index: number, by: -1 | 1) =>
    setDraft((items) => {
      const to = index + by;
      if (to < 0 || to >= items.length) return items;
      const next = [...items];
      [next[index], next[to]] = [next[to], next[index]];
      return next;
    });

  const add = (candidate: AnilistCandidate) =>
    setDraft((items) => (items.length >= MAX_ENTRIES || items.some((i) => i.id === candidate.id) ? items : [...items, itemOfCandidate(candidate)]));

  const dirty = !sameIds(draft, info.entries);

  return (
    <Dialog.Root open={open} onOpenChange={onOpenChange}>
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-[250] bg-[rgba(4,6,10,0.62)] data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:animate-in data-[state=open]:fade-in-0" />
        <Dialog.Content
          aria-describedby={undefined}
          className={cn(
            "fixed top-1/2 left-1/2 z-[251] flex max-h-[min(92dvh,860px)] w-[min(720px,calc(100vw-32px))] -translate-x-1/2 -translate-y-1/2 flex-col rounded-2xl border border-hairline bg-surface-1 shadow-[0_20px_50px_-16px_var(--shadow-color)] outline-none",
            "data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:animate-in data-[state=open]:fade-in-0",
          )}
        >
          <div className="flex items-center gap-2 border-b border-hairline-soft py-2 pr-2 pl-4">
            <Dialog.Title className="min-w-0 flex-1 truncate text-[17px] font-bold">시즌 {info.season} AniList 연결</Dialog.Title>
            <Dialog.Close
              aria-label="닫기"
              className="inline-flex size-11 flex-none items-center justify-center rounded-full text-text-secondary hover:bg-surface-2 hover:text-text-primary"
            >
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" aria-hidden="true" focusable="false" className="size-5">
                <path d="M6 6l12 12M18 6L6 18" />
              </svg>
            </Dialog.Close>
          </div>

          <div className="flex min-h-0 flex-1 flex-col gap-5 overflow-x-hidden overflow-y-auto overscroll-contain p-4 pb-[calc(16px+env(safe-area-inset-bottom,0px))]">
            <section aria-labelledby="links-title" className="flex flex-col gap-2.5">
              <h3 id="links-title" className="text-[13px] font-bold text-text-muted">
                연결한 AniList 항목 (이 순서로 이어서 보여줘요)
              </h3>
              {draft.length === 0 ? (
                <p className="text-[13px] leading-relaxed text-text-secondary">
                  연결한 항목이 없어요.
                  {info.origin === "auto" && info.pending === "search" ? " 앱이 제목이 같은 작품을 찾고 있어요." : ""}
                  {info.note ? ` ${info.note.message}` : ""}
                </p>
              ) : (
                <ol className="m-0 flex list-none flex-col gap-2 p-0" aria-label="연결한 AniList 항목">
                  {draft.map((item, index) => (
                    <li key={item.id} className="flex flex-wrap items-center gap-x-3 gap-y-1.5 rounded-[10px] bg-surface-2 px-3 py-2 dark:bg-surface-2">
                      <span className="text-[12px] font-bold text-text-muted">{index + 1}</span>
                      <span className="flex min-w-0 flex-1 basis-40 flex-col">
                        <span className="text-[13px] leading-snug font-semibold break-words">{item.title}</span>
                        <span className="text-xs text-text-muted">
                          {item.facts ? `${item.facts} · ` : ""}#{item.id}
                        </span>
                      </span>
                      <span className="flex flex-none items-center gap-1">
                        <Button
                          type="button"
                          variant="ghost"
                          className={cn(btnNeutral, "px-3")}
                          aria-label={`${item.title} 위로`}
                          disabled={index === 0 || busy !== null}
                          onClick={() => move(index, -1)}
                        >
                          위로
                        </Button>
                        <Button
                          type="button"
                          variant="ghost"
                          className={cn(btnNeutral, "px-3")}
                          aria-label={`${item.title} 아래로`}
                          disabled={index === draft.length - 1 || busy !== null}
                          onClick={() => move(index, 1)}
                        >
                          아래로
                        </Button>
                        <Button
                          type="button"
                          variant="ghost"
                          className={cn(btnDanger, "px-3")}
                          aria-label={`${item.title} 빼기`}
                          disabled={busy !== null}
                          onClick={() => setDraft((items) => items.filter((i) => i.id !== item.id))}
                        >
                          빼기
                        </Button>
                      </span>
                    </li>
                  ))}
                </ol>
              )}
              <p className={hintClass}>
                TheTVDB의 시즌 하나가 AniList에서 두 항목(1쿨·2쿨)으로 나뉘어 있으면 모두 골라 순서를 맞춰 주세요. 줄거리는 첫 항목의 것을 보여줘요.
                {draft.length >= MAX_ENTRIES ? ` 항목은 ${MAX_ENTRIES}개까지 이을 수 있어요.` : ""}
              </p>
              <div className="flex flex-wrap gap-2">
                <Button
                  type="button"
                  variant="ghost"
                  className={btnPrimary}
                  disabled={busy !== null || !dirty}
                  onClick={() =>
                    void run(
                      "save",
                      () =>
                        setSeasonLinks(
                          workId,
                          info.season,
                          info.version,
                          draft.map((i) => i.id),
                        ),
                      null,
                      true,
                    )
                  }
                >
                  {busy === "save" ? "저장하는 중…" : "저장"}
                </Button>
                {draft.length > 0 && (
                  <Button
                    type="button"
                    variant="ghost"
                    className={btnDanger}
                    disabled={busy !== null}
                    onClick={() => void run("unlink", () => setSeasonLinks(workId, info.season, info.version, []), null, true)}
                  >
                    연결 끊기
                  </Button>
                )}
                {info.can_auto && (
                  <Button
                    type="button"
                    variant="ghost"
                    className={btnAction}
                    disabled={busy !== null}
                    onClick={() => void run("auto", () => restartSeasonAuto(workId, info.season, info.version), null, true)}
                  >
                    자동으로 다시 찾기
                  </Button>
                )}
                {info.entries.length > 0 && (
                  <Button
                    type="button"
                    variant="ghost"
                    className={btnNeutral}
                    disabled={busy !== null || dirty}
                    onClick={() => void run("refresh", () => refreshSeason(workId, info.season), "AniList에서 다시 받았어요.", false)}
                  >
                    {busy === "refresh" ? "받는 중…" : "정보 다시 받기"}
                  </Button>
                )}
              </div>
              {info.can_auto && (
                <p className={hintClass}>
                  `자동으로 다시 찾기`는 지금 연결을 끊고, 작품 폴더 이름과 정확히 같은 제목의 AniList 작품 하나를 앱이 다시 찾게 해요.
                </p>
              )}
              {notice && (
                <p className="text-[13px] font-semibold text-ok" role="status">
                  {notice}
                </p>
              )}
              {error && (
                <p role="alert" className="text-[13px] leading-relaxed font-semibold text-urgent">
                  {error}
                </p>
              )}
              {busy !== null && busy !== "save" && busy !== "refresh" && (
                <p className={hintClass} role="status">
                  바꾸는 중이에요.
                </p>
              )}
            </section>

            <SeasonSearch workId={workId} season={info.season} initial={workName} chosen={draft} full={draft.length >= MAX_ENTRIES} onAdd={add} />
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

/** AniList's search, a page at a time; an entry is added to the draft, not saved yet. */
function SeasonSearch({
  workId,
  season,
  initial,
  chosen,
  full,
  onAdd,
}: {
  workId: string;
  season: number;
  initial: string;
  chosen: readonly Item[];
  full: boolean;
  onAdd: (candidate: AnilistCandidate) => void;
}) {
  const [query, setQuery] = useState(initial);
  const [results, setResults] = useState<{ q: string; items: AnilistCandidate[]; page: number; hasNext: boolean } | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const search = async (q: string, page: number) => {
    setLoading(true);
    setError(null);
    try {
      const answer = await searchSeason(workId, season, q, page);
      setResults((prev) => ({
        q,
        items: page > 1 && prev ? [...prev.items, ...answer.items] : answer.items,
        page: answer.page,
        hasNext: answer.has_next,
      }));
    } catch (e) {
      setError(e instanceof ApiError ? e.message : "AniList를 검색하지 못했어요.");
    } finally {
      setLoading(false);
    }
  };

  // The folder name is searched as soon as the dialog opens.
  const started = useRef(false);
  useEffect(() => {
    if (started.current) return;
    started.current = true;
    void search(initial, 1);
  }, []);

  const submit = (event: FormEvent) => {
    event.preventDefault();
    const q = query.trim();
    if (q !== "") void search(q, 1);
  };

  return (
    <section aria-labelledby="season-search-title" className="flex flex-col gap-3 border-t border-hairline-soft pt-4">
      <h3 id="season-search-title" className="text-[13px] font-bold text-text-muted">
        AniList에서 찾기
      </h3>
      <form onSubmit={submit} className="flex gap-2">
        <Input
          aria-label="AniList 검색어"
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          className={cn(inputClass, "min-w-0 flex-1")}
        />
        <Button type="submit" variant="ghost" className={btnNeutral} disabled={loading}>
          검색
        </Button>
      </form>
      {error && (
        <p role="alert" className="text-[13px] font-semibold text-urgent">
          {error}
        </p>
      )}
      {results && results.items.length === 0 && !loading && (
        <p className={hintClass}>‘{results.q}’로 찾은 AniList 작품이 없어요. 다른 이름으로 찾아 보세요.</p>
      )}
      {results && results.items.length > 0 && (
        <ul aria-label="AniList 검색 결과" className="m-0 flex list-none flex-col gap-2 p-0">
          {results.items.map((candidate) => {
            const item = itemOfCandidate(candidate);
            const added = chosen.some((c) => c.id === candidate.id);
            const others = candidate.titles.filter((t) => t !== candidate.title);
            return (
              <li key={candidate.id} className="flex flex-wrap items-center gap-x-3 gap-y-1.5 rounded-[10px] bg-surface-2 px-3 py-2 dark:bg-surface-2">
                <span className="flex min-w-0 flex-1 basis-48 flex-col">
                  <span className="text-[13px] leading-snug font-semibold break-words">{candidate.title}</span>
                  {others.length > 0 && <span className="line-clamp-1 text-[11.5px] text-text-muted">{others.join(" / ")}</span>}
                  <span className="text-xs text-text-secondary">
                    {item.facts ? `${item.facts} · ` : ""}#{candidate.id}
                  </span>
                </span>
                <Button
                  type="button"
                  variant="ghost"
                  className={btnAction}
                  disabled={added || full}
                  aria-label={added ? `${candidate.title} 이미 추가함` : `${candidate.title} 추가`}
                  onClick={() => onAdd(candidate)}
                >
                  {added ? "추가함" : "추가"}
                </Button>
              </li>
            );
          })}
        </ul>
      )}
      {loading && <p className={hintClass}>AniList에서 찾는 중이에요.</p>}
      {results?.hasNext && !loading && (
        <Button type="button" variant="ghost" className={cn(btnNeutral, "self-start")} onClick={() => void search(results.q, results.page + 1)}>
          결과 더 보기
        </Button>
      )}
    </section>
  );
}
