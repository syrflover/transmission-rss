import { useEffect, useId, useRef, useState, type FormEvent } from "react";

import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { ApiError } from "@/lib/api";
import { useCached } from "@/lib/cached";
import { ago, dateTime } from "@/lib/time";
import { KEYS } from "@/screens/collect/cache";
import { forgetLibrary } from "@/screens/library/api";
import { hintClass, inputClass, labelClass } from "@/screens/collect/channels/styles";

import { Labeled, BTN, Facts } from "../parts";
import {
  addWatchFolder,
  loadWatchFolders,
  removeWatchFolder,
  type WatchFolder,
  type WatchFolderList,
} from "./api";
import { useRescan } from "./useRescan";

const LOAD_FAILED = "감시 폴더를 불러오지 못했어요.";

/**
 * `감시 폴더`: the folders the app reads for works. Each row has the counts of
 * what was found there, `다시 확인` (a web command the worker carries out) and
 * `등록 해제` (asked in place; no file is touched).
 */
export function FoldersPanel() {
  const folders = useCached<WatchFolderList>(KEYS.watchFolders, loadWatchFolders, LOAD_FAILED);
  const data = folders.data;
  // The library list and the work pages carry what these folders hold: they are read again the next time they open.
  const reload = () => {
    forgetLibrary();
    folders.reload();
  };

  return (
    <div className="flex flex-col gap-5 p-5 max-[720px]:p-4">
      <header className="flex flex-col gap-1.5">
        <h2 id="panel-title" tabIndex={-1} className="m-0 text-xl font-bold outline-none">
          감시 폴더
        </h2>
        <p className="max-w-[62ch] text-[13.5px] leading-relaxed text-text-secondary">
          영상이 든 폴더를 등록하면 그 안의 작품, 시즌, 회차와 영상·자막 파일을 찾아 라이브러리에 모아요. 폴더는
          읽기만 하고 파일을 만들거나 옮기거나 지우지 않아요. 수집할 때마다 다시 확인하고, 새로 생긴 파일은 그때
          추가한 것으로 기록해요.
        </p>
      </header>

      {data === undefined && folders.error !== null && (
        <div className="flex flex-col items-start gap-2.5">
          <p role="alert" className="text-[13px] font-semibold text-urgent">
            {folders.error}
          </p>
          <button type="button" className={BTN.plain} onClick={folders.reload}>
            재시도
          </button>
        </div>
      )}
      {data === undefined && folders.error === null && folders.slow && (
        <p className="text-[13px] text-text-muted">불러오는 중이에요.</p>
      )}
      {data !== undefined && (
        <>
          {data.folders.length === 0 ? (
            <p className="rounded-card border border-dashed border-hairline px-4 py-6 text-center text-[13.5px] leading-relaxed text-text-muted">
              등록한 감시 폴더가 아직 없어요. 아래에서 폴더를 추가하면 그 안의 작품을 찾아요.
            </p>
          ) : (
            <ul className="m-0 flex list-none flex-col gap-3 p-0">
              {data.folders.map((folder) => (
                <li key={folder.id}>
                  <FolderRow
                    folder={folder}
                    onReload={reload}
                    onRemoved={() => {
                      forgetLibrary();
                      folders.update((list) => ({ folders: list.folders.filter((f) => f.id !== folder.id) }));
                    }}
                  />
                </li>
              ))}
            </ul>
          )}
          <AddFolder
            onAdded={(folder) => {
              forgetLibrary();
              folders.update((list) => ({ folders: [...list.folders, folder] }));
            }}
          />
        </>
      )}
    </div>
  );
}

function FolderRow({
  folder,
  onReload,
  onRemoved,
}: {
  folder: WatchFolder;
  onReload: () => void;
  onRemoved: () => void;
}) {
  const uid = useId();
  const rescan = useRescan(folder.id, onReload);
  const [confirming, setConfirming] = useState(false);
  const [removing, setRemoving] = useState(false);
  const [removeError, setRemoveError] = useState<string | null>(null);
  const confirmRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (confirming) confirmRef.current?.focus();
  }, [confirming]);

  const waiting = rescan.phase.kind === "sending" || rescan.phase.kind === "waiting" || rescan.phase.kind === "unconfirmed";

  const remove = async () => {
    setRemoving(true);
    setRemoveError(null);
    try {
      await removeWatchFolder(folder.id);
      onRemoved();
    } catch (e) {
      if (e instanceof ApiError && e.code === "not_found") {
        // Already unregistered somewhere else: the row goes either way.
        onRemoved();
        return;
      }
      setConfirming(false);
      setRemoveError(e instanceof ApiError ? e.message : "등록을 해제하지 못했어요. 잠시 뒤 다시 시도해 주세요.");
    } finally {
      setRemoving(false);
    }
  };

  return (
    <section
      aria-labelledby={`${uid}-path`}
      className="flex flex-col gap-3 rounded-card border border-hairline bg-surface-2 p-3.5"
    >
      <p id={`${uid}-path`} className="m-0 min-w-0 font-mono text-[13px] font-semibold break-all">
        {folder.path}
      </p>

      <Facts>
        <Labeled label="작품">{folder.works}개</Labeled>
        <Labeled label="연결한 작품">{folder.linked_works}개</Labeled>
        <Labeled label="새로 발견한 작품">{folder.new_works}개</Labeled>
        {folder.missing_works > 0 && <Labeled label="폴더 없음">{folder.missing_works}개</Labeled>}
        <span className="text-[13px] text-text-muted" title={folder.checked_at === null ? undefined : dateTime(folder.checked_at)}>
          {folder.checked_at === null ? "아직 확인하지 않았어요" : `마지막 확인 ${ago(folder.checked_at)}`}
        </span>
      </Facts>

      {folder.error && (
        <p role="alert" className="m-0 text-[13px] leading-normal font-semibold text-urgent">
          {folder.error} 이전에 찾은 작품은 그대로 두었어요.
        </p>
      )}
      {rescan.phase.kind === "idle" && rescan.phase.message && rescan.phase.message !== folder.error && (
        <p role="alert" className="m-0 text-[13px] leading-normal font-semibold text-urgent">
          {rescan.phase.message}
        </p>
      )}
      {removeError && (
        <p role="alert" className="m-0 text-[13px] leading-normal font-semibold text-urgent">
          {removeError}
        </p>
      )}
      <p role="status" className="m-0 text-[13px] font-semibold text-text-secondary empty:hidden">
        {rescan.phase.kind === "sending" || rescan.phase.kind === "waiting"
          ? "다시 확인하는 중이에요."
          : rescan.phase.kind === "unconfirmed"
            ? "서버 응답을 받지 못해서 다시 묻는 중이에요."
            : rescan.done
              ? "다시 확인했어요."
              : null}
      </p>

      {confirming ? (
        <div
          ref={confirmRef}
          tabIndex={-1}
          role="group"
          aria-labelledby={`${uid}-remove`}
          className="flex flex-col gap-2.5 rounded-xl border border-hairline bg-surface-1 p-3.5 outline-offset-2"
        >
          <p id={`${uid}-remove`} className="m-0 text-sm font-bold">
            이 감시 폴더의 등록을 해제할까요?
          </p>
          <p className="m-0 text-[13px] leading-normal text-text-secondary">
            디스크의 파일은 그대로 두고, 이 폴더의 작품 {folder.works}개만 라이브러리에서 빼요. 나중에 같은 폴더를
            다시 등록하면 처음부터 다시 찾고, 그때는 모든 파일의 추가 시각이 미상이에요.
          </p>
          <div className="flex flex-wrap gap-2">
            <button type="button" className={BTN.main} disabled={removing} onClick={() => void remove()}>
              {removing ? "해제 중" : "등록 해제"}
            </button>
            <button type="button" className={BTN.plain} disabled={removing} onClick={() => setConfirming(false)}>
              취소
            </button>
          </div>
        </div>
      ) : (
        <div className="flex flex-wrap items-center gap-x-3 gap-y-2">
          <button type="button" className={BTN.action} disabled={waiting} onClick={() => void rescan.press()}>
            {waiting ? "확인 중" : "다시 확인"}
          </button>
          {folder.automatic ? (
            <p className="m-0 min-w-0 flex-1 basis-60 text-[13px] leading-normal text-text-secondary">
              수집 폴더 또는 보관 폴더라서, 수집 폴더 설정에 정해 두는 동안 늘 감시해요.
            </p>
          ) : (
            <button type="button" className={BTN.plain} disabled={waiting} onClick={() => setConfirming(true)}>
              등록 해제
            </button>
          )}
        </div>
      )}
    </section>
  );
}

function AddFolder({ onAdded }: { onAdded: (folder: WatchFolder) => void }) {
  const uid = useId();
  const [path, setPath] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [found, setFound] = useState<number | null>(null);

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    if (busy || path.trim() === "") return;
    setBusy(true);
    setError(null);
    setFound(null);
    try {
      const added = await addWatchFolder(path.trim());
      onAdded(added.folder);
      setPath("");
      setFound(added.works_found);
    } catch (e) {
      setError(e instanceof ApiError ? e.message : "추가하지 못했어요. 잠시 뒤 다시 시도해 주세요.");
    } finally {
      setBusy(false);
    }
  };

  return (
    <form onSubmit={submit} className="flex flex-col gap-3 border-t border-hairline-soft pt-4" noValidate>
      <div className="flex min-w-0 flex-col gap-1.5">
        <Label htmlFor={`${uid}-path`} className={labelClass}>
          감시 폴더 추가
        </Label>
        <div className="flex flex-wrap gap-2">
          <Input
            id={`${uid}-path`}
            value={path}
            onChange={(e) => {
              setPath(e.target.value);
              setError(null);
              setFound(null);
            }}
            className={`${inputClass} min-w-0 flex-1 basis-64 font-mono`}
            placeholder="/media/anime"
            autoComplete="off"
            autoCapitalize="none"
            spellCheck={false}
            aria-describedby={`${uid}-hint`}
            aria-invalid={error !== null}
          />
          <button type="submit" className={BTN.main} disabled={busy || path.trim() === ""}>
            {busy ? "찾는 중" : "추가"}
          </button>
        </div>
        <p id={`${uid}-hint`} className={hintClass}>
          앱이 볼 수 있는 전체 경로로 적어요. 이미 등록한 폴더의 안쪽이나 바깥 폴더는 추가할 수 없어요.
        </p>
      </div>
      {error && (
        <p role="alert" className="m-0 text-[13px] leading-normal font-semibold text-urgent">
          {error}
        </p>
      )}
      <p role="status" className="m-0 text-[13px] font-semibold text-text-secondary empty:hidden">
        {found !== null && !error ? `작품 ${found}개를 찾았어요` : null}
      </p>
    </form>
  );
}
