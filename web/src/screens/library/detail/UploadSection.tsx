import { useCallback, useEffect, useId, useMemo, useRef, useState } from "react";
import { Link } from "react-router-dom";

import { Button } from "@/components/ui/button";
import { ApiError } from "@/lib/api";
import { newCommandId } from "@/lib/commands";
import { useMediaQuery } from "@/lib/media";
import { cn } from "@/lib/utils";

import { btnAction, btnNeutral, hintClass, inputClass } from "../../collect/channels/styles";
import { jobPath, uploadSubtitles, type UploadResult } from "../../todo/api";
import { uploadKept } from "../../todo/format";
import type { CandidateList } from "../api";
import {
  KIND_LABEL,
  TOUCH_QUERY,
  bytesText,
  canPickFolder,
  countsText,
  entriesOf,
  planOf,
  signatureOf,
  type Entry,
} from "./upload";

/** The most rows of the chosen files the list draws; the rest are counted. */
const SHOWN_ROWS = 200;

/** Whether this browser's file `<input>` has the `webkitdirectory` property. */
const hasDirectoryInput = (): boolean => "webkitdirectory" in document.createElement("input");

/** The creator value for `제작자 알 수 없음`, which the select needs as a value of its own. */
const UNKNOWN = "";

type Phase =
  | { kind: "idle" }
  | { kind: "sending" }
  | { kind: "made"; result: UploadResult }
  /** `network`: how many answers in a row did not come at all. */
  | { kind: "unconfirmed"; message: string; network: number }
  | { kind: "refused"; message: string };

/** The one upload the user has not seen the answer to: its ID is kept so a resend makes no second job. */
interface Sent {
  id: string;
  signature: string;
}

/** The creators of the season's Anissia candidates, once each, in the order they appear. */
function creatorsOf(list: CandidateList | null): { sourceId: string; name: string }[] {
  const seen = new Map<string, string>();
  for (const c of list?.candidates ?? []) if (!seen.has(c.source_id)) seen.set(c.source_id, c.creator);
  return [...seen].map(([sourceId, name]) => ({ sourceId, name }));
}

/**
 * The `자막 올리기` section of the chosen season, below the candidates: files,
 * ZIPs or a folder of subtitles and fonts become one job. The list shows what
 * will be kept and what will be left out before anything is sent; the server
 * judges again by the content of each file, so the answer may leave out more.
 * Nothing to keep makes no job.
 */
export function UploadSection({
  workId,
  season,
  seasonCount,
  candidates,
}: {
  workId: string;
  season: number;
  seasonCount: number;
  candidates: CandidateList | null;
}) {
  const [entries, setEntries] = useState<Entry<File>[]>([]);
  const [creator, setCreator] = useState(UNKNOWN);
  const [phase, setPhase] = useState<Phase>({ kind: "idle" });
  const nextKey = useRef(0);
  const sent = useRef<Sent | null>(null);
  /** The sends of the unconfirmed request whose answer did not come at all, in a row. */
  const lost = useRef(0);
  const busy = useRef(false);
  const alive = useRef(true);
  const filesRef = useRef<HTMLInputElement>(null);
  const folderRef = useRef<HTMLInputElement>(null);
  const selectId = useId();
  const titleId = useId();

  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);

  const touchOnly = useMediaQuery(TOUCH_QUERY);
  const directoryInput = useMemo(hasDirectoryInput, []);
  const folder = canPickFolder({ hasDirectoryInput: directoryInput, touchOnly });
  const creators = useMemo(() => creatorsOf(candidates), [candidates]);
  const plan = useMemo(() => planOf(entries), [entries]);
  const sending = phase.kind === "sending";
  const total = entries.length;

  // A creator the list no longer has (the candidates were read again) is not kept as a choice.
  const chosen = creators.some((c) => c.sourceId === creator) ? creator : UNKNOWN;

  const add = (list: FileList | null) => {
    if (!list || list.length === 0) return;
    const added = entriesOf(Array.from(list), nextKey.current);
    lost.current = 0;
    nextKey.current += added.length;
    setEntries((prev) => [...prev, ...added]);
    setPhase((was) => (was.kind === "sending" ? was : { kind: "idle" }));
  };
  const remove = (key: number) => {
    lost.current = 0;
    setEntries((prev) => prev.filter((e) => e.key !== key));
  };
  const clear = () => {
    lost.current = 0;
    setEntries([]);
  };

  const send = useCallback(
    async (request: Sent) => {
      busy.current = true;
      setPhase({ kind: "sending" });
      try {
        const result = await uploadSubtitles({
          id: request.id,
          work_id: workId,
          season,
          creator: chosen === UNKNOWN ? null : chosen,
          files: plan.send.map((e) => ({ name: e.name, file: e.file })),
          skipped: plan.skip.map((e) => e.name),
        });
        sent.current = null;
        lost.current = 0;
        if (alive.current) {
          setPhase({ kind: "made", result });
          setEntries([]);
        }
      } catch (e) {
        // The server answered and said no (or that it is busy, having done nothing): nothing was made.
        const refused =
          e instanceof ApiError &&
          (e.code === "invalid" || e.code === "conflict" || e.code === "not_found" || e.code === "unavailable");
        if (refused) sent.current = null;
        // No answer at all: the connection broke, or the server refused a body it was still being sent.
        const network = e instanceof ApiError && e.code === "network";
        lost.current = network ? lost.current + 1 : 0;
        if (alive.current) {
          const message = e instanceof ApiError ? e.message : "올리지 못했어요.";
          setPhase(refused ? { kind: "refused", message } : { kind: "unconfirmed", message, network: lost.current });
        }
      } finally {
        busy.current = false;
      }
    },
    [workId, season, chosen, plan],
  );

  const upload = () => {
    if (busy.current || plan.send.length === 0 || plan.problems.length > 0) return;
    const signature = signatureOf(chosen, plan.send, plan.skip);
    const kept = sent.current;
    const request = kept && kept.signature === signature ? kept : (sent.current = { id: newCommandId(), signature });
    void send(request);
  };
  const title = seasonCount > 1 ? `시즌 ${season} 자막 올리기` : "자막 올리기";
  const nothing = total > 0 && plan.send.length === 0;

  return (
    <section aria-labelledby={titleId} data-testid="upload-section">
      <div className="flex flex-wrap items-center gap-x-2.5 gap-y-2">
        <h2 id={titleId} tabIndex={-1} className="text-[17px] font-bold outline-none">
          {title}
        </h2>
      </div>
      <p className={cn(hintClass, "m-0 mt-1.5 text-[12.5px]")}>
        자막과 폰트만 남겨서 작업 하나로 받아요. 다른 파일은 빼고, 뺀 파일의 이름과 이유는 작업에서 볼 수 있어요. 압축 파일은 그대로 받은 뒤 풀어서 안의 파일을 나눠요.
      </p>

      <div className="mt-3 flex flex-wrap items-center gap-2">
        <input
          ref={filesRef}
          type="file"
          multiple
          className="sr-only"
          tabIndex={-1}
          aria-label="올릴 파일 고르기"
          data-testid="upload-files"
          onChange={(e) => {
            add(e.target.files);
            e.target.value = "";
          }}
        />
        <Button type="button" variant="ghost" className={btnNeutral} disabled={sending} onClick={() => filesRef.current?.click()}>
          파일 고르기
        </Button>
        {folder && (
          <>
            <input
              ref={(el) => {
                folderRef.current = el;
                // React does not know the attribute; the browser reads the property.
                el?.setAttribute("webkitdirectory", "");
              }}
              type="file"
              className="sr-only"
              tabIndex={-1}
              aria-label="올릴 폴더 고르기"
              data-testid="upload-folder"
              onChange={(e) => {
                add(e.target.files);
                e.target.value = "";
              }}
            />
            <Button type="button" variant="ghost" className={btnNeutral} disabled={sending} onClick={() => folderRef.current?.click()}>
              폴더 고르기
            </Button>
          </>
        )}
        {total > 0 && (
          <Button type="button" variant="ghost" className={btnNeutral} disabled={sending} onClick={clear}>
            목록 비우기
          </Button>
        )}
        {!folder && <span className={hintClass}>이 기기에서는 파일과 ZIP만 올릴 수 있어요.</span>}
      </div>

      <div className="mt-3 flex flex-wrap items-center gap-x-3 gap-y-2">
        <label htmlFor={selectId} className="text-[12.5px] font-semibold text-text-secondary">
          제작자
        </label>
        <select
          id={selectId}
          value={chosen}
          disabled={sending}
          onChange={(e) => {
            lost.current = 0;
            setCreator(e.target.value);
          }}
          className={cn(inputClass, "w-[240px] max-w-full border py-1")}
        >
          <option value={UNKNOWN}>제작자 알 수 없음</option>
          {creators.map((c) => (
            <option key={c.sourceId} value={c.sourceId}>
              {c.name}
            </option>
          ))}
        </select>
      </div>

      {total > 0 && (
        <div className="mt-3">
          <p className="m-0 text-[13px] font-semibold text-text-secondary" data-testid="upload-summary">
            {plan.send.length > 0 ? `올릴 파일: ${countsText(plan.counts)} · ${bytesText(plan.totalBytes)}` : "올릴 파일이 없어요."}
            {plan.skip.length > 0 && <span className="font-normal text-text-muted"> · 뺄 파일 {plan.skip.length}개</span>}
          </p>
          <ul className="m-0 mt-2 max-h-72 list-none overflow-y-auto rounded-card border border-hairline-soft bg-surface-1 p-0 shadow-(--card-shadow)">
            {entries.slice(0, SHOWN_ROWS).map((entry) => (
              <li
                key={entry.key}
                className="flex items-start gap-2 border-t border-hairline-soft px-3 py-1.5 first:border-t-0"
                data-testid="upload-row"
              >
                <span className="flex min-w-0 flex-1 flex-col gap-0.5">
                  <span className="text-[13px] leading-snug font-semibold [overflow-wrap:anywhere]">{entry.name}</span>
                  <span className={cn("text-xs", entry.kind === null ? "text-text-secondary" : "text-text-muted")}>
                    {entry.kind === null
                      ? "빼요 · 이름으로 보아 자막이나 폰트가 아니에요"
                      : `${KIND_LABEL[entry.kind]} · ${bytesText(entry.file.size)}${entry.kind === "archive" ? " · 받은 뒤 풀어요" : ""}`}
                  </span>
                </span>
                <button
                  type="button"
                  disabled={sending}
                  onClick={() => remove(entry.key)}
                  className="flex-none rounded-md px-2 py-1 text-xs font-semibold text-text-secondary hover:text-text-primary disabled:opacity-50 max-[720px]:min-h-10"
                >
                  빼기<span className="sr-only"> {entry.name}</span>
                </button>
              </li>
            ))}
          </ul>
          {total > SHOWN_ROWS && <p className={cn(hintClass, "m-0 mt-1.5")}>처음 {SHOWN_ROWS}개만 보여요. 나머지 {total - SHOWN_ROWS}개도 같이 올라가요.</p>}
          {plan.skip.length > 0 && plan.send.length > 0 && (
            <p className={cn(hintClass, "m-0 mt-1.5")}>
              뺄 파일은 이름만 서버에 알려요. 올리는 파일도 서버가 내용을 다시 살펴서, 이름만 자막인 파일은 더 뺄 수 있어요.
            </p>
          )}
        </div>
      )}

      {plan.problems.length > 0 && (
        <ul role="alert" className="m-0 mt-2 list-none p-0">
          {plan.problems.map((problem) => (
            <li key={problem} className="text-[13px] font-semibold text-urgent">
              {problem}
            </li>
          ))}
        </ul>
      )}
      {nothing && (
        <p role="status" className="m-0 mt-2 text-[13px] font-semibold text-urgent">
          올릴 자막이나 폰트가 없어요. 고른 파일은 모두 빼야 해서 작업을 만들지 않아요.
        </p>
      )}

      <div className="mt-3 flex flex-wrap items-center gap-2">
        <Button
          type="button"
          variant="ghost"
          className={btnAction}
          disabled={sending || plan.send.length === 0 || plan.problems.length > 0}
          onClick={upload}
        >
          {sending ? "올리는 중…" : "올리기"}
        </Button>
        {sending && <span className={hintClass}>파일이 클수록 오래 걸려요. 이 화면을 닫지 말아 주세요.</span>}
      </div>

      <div className="mt-2 empty:hidden">
        <UploadStatus phase={phase} onResend={upload} />
      </div>
    </section>
  );
}

function UploadStatus({ phase, onResend }: { phase: Phase; onResend: () => void }) {
  switch (phase.kind) {
    case "idle":
    case "sending":
      return null;
    case "made": {
      const { result } = phase;
      const kept = result.kept ? uploadKept(result.kept) : null;
      const dropped = result.dropped ?? [];
      return (
        <div role="status" className="flex flex-col gap-1.5 text-[12.5px] text-text-secondary">
          <p className="m-0">
            {kept !== null ? `올린 파일: ${kept}.` : "올렸어요."}{" "}
            <Link to={jobPath(result.id)} className="font-semibold underline underline-offset-2">
              작업 보기
            </Link>
          </p>
          {dropped.length > 0 && (
            <div>
              <p className="m-0 font-semibold">뺀 파일 {dropped.length}개</p>
              <ul className="m-0 mt-1 list-none p-0">
                {dropped.slice(0, 20).map((file, i) => (
                  <li key={`${file.name}:${i}`} className="[overflow-wrap:anywhere]">
                    {file.name} <span className="text-text-muted">· {file.reason}</span>
                  </li>
                ))}
              </ul>
              {dropped.length > 20 && <p className="m-0 mt-1 text-text-muted">나머지 {dropped.length - 20}개는 작업에서 볼 수 있어요.</p>}
            </div>
          )}
        </div>
      );
    }
    case "unconfirmed":
      // Two answers in a row that never came: the same upload keeps failing the same way (a body the server
      // refuses while it is still being sent shows as a broken connection), so sending it again is not offered.
      return phase.network >= 2 ? (
        <p role="alert" className="m-0 text-[12.5px] leading-snug text-text-secondary [overflow-wrap:anywhere]">
          서버에서 답이 오지 않았어요. 두 번 이어서 같은 일이 생겼어요. 작업을 만들었는지 알 수 없으니 먼저{" "}
          <Link to="/todo" className="font-semibold underline underline-offset-2">
            할 일의 자막 작업
          </Link>
          에서 확인해 주세요. 없으면 파일 수나 크기를 줄여서 나눠 올려 보세요.
        </p>
      ) : (
        <div role="alert" className="flex flex-wrap items-center gap-x-3 gap-y-1.5">
          <p className="m-0 min-w-0 text-[12.5px] leading-snug text-text-secondary [overflow-wrap:anywhere]">
            {phase.message} 작업을 만들었는지 알 수 없어요. 같은 파일을 다시 보내도 작업은 하나만 생겨요.
            {phase.network === 1 && " 서버가 올리는 도중에 거절했을 수도 있어요. 한도를 넘지 않았는지 확인해 보세요."}
          </p>
          <Button type="button" variant="ghost" className={btnAction} onClick={onResend}>
            다시 보내기
          </Button>
        </div>
      );
    case "refused":
      return (
        <p role="alert" className="m-0 text-[12.5px] leading-snug font-semibold text-urgent [overflow-wrap:anywhere]">
          {phase.message}
        </p>
      );
  }
}
