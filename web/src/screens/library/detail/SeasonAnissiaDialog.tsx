import { useRef, useState, type FormEvent } from "react";
import { Dialog } from "radix-ui";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { ApiError } from "@/lib/api";
import { cn } from "@/lib/utils";

import { PickAnime } from "../../collect/subs/add/PickAnime";
import { todayWeek } from "../../collect/subs/format";
import type { ScheduleEntry } from "../../collect/subs/api";
import { btnNeutral, btnPrimary, hintClass, inputClass } from "../../collect/channels/styles";
import { searchAnissia, setAnissiaLink, type AnissiaCandidate, type AnissiaLink, type AnissiaSource, type ReferenceTitle } from "../api";
import { anissiaStatusText as statusText } from "./model";

type Tab = "list" | "schedule";

/** The anime picked, and where it was picked from (the server checks it against that list). */
interface Picked {
  anime_no: number;
  subject: string;
  source: AnissiaSource;
  /** The schedule's entry, to show it picked on its tab. */
  entry?: ScheduleEntry;
}

/** An anime of the full list with the page it was listed on. */
interface Row {
  candidate: AnissiaCandidate;
  page: number;
}

const dates = (c: AnissiaCandidate) => {
  const start = c.start_date ?? "";
  const end = c.end_date ?? "";
  return start || end ? `${start} ~ ${end}` : "";
};

/**
 * Picks the Anissia anime a season is linked to: from Anissia's full list,
 * finished anime included (the user writes the query, reading the season's
 * reference titles; nothing is searched on open), or from this quarter's schedule. Nothing is saved until `연결`;
 * when someone changed the link first, the dialog shows the current link and
 * keeps the search and the pick.
 */
export function SeasonAnissiaDialog({
  workId,
  link,
  open,
  onOpenChange,
  onChanged,
}: {
  workId: string;
  link: AnissiaLink;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onChanged: (link: AnissiaLink) => void;
}) {
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
            <Dialog.Title className="min-w-0 flex-1 truncate text-[17px] font-bold">시즌 {link.season} Anissia 연결</Dialog.Title>
            <Dialog.Close
              aria-label="닫기"
              className="inline-flex size-11 flex-none items-center justify-center rounded-full text-text-secondary hover:bg-surface-2 hover:text-text-primary"
            >
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" aria-hidden="true" focusable="false" className="size-5">
                <path d="M6 6l12 12M18 6L6 18" />
              </svg>
            </Dialog.Close>
          </div>
          <Body workId={workId} link={link} onClose={() => onOpenChange(false)} onChanged={onChanged} />
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

function Body({
  workId,
  link,
  onClose,
  onChanged,
}: {
  workId: string;
  link: AnissiaLink;
  onClose: () => void;
  onChanged: (link: AnissiaLink) => void;
}) {
  const [tab, setTab] = useState<Tab>("list");
  const [picked, setPicked] = useState<Picked | null>(null);
  const [week, setWeek] = useState(() => todayWeek());
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const linkRef = useRef(link);
  linkRef.current = link;

  const save = async () => {
    if (picked === null || busy) return;
    setBusy(true);
    setError(null);
    try {
      onChanged(await setAnissiaLink(workId, link.season, linkRef.current.version, picked.anime_no, picked.source));
      onClose();
    } catch (e) {
      if (e instanceof ApiError && e.code === "conflict" && e.current) {
        // The screen shows the current link; the search and the pick stay.
        onChanged(e.current as AnissiaLink);
        setError("다른 곳에서 먼저 바꿨어요. 지금 연결을 위에 보여드려요. 그대로 연결하려면 다시 눌러 주세요.");
      } else {
        setError(e instanceof ApiError ? e.message : "연결하지 못했어요. 잠시 뒤 다시 시도해 주세요.");
      }
    } finally {
      setBusy(false);
    }
  };

  const tabs: { id: Tab; label: string }[] = [
    { id: "list", label: "전체 목록" },
    { id: "schedule", label: "편성표" },
  ];

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-4 overflow-x-hidden overflow-y-auto overscroll-contain p-4 pb-[calc(16px+env(safe-area-inset-bottom,0px))]">
      <p className="text-[13px] leading-relaxed text-text-secondary" data-testid="anissia-current">
        {link.anime ? (
          <>
            지금 연결: <strong className="font-semibold break-words">{link.anime.subject}</strong> ({statusText(link.anime.status)})
          </>
        ) : (
          "지금 연결한 작품이 없어요."
        )}
      </p>

      <div role="group" aria-label="찾는 곳" className="flex gap-1.5">
        {tabs.map((t) => (
          <button
            key={t.id}
            type="button"
            aria-pressed={tab === t.id}
            onClick={() => setTab(t.id)}
            className={cn(
              "min-h-9 rounded-full border px-4 text-[13px] font-semibold outline-offset-2 focus-visible:outline-2 focus-visible:outline-focus max-[720px]:min-h-10",
              tab === t.id ? "border-focus bg-focus text-primary-foreground" : "border-hairline bg-surface-1 text-text-primary hover:border-text-secondary",
            )}
          >
            {t.label}
          </button>
        ))}
      </div>

      {/* Both tabs stay mounted, so a search and its place survive switching. */}
      <div hidden={tab !== "list"}>
        <FullList workId={workId} season={link.season} titles={link.reference_titles} picked={picked} onPick={setPicked} />
      </div>
      <div hidden={tab !== "schedule"}>
        <PickAnime
          heading="이번 분기 편성표에서 골라요"
          week={week}
          selected={picked?.entry ?? null}
          onWeek={setWeek}
          onPick={(entry: ScheduleEntry) => setPicked({ anime_no: entry.anime_no, subject: entry.subject, source: { week }, entry })}
        />
      </div>

      <div className="flex flex-col gap-2 border-t border-hairline-soft pt-3">
        <p className={hintClass} role="status">
          {picked ? `고른 작품: ${picked.subject}` : "연결할 작품을 골라 주세요."}
        </p>
        {error && (
          <p role="alert" className="text-[13px] leading-relaxed font-semibold text-urgent">
            {error}
          </p>
        )}
        <div className="flex flex-wrap gap-2">
          <Button type="button" variant="ghost" className={btnPrimary} disabled={picked === null || busy} onClick={() => void save()}>
            {busy ? "연결하는 중…" : "연결"}
          </Button>
          <Button type="button" variant="ghost" className={btnNeutral} disabled={busy} onClick={onClose}>
            취소
          </Button>
        </div>
      </div>
    </div>
  );
}

/** Anissia's full list, a page at a time. Its failure is its own: the schedule tab is not affected. */
function FullList({
  workId,
  season,
  titles,
  picked,
  onPick,
}: {
  workId: string;
  season: number;
  titles: readonly ReferenceTitle[];
  picked: Picked | null;
  onPick: (picked: Picked) => void;
}) {
  // Nothing is searched until the user writes a query.
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<{ q: string; rows: Row[]; page: number; hasNext: boolean; maxPage: number } | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const search = async (q: string, page: number) => {
    setLoading(true);
    setError(null);
    try {
      const answer = await searchAnissia(workId, season, q, page);
      const rows = answer.items.map((candidate) => ({ candidate, page: answer.page }));
      setResults((prev) => ({
        q: answer.q,
        rows: page > 1 && prev ? [...prev.rows, ...rows] : rows,
        page: answer.page,
        hasNext: answer.has_next,
        maxPage: answer.max_page,
      }));
    } catch (e) {
      setError(e instanceof ApiError ? e.message : "Anissia 전체 목록을 검색하지 못했어요. 편성표에서는 고를 수 있어요.");
    } finally {
      setLoading(false);
    }
  };

  const submit = (event: FormEvent) => {
    event.preventDefault();
    const q = query.trim();
    if (q !== "") void search(q, 1);
  };

  return (
    <section aria-label="Anissia 전체 목록" className="flex flex-col gap-3">
      <ReferenceTitles titles={titles} />
      <form onSubmit={submit} className="flex gap-2">
        <Input
          aria-label="Anissia 검색어"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="한국어 제목으로 검색"
          className={cn(inputClass, "min-w-0 flex-1")}
        />
        <Button type="submit" variant="ghost" className={btnNeutral} disabled={loading || query.trim() === ""}>
          검색
        </Button>
      </form>
      <p className={hintClass}>방영이 끝난 작품도 찾아요. Anissia는 한국어 제목으로 찾아져서, 일본어나 영어 제목은 거의 맞지 않아요. 위 제목을 참고해 한국어 제목을 직접 써 보세요. 전에 받아 둔 시즌은 ‘정보 다시 받기’를 하면 AniList의 한국어 별칭도 받아 와요(AniList에 있는 작품만요).</p>
      {error && (
        <p role="alert" className="text-[13px] leading-relaxed font-semibold text-urgent">
          {error}
        </p>
      )}
      {results && results.rows.length === 0 && !loading && !error && (
        <p className={hintClass}>‘{results.q}’로 찾은 Anissia 작품이 없어요. 다른 이름으로 찾아 보세요.</p>
      )}
      {results && results.rows.length > 0 && (
        <ul aria-label="Anissia 검색 결과" className="m-0 flex list-none flex-col gap-2 p-0">
          {results.rows.map(({ candidate: c, page }) => {
            const on = picked?.anime_no === c.anime_no;
            return (
              <li key={c.anime_no}>
                <button
                  type="button"
                  aria-pressed={on}
                  onClick={() => onPick({ anime_no: c.anime_no, subject: c.subject, source: { q: results.q, page } })}
                  className={cn(
                    "flex w-full min-w-0 flex-col gap-0.5 rounded-[10px] border bg-surface-2 px-3 py-2 text-left outline-offset-2 focus-visible:outline-2 focus-visible:outline-focus dark:bg-surface-2",
                    on ? "border-focus" : "border-hairline-soft",
                  )}
                >
                  <span className="flex min-w-0 flex-wrap items-baseline gap-x-2 gap-y-1">
                    <span className="min-w-0 text-[14px] leading-snug font-semibold break-words">{c.subject}</span>
                    <span className="flex-none rounded-full border border-hairline px-2 py-px text-[11.5px] font-semibold text-text-secondary">
                      {statusText(c.status)}
                    </span>
                  </span>
                  {c.original_subject && <span className="text-xs leading-snug break-words text-text-muted">{c.original_subject}</span>}
                  {dates(c) && <span className="text-xs text-text-secondary">{dates(c)}</span>}
                </button>
              </li>
            );
          })}
        </ul>
      )}
      {loading && <p className={hintClass}>Anissia에서 찾는 중이에요.</p>}
      {results?.hasNext && results.page < results.maxPage && !loading && (
        <Button type="button" variant="ghost" className={cn(btnNeutral, "self-start")} onClick={() => void search(results.q, results.page + 1)}>
          결과 더 보기
        </Button>
      )}
    </section>
  );
}

const KIND_LABELS: Record<ReferenceTitle["kind"], string> = {
  native: "원제",
  english: "영어",
  romaji: "로마자",
  korean: "한국어",
  folder: "폴더 이름",
};

/** The season's titles as plain text to read (and copy) when writing a search; nothing here searches. */
function ReferenceTitles({ titles }: { titles: readonly ReferenceTitle[] }) {
  if (titles.length === 0) return null;
  return (
    <div className="flex flex-col gap-1.5" data-testid="reference-titles">
      <h3 className="text-[12.5px] font-semibold text-text-secondary">참고할 제목</h3>
      <dl className="m-0 grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-1 rounded-[10px] bg-surface-2 px-3 py-2 text-[13px] leading-snug dark:bg-surface-2">
        {titles.map((t) => (
          <div key={`${t.kind}:${t.title}`} className="contents">
            <dt className="text-xs text-text-muted">{KIND_LABELS[t.kind]}</dt>
            <dd className="m-0 break-words text-text-primary select-text">{t.title}</dd>
          </div>
        ))}
      </dl>
    </div>
  );
}
