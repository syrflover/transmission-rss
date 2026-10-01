import { useEffect, useRef, useState, type FormEvent, type ReactNode } from "react";
import { Dialog } from "radix-ui";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { ApiError } from "@/lib/api";
import { cn } from "@/lib/utils";

import { btnAction, btnDanger, btnNeutral, hintClass, inputClass } from "../../collect/channels/styles";
import {
  changeArtwork,
  loadArtwork,
  pickArtwork,
  searchAnilist,
  uploadArtwork,
  type AnilistCandidate,
  type ArtworkState,
} from "../api";
import type { Work } from "../model";
import { Cover } from "../WorkItem";

/** How often an open cover view reads the state again while the app is still searching or receiving. */
const PENDING_POLL_MS = 3000;

const FAILED = "표지 상태를 불러오지 못했어요.";

/** What the state says, as the line under the cover. */
function describe(state: ArtworkState): string {
  const id = state.anilist_media_id;
  switch (state.mode) {
    case "disabled":
      return "표지를 비워 뒀어요. 자동으로 찾지 않아요.";
    case "manual":
      return state.source === "upload" ? "직접 올린 파일이에요." : `AniList에서 직접 고른 표지예요 (AniList #${id}).`;
    case "auto":
      if (state.source === "anilist") return `AniList에서 자동으로 고른 표지예요 (AniList #${id}).`;
      return state.pending === "search" ? "AniList에서 제목이 같은 작품을 찾고 있어요." : "아직 정해진 표지가 없어요.";
  }
}

/** What is wrong with the image file, and what fixes it. */
function recovery(state: ArtworkState): string | null {
  const image = state.image;
  // An AniList entry is selected but its image never arrived (the receiving gave up).
  if (!image && state.source === "anilist" && state.pending === null) {
    return "표지 이미지를 받지 못했어요. 다시 받으면 같은 AniList 작품의 표지를 받아요.";
  }
  if (!image || image.status === "available") return null;
  const what =
    image.status === "missing"
      ? "표지 파일을 찾지 못했어요."
      : image.status === "mismatch"
        ? "표지 파일의 내용이 기록과 달라요."
        : "표지 파일을 확인하지 못했어요.";
  const fix =
    state.source === "upload"
      ? "같은 이미지를 다시 올리거나 다른 표지를 골라 주세요."
      : "다시 받으면 같은 AniList 작품의 표지를 받아요.";
  return `${what} ${fix}`;
}

/**
 * The cover, large, with the ways to change it: choose from AniList's search,
 * upload a file, clear it, or go back to automatic. Every change is sent with
 * the version the view shows; when someone changed the cover first, the view
 * shows the current state and says so.
 */
export function CoverDialog({
  workId,
  work,
  coverUrl,
  open,
  onOpenChange,
  onChanged,
}: {
  workId: string;
  work: Pick<Work, "title" | "hue" | "initial">;
  coverUrl: string | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** The cover URL after a change, for the head and the list. */
  onChanged: (coverUrl: string | null) => void;
}) {
  const [state, setState] = useState<ArtworkState | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [searching, setSearching] = useState(false);
  const fileRef = useRef<HTMLInputElement>(null);
  const onChangedRef = useRef(onChanged);
  onChangedRef.current = onChanged;

  // Read the state when the view opens, and again while something is pending.
  const pending = state?.pending ?? null;
  useEffect(() => {
    if (!open) return;
    const controller = new AbortController();
    const read = () =>
      loadArtwork(workId, controller.signal).then(
        (next) => {
          setState(next);
          setLoadError(null);
          onChangedRef.current(next.image?.status === "available" ? next.image.url : null);
        },
        (e: unknown) => {
          if (controller.signal.aborted) return;
          setLoadError(e instanceof ApiError ? e.message : FAILED);
        },
      );
    void read();
    const timer = pending ? setInterval(() => void read(), PENDING_POLL_MS) : undefined;
    return () => {
      controller.abort();
      clearInterval(timer);
    };
  }, [open, workId, pending]);

  useEffect(() => {
    if (!open) {
      setSearching(false);
      setError(null);
    }
  }, [open]);

  const run = async (label: string, action: (version: number) => Promise<ArtworkState>) => {
    if (!state || busy) return;
    setBusy(label);
    setError(null);
    try {
      const next = await action(state.version);
      setState(next);
      setSearching(false);
      onChanged(next.image?.status === "available" ? next.image.url : null);
    } catch (e) {
      if (e instanceof ApiError && e.code === "conflict" && e.current) {
        setState(e.current as ArtworkState);
      }
      setError(e instanceof ApiError ? e.message : "표지를 바꾸지 못했어요.");
    } finally {
      setBusy(null);
    }
  };

  const image = state?.image?.status === "available" ? state.image.url : state ? null : coverUrl;
  const problem = state ? recovery(state) : null;

  return (
    <Dialog.Root open={open} onOpenChange={onOpenChange}>
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-[250] bg-[rgba(4,6,10,0.62)] data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:animate-in data-[state=open]:fade-in-0" />
        <Dialog.Content
          aria-describedby={undefined}
          className={cn(
            "fixed top-1/2 left-1/2 z-[251] flex max-h-[min(92dvh,860px)] w-[min(880px,calc(100vw-32px))] -translate-x-1/2 -translate-y-1/2 flex-col rounded-2xl border border-hairline bg-surface-1 shadow-[0_20px_50px_-16px_var(--shadow-color)] outline-none",
            "data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:animate-in data-[state=open]:fade-in-0",
          )}
        >
          <div className="flex items-center gap-2 border-b border-hairline-soft py-2 pr-2 pl-4">
            <Dialog.Title className="min-w-0 flex-1 truncate text-[17px] font-bold">표지 · {work.title}</Dialog.Title>
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
            <div className="flex items-start gap-5 max-[560px]:flex-col max-[560px]:items-center">
              <Cover
                work={work}
                imageUrl={image}
                className="aspect-[2/3] w-[240px] flex-none max-[560px]:w-[min(240px,62vw)]"
                letterClass="text-7xl"
              />
              <div className="flex min-w-0 flex-1 flex-col gap-3 max-[560px]:w-full">
                {state ? (
                  <>
                    <p className="text-[14px] leading-relaxed font-semibold text-text-primary">{describe(state)}</p>
                    {state.pending === "fetch" && <p className={hintClass}>표지 이미지를 받고 있어요.</p>}
                    {state.note && <p className="text-[13px] leading-relaxed text-text-secondary">{state.note.message}</p>}
                    {problem && (
                      <p className="text-[13px] leading-relaxed font-semibold text-focus" role="status">
                        {problem}
                      </p>
                    )}
                  </>
                ) : loadError ? (
                  <p role="alert" className="text-[13px] font-semibold text-urgent">
                    {loadError}
                  </p>
                ) : (
                  <p className={hintClass}>표지 상태를 불러오는 중이에요.</p>
                )}

                {state && (
                  <div className="flex flex-wrap gap-2">
                    <Button
                      type="button"
                      variant="ghost"
                      className={btnAction}
                      aria-expanded={searching}
                      onClick={() => setSearching((v) => !v)}
                      disabled={busy !== null}
                    >
                      AniList에서 고르기
                    </Button>
                    <Button
                      type="button"
                      variant="ghost"
                      className={btnAction}
                      onClick={() => fileRef.current?.click()}
                      disabled={busy !== null}
                    >
                      {busy === "upload" ? "올리는 중…" : "파일 올리기"}
                    </Button>
                    {problem && state.source === "anilist" && (
                      <Button
                        type="button"
                        variant="ghost"
                        className={btnAction}
                        onClick={() => void run("repair", (v) => changeArtwork(workId, v, "repair"))}
                        disabled={busy !== null || state.pending === "fetch"}
                      >
                        다시 받기
                      </Button>
                    )}
                    {state.mode !== "auto" ? (
                      <Button
                        type="button"
                        variant="ghost"
                        className={btnNeutral}
                        onClick={() => void run("auto", (v) => changeArtwork(workId, v, "auto"))}
                        disabled={busy !== null}
                      >
                        자동으로 되돌리기
                      </Button>
                    ) : (
                      state.source === null &&
                      state.pending === null && (
                        <Button
                          type="button"
                          variant="ghost"
                          className={btnNeutral}
                          onClick={() => void run("auto", (v) => changeArtwork(workId, v, "auto"))}
                          disabled={busy !== null}
                        >
                          자동으로 다시 찾기
                        </Button>
                      )
                    )}
                    {state.mode !== "disabled" && (
                      <Button
                        type="button"
                        variant="ghost"
                        className={btnDanger}
                        onClick={() => void run("clear", (v) => changeArtwork(workId, v, "clear"))}
                        disabled={busy !== null}
                      >
                        표지 비우기
                      </Button>
                    )}
                    <input
                      ref={fileRef}
                      type="file"
                      accept="image/jpeg,image/png,image/webp"
                      className="hidden"
                      onChange={(event) => {
                        const file = event.target.files?.[0];
                        // Choosing the same file again after a refusal must fire again.
                        event.target.value = "";
                        if (file) void run("upload", (v) => uploadArtwork(workId, v, file));
                      }}
                    />
                  </div>
                )}
                {state && (
                  <p className={hintClass}>
                    JPEG·PNG·WebP 이미지를 10MB까지 올릴 수 있어요. 파일 이름이 아니라 내용으로 판단해요. 표지를 비우면 자동으로도 찾지 않아요.
                  </p>
                )}
                {error && (
                  <p role="alert" className="text-[13px] leading-relaxed font-semibold text-urgent">
                    {error}
                  </p>
                )}
                {busy !== null && busy !== "upload" && busy !== "pick" && (
                  <p className={hintClass} role="status">
                    바꾸는 중이에요.
                  </p>
                )}
              </div>
            </div>

            {searching && state && (
              <AnilistSearch
                workId={workId}
                initial={work.title}
                busy={busy}
                onPick={(candidate) => void run("pick", (v) => pickArtwork(workId, v, candidate.id))}
              />
            )}
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

/** AniList's search, a page at a time; choosing an entry makes its cover the work's. */
function AnilistSearch({
  workId,
  initial,
  busy,
  onPick,
}: {
  workId: string;
  initial: string;
  busy: string | null;
  onPick: (candidate: AnilistCandidate) => void;
}) {
  const [query, setQuery] = useState(initial);
  const [results, setResults] = useState<{ q: string; items: AnilistCandidate[]; page: number; hasNext: boolean } | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [picked, setPicked] = useState<number | null>(null);

  const search = async (q: string, page: number) => {
    setLoading(true);
    setError(null);
    try {
      const answer = await searchAnilist(workId, q, page);
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

  // The folder name is searched as soon as the panel opens.
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
    <section aria-labelledby="anilist-title" className="flex flex-col gap-3 border-t border-hairline-soft pt-4">
      <h3 id="anilist-title" className="text-[13px] font-bold text-text-muted">
        AniList에서 고르기
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
        <ul aria-label="AniList 검색 결과" className="m-0 grid list-none grid-cols-[repeat(auto-fill,minmax(132px,1fr))] gap-3 p-0">
          {results.items.map((candidate) => (
            <li key={candidate.id} className="min-w-0">
              <CandidateButton
                candidate={candidate}
                disabled={busy !== null}
                choosing={busy === "pick" && picked === candidate.id}
                onPick={() => {
                  setPicked(candidate.id);
                  onPick(candidate);
                }}
              />
            </li>
          ))}
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

function CandidateButton({
  candidate,
  disabled,
  choosing,
  onPick,
}: {
  candidate: AnilistCandidate;
  disabled: boolean;
  choosing: boolean;
  onPick: () => void;
}) {
  const facts = [candidate.format, candidate.season_year?.toString()].filter(Boolean).join(" · ");
  const others = candidate.titles.filter((t) => t !== candidate.title);
  return (
    <button
      type="button"
      onClick={onPick}
      disabled={disabled}
      aria-label={`${candidate.title}${facts ? ` (${facts})` : ""} 표지로 정하기`}
      className="flex w-full flex-col gap-2 rounded-card bg-surface-2 p-2 text-left text-text-primary hover:bg-surface-3 disabled:cursor-default disabled:opacity-60 dark:bg-surface-2 dark:hover:bg-surface-3"
    >
      <span className="relative block aspect-[2/3] w-full overflow-hidden rounded-md bg-surface-3">
        {candidate.thumb_url ? (
          <img src={candidate.thumb_url} alt="" loading="lazy" decoding="async" referrerPolicy="no-referrer" className="absolute inset-0 size-full object-cover" />
        ) : (
          <span className="absolute inset-0 flex items-center justify-center px-2 text-center text-xs text-text-muted">표지 없음</span>
        )}
      </span>
      <Line>{candidate.title}</Line>
      {others.length > 0 && <span className="line-clamp-1 text-[11.5px] text-text-muted">{others.join(" / ")}</span>}
      <span className="text-[11.5px] text-text-secondary">
        {facts ? `${facts} · ` : ""}#{candidate.id}
      </span>
      {choosing && <span className="text-[11.5px] font-semibold text-focus">표지를 받는 중이에요</span>}
    </button>
  );
}

function Line({ children }: { children: ReactNode }) {
  return <span className="line-clamp-2 text-[13px] leading-snug font-semibold">{children}</span>;
}
