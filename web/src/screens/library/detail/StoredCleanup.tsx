import { useEffect, useId, useMemo, useRef, useState } from "react";

import { Button } from "@/components/ui/button";
import { ApiError } from "@/lib/api";
import { forget } from "@/lib/cached";
import { cn } from "@/lib/utils";

import { btnDanger, btnDangerSolid, btnNeutral } from "../../collect/channels/styles";
import { cleanStored, STORAGE_KEY } from "../api";
import {
  CHANGED_NOTICE,
  canClean,
  cleanBody,
  cleanFailure,
  cleaningText,
  confirmView,
  creatorText,
  episodeText,
  groupByKind,
  isCleaning,
  receivedText,
  sentNotice,
  storageSignature,
  totalText,
  type CleanableEntry,
  type CleaningEntry,
  type SentClean,
  type WorkStorage,
} from "../storage.ts";
import { sizeText } from "../../todo/bytes.ts";

/** How often the page reads the work while a clean is waiting for the worker. */
const POLL_MS = 3000;

/** At least 44px tall on a phone. */
const touch = "max-[720px]:min-h-11";

const subHeading = "text-xs font-bold text-text-muted";

/** What is confirmed: the entry as the confirmation shows it, and whether it is a new list after a change. */
interface Confirming {
  entry: CleanableEntry;
  changed: boolean;
}

/**
 * The `정리할 수 있는 파일` part of the 파일 card: the work's stored size, then the stored subtitles that can be
 * cleaned, by kind. A clean is one file at a time and asked in place, naming every file it deletes and the linked
 * files that stay; nothing selects several entries. The page reads the work again while the worker has a clean to
 * carry out.
 */
export function StoredCleanup({
  workId,
  storage,
  seasons,
  onRefresh,
}: {
  workId: string;
  storage: WorkStorage;
  seasons: number;
  /** Reads the work again and shows it. */
  onRefresh: () => Promise<void>;
}) {
  const [confirming, setConfirming] = useState<Confirming | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [sent, setSent] = useState<SentClean | null>(null);
  const heading = useRef<HTMLHeadingElement>(null);
  const shown = notice ?? sentNotice(sent, storage);
  const groups = useMemo(() => groupByKind(storage.cleanable), [storage.cleanable]);

  // The confirmation goes when its entry is no longer on the list (cleaned, or gone by another way).
  const open = confirming && storage.cleanable.some((e) => e.id === confirming.entry.id) ? confirming : null;

  const refresh = useRef(onRefresh);
  refresh.current = onRefresh;
  const quietRefresh = async () => {
    try {
      await refresh.current();
    } catch {
      // The page keeps what it shows; the next read (or a reload) brings the rest.
    }
  };

  // A clean that the worker has not finished: read the work again now and then.
  const asked = isCleaning(storage);
  useEffect(() => {
    if (!asked) return;
    let stopped = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const tick = async () => {
      if (!document.hidden) await quietRefresh();
      if (!stopped) timer = setTimeout(() => void tick(), POLL_MS);
    };
    timer = setTimeout(() => void tick(), POLL_MS);
    return () => {
      stopped = true;
      clearTimeout(timer);
    };
  }, [asked]);

  // The settings list shows the same sizes and counts: it is read again when they change.
  const signature = storageSignature(storage);
  const seen = useRef(signature);
  useEffect(() => {
    if (seen.current === signature) return;
    seen.current = signature;
    forget(STORAGE_KEY);
  }, [signature]);

  const empty = storage.cleanable.length === 0 && storage.cleaning.length === 0;
  return (
    <div className="flex flex-col gap-2.5 border-t border-hairline-soft pt-3">
      <h3 ref={heading} tabIndex={-1} className={cn(subHeading, "outline-none")}>
        정리할 수 있는 파일
      </h3>
      <p className="text-[13px] font-semibold">{totalText(storage.total)}</p>
      {shown !== null && (
        <p role="status" className="text-[12.5px] leading-relaxed text-text-secondary">
          {shown}
        </p>
      )}
      {empty && <p className="text-[13px] text-text-muted">정리할 수 있는 파일이 없어요.</p>}
      {storage.cleanable.length > 0 && (
        <p className="text-xs leading-relaxed text-text-muted">정리는 파일 하나씩 해요. 지운 파일은 되돌릴 수 없어요.</p>
      )}
      {groups.map((group) => (
        <section key={group.kind} className="flex flex-col gap-1.5" aria-label={group.label}>
          <h4 className="text-[12.5px] font-bold text-text-secondary">
            {group.label} <span className="font-medium text-text-muted">{group.entries.length}개</span>
          </h4>
          <ul className="m-0 flex list-none flex-col gap-2 p-0">
            {group.entries.map((entry) => (
              <li key={entry.id} className="min-w-0">
                <EntryRow
                  workId={workId}
                  entry={entry}
                  all={storage.cleanable}
                  seasons={seasons}
                  confirming={open?.entry.id === entry.id ? open : null}
                  onAsk={() => {
                    setNotice(null);
                    setSent(null);
                    setConfirming({ entry, changed: false });
                  }}
                  onConfirming={setConfirming}
                  onClose={() => setConfirming(null)}
                  onNotice={setNotice}
                  onSent={(next) => {
                    setSent(next);
                    // The question and its row are gone: the focus goes back to the part's title.
                    heading.current?.focus();
                  }}
                  onRefresh={quietRefresh}
                />
              </li>
            ))}
          </ul>
        </section>
      ))}
      {storage.cleaning.length > 0 && (
        <section className="flex flex-col gap-1.5" aria-label="정리를 맡긴 파일">
          <h4 className="text-[12.5px] font-bold text-text-secondary">정리를 맡긴 파일</h4>
          <ul className="m-0 flex list-none flex-col gap-2 p-0">
            {storage.cleaning.map((entry) => (
              <li key={entry.id} className="min-w-0">
                <CleaningRow entry={entry} />
              </li>
            ))}
          </ul>
        </section>
      )}
    </div>
  );
}

const rowBox = "flex flex-col gap-1.5 rounded-[10px] border border-hairline-soft bg-surface-2 px-3 py-2.5";

function CleaningRow({ entry }: { entry: CleaningEntry }) {
  return (
    <div className={rowBox}>
      <span title={entry.name} className="font-mono text-[12px] leading-snug [overflow-wrap:anywhere]">
        {entry.name}
      </span>
      <span
        role={entry.state === "asked" ? "status" : undefined}
        className={cn("text-xs leading-snug", entry.state === "asked" ? "text-text-muted" : "font-semibold text-text-secondary")}
      >
        {cleaningText(entry)}
      </span>
    </div>
  );
}

function EntryRow({
  workId,
  entry,
  all,
  seasons,
  confirming,
  onAsk,
  onConfirming,
  onClose,
  onNotice,
  onSent,
  onRefresh,
}: {
  workId: string;
  entry: CleanableEntry;
  all: readonly CleanableEntry[];
  seasons: number;
  confirming: Confirming | null;
  onAsk: () => void;
  onConfirming: (next: Confirming) => void;
  onClose: () => void;
  onNotice: (text: string | null) => void;
  /** The clean was taken: the page says how it ends. */
  onSent: (sent: SentClean) => void;
  onRefresh: () => Promise<void>;
}) {
  const facts = [episodeText(entry, seasons), creatorText(entry.creator), sizeText(entry.size), receivedText(entry, all)];
  return (
    <div className={rowBox}>
      <span title={entry.name} className="font-mono text-[12px] leading-snug [overflow-wrap:anywhere]">
        {entry.name}
      </span>
      <span className="flex flex-wrap gap-x-3 gap-y-0.5 text-[12px] leading-snug text-text-muted">
        {facts.map((fact, index) => (
          <span key={index}>{fact}</span>
        ))}
      </span>
      {confirming ? (
        <CleanConfirm
          workId={workId}
          confirming={confirming}
          onConfirming={onConfirming}
          onClose={onClose}
          onNotice={onNotice}
          onSent={onSent}
          onRefresh={onRefresh}
        />
      ) : canClean(entry) ? (
        <span>
          <Button type="button" variant="ghost" className={cn(btnDanger, touch)} onClick={onAsk}>
            정리
          </Button>
        </span>
      ) : (
        <span className="text-xs leading-snug text-text-secondary">{entry.blocked}</span>
      )}
    </div>
  );
}

/** The question in place: every file the clean deletes, the linked files that stay, and the two buttons. */
function CleanConfirm({
  workId,
  confirming,
  onConfirming,
  onClose,
  onNotice,
  onSent,
  onRefresh,
}: {
  workId: string;
  confirming: Confirming;
  onConfirming: (next: Confirming) => void;
  onClose: () => void;
  onNotice: (text: string | null) => void;
  /** The clean was taken: the page says how it ends. */
  onSent: (sent: SentClean) => void;
  onRefresh: () => Promise<void>;
}) {
  const uid = useId();
  const box = useRef<HTMLDivElement>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const { entry, changed } = confirming;
  const view = useMemo(() => confirmView(entry), [entry]);

  // The question takes focus when it opens and again when it shows a new list.
  useEffect(() => {
    box.current?.focus();
  }, [entry]);

  const submit = async () => {
    if (busy || view.blocked !== null) return;
    setBusy(true);
    setError(null);
    let answer: { cleanup_id: string };
    try {
      answer = await cleanStored(workId, entry.id, cleanBody(entry).assets);
    } catch (e) {
      setBusy(false);
      const failure = cleanFailure(
        e instanceof ApiError ? e : { code: "network", message: "정리를 요청하지 못했어요. 잠시 뒤 다시 시도해 주세요." },
      );
      if (failure.kind === "changed") {
        // The files changed: the question shows the new list, and the page reads the work again behind it.
        onConfirming({ entry: failure.entry, changed: true });
        void onRefresh();
      } else if (failure.kind === "message") {
        setError(failure.message);
      } else {
        onClose();
        onNotice(failure.message);
        await onRefresh();
      }
      return;
    }
    forget(STORAGE_KEY);
    onNotice(null);
    // Read first: the row of the clean (`정리하는 중`) then says how it goes, or the notice that it ended.
    await onRefresh();
    setBusy(false);
    onClose();
    onSent({ id: answer.cleanup_id, entryId: entry.id, name: entry.name });
  };

  return (
    <div
      ref={box}
      tabIndex={-1}
      role="group"
      aria-labelledby={`${uid}-title`}
      className="flex flex-col gap-2.5 rounded-xl border border-hairline bg-surface-1 p-3 outline-offset-2"
    >
      <p id={`${uid}-title`} className="m-0 text-sm font-bold">
        이 보관본을 정리할까요?
      </p>
      {changed && (
        <p role="status" className="m-0 text-[12.5px] leading-relaxed font-semibold text-text-primary">
          {CHANGED_NOTICE}
        </p>
      )}
      {view.blocked !== null ? (
        <p className="m-0 text-[13px] leading-relaxed text-text-secondary">{view.blocked}</p>
      ) : (
        <>
          <div className="flex flex-col gap-1.5">
            <h5 className={subHeading}>지우는 파일 {view.deletes.length}개</h5>
            {view.deletes.length === 0 ? (
              <p className="m-0 text-[12.5px] leading-relaxed text-text-secondary">지울 파일이 없고 이 보관본의 기록만 정리해요.</p>
            ) : (
              <ul className="m-0 flex list-none flex-col gap-1.5 p-0">
                {view.deletes.map((line) => (
                  <li key={line.id} className="flex min-w-0 flex-col gap-0.5">
                    <span className="font-mono text-[12px] leading-snug [overflow-wrap:anywhere]">{line.name}</span>
                    <span className="flex gap-x-3 text-[12px] text-text-muted">
                      <span>{line.kindLabel}</span>
                      <span>{line.sizeText}</span>
                    </span>
                  </li>
                ))}
              </ul>
            )}
            {view.deletes.length > 0 && <p className="m-0 text-[12.5px] font-semibold">합계 {view.totalText}</p>}
            {view.subtitleStays && (
              <p className="m-0 text-[12.5px] leading-relaxed text-text-secondary">
                자막 파일은 다른 보관본과 내용이 같아서 지우지 않아요.
              </p>
            )}
          </div>
          {view.kept.length > 0 && (
            <div className="flex flex-col gap-1.5">
              <h5 className={subHeading}>남는 파일 {view.kept.length}개</h5>
              <ul className="m-0 flex list-none flex-col gap-1.5 p-0">
                {view.kept.map((line) => (
                  <li key={line.id} className="flex min-w-0 flex-col gap-0.5">
                    <span className="font-mono text-[12px] leading-snug [overflow-wrap:anywhere]">{line.name}</span>
                    <span className="text-[12px] leading-snug text-text-muted">
                      {line.kindLabel}, {line.reason}
                    </span>
                  </li>
                ))}
              </ul>
            </div>
          )}
          {view.deletes.length > 0 && (
            <p className="m-0 text-[12.5px] leading-relaxed text-text-secondary">
              파일은 작품 폴더의 .trss/나 앱 데이터 폴더에서 지워요. 지운 파일은 되돌릴 수 없어요.
            </p>
          )}
        </>
      )}
      {error !== null && (
        <p role="alert" className="m-0 text-[12.5px] leading-relaxed font-semibold text-urgent">
          {error}
        </p>
      )}
      <div className="flex flex-wrap gap-2">
        {view.blocked === null && (
          <Button type="button" variant="ghost" className={cn(btnDangerSolid, touch)} disabled={busy} onClick={() => void submit()}>
            {busy ? "정리하는 중…" : "정리"}
          </Button>
        )}
        <Button type="button" variant="ghost" className={cn(btnNeutral, touch)} disabled={busy} onClick={onClose}>
          {view.blocked === null ? "취소" : "닫기"}
        </Button>
      </div>
    </div>
  );
}
