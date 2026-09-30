import { useEffect, useId, useRef, useState, type FormEvent } from "react";

import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { ApiError } from "@/lib/api";
import { useCached } from "@/lib/cached";
import { collectFolderChanged, KEYS } from "@/screens/collect/cache";
import {
  hintClass,
  inputClass,
  labelClass,
} from "@/screens/collect/channels/styles";

import { Banner, BTN } from "../parts";
import { loadCollection, saveCollection, type Collection } from "./api";

const LOAD_FAILED = "수집 폴더를 불러오지 못했어요.";

/** `수집 폴더`: where the worker saves what it receives, and the optional folder finished works move to. */
export function CollectionPanel() {
  const collection = useCached<Collection>(
    KEYS.collection,
    loadCollection,
    LOAD_FAILED,
  );
  const data = collection.data;

  return (
    <div className="flex flex-col gap-5 p-5 max-[720px]:p-4">
      <header className="flex flex-col gap-1.5">
        <h2
          id="panel-title"
          tabIndex={-1}
          className="m-0 text-xl font-bold outline-none"
        >
          수집 폴더
        </h2>
        <p className="max-w-[62ch] text-[13.5px] leading-relaxed text-text-secondary">
          RSS로 받는 파일은 모두 수집 폴더 아래에 저장해요. 규칙의 저장 폴더는
          이 폴더를 기준으로 한 경로예요. 보관 폴더는 수집 폴더와 짝이 되는
          폴더이고, 지금은 저장만 해요.
        </p>
      </header>

      {data === undefined && collection.error !== null && (
        <div className="flex flex-col items-start gap-2.5">
          <p role="alert" className="text-[13px] font-semibold text-urgent">
            {collection.error}
          </p>
          <button
            type="button"
            className={BTN.plain}
            onClick={collection.reload}
          >
            재시도
          </button>
        </div>
      )}
      {data === undefined && collection.error === null && collection.slow && (
        <p className="text-[13px] text-text-muted">불러오는 중이에요.</p>
      )}
      {data !== undefined && (
        <CollectionForm stored={data} onSaved={collection.update} />
      )}
    </div>
  );
}

function CollectionForm({
  stored,
  onSaved,
}: {
  stored: Collection;
  onSaved: (next: Collection) => void;
}) {
  const uid = useId();
  const [folder, setFolder] = useState(stored.folder ?? "");
  const [archive, setArchive] = useState(stored.archive_folder ?? "");
  // What the server last told us; a save is sent against its version.
  const [known, setKnown] = useState(stored);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [conflict, setConflict] = useState<Collection | null>(null);
  const [saved, setSaved] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const confirmRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (confirming) confirmRef.current?.focus();
  }, [confirming]);

  const unchanged =
    folder.trim() === (known.folder ?? "") &&
    archive.trim() === (known.archive_folder ?? "");
  // Moving an existing collect folder changes where new files go; ask once before it is saved.
  const movesCollectFolder =
    known.folder !== null && folder.trim() !== known.folder;

  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (busy) return;
    if (movesCollectFolder) {
      setConfirming(true);
      return;
    }
    void save();
  };

  const save = async () => {
    setConfirming(false);
    setBusy(true);
    setError(null);
    setSaved(false);
    try {
      const next = await saveCollection(
        known.version,
        folder.trim(),
        archive.trim(),
      );
      setKnown(next);
      setConflict(null);
      setFolder(next.folder ?? "");
      setArchive(next.archive_folder ?? "");
      onSaved(next);
      // The rule list and its previews carry full paths, and the status board says whether a folder is set.
      collectFolderChanged();
      setSaved(true);
    } catch (e) {
      if (e instanceof ApiError && e.code === "conflict") {
        // The server does not send its value with this answer: read it, keep the input.
        try {
          const current = await loadCollection();
          setKnown(current);
          setConflict(current);
          onSaved(current);
        } catch {
          setError(e.message);
        }
      } else {
        setConflict(null);
        setError(
          e instanceof ApiError
            ? e.message
            : "저장하지 못했어요. 잠시 뒤 다시 시도해 주세요.",
        );
      }
    } finally {
      setBusy(false);
    }
  };

  return (
    <form onSubmit={submit} className="flex flex-col gap-4" noValidate>
      {stored.folder === null && known.folder === null && (
        <Banner tone="calm" title="수집 폴더를 정해 주세요">
          정하기 전에는 새 항목을 받지 않아요. 규칙에 맞는 항목은 그대로
          두었다가, 폴더를 정하면 다음 수집에서 받아요.
        </Banner>
      )}

      <div className="flex min-w-0 flex-col gap-1.5">
        <Label htmlFor={`${uid}-folder`} className={labelClass}>
          수집 폴더
        </Label>
        <Input
          id={`${uid}-folder`}
          value={folder}
          onChange={(e) => {
            setFolder(e.target.value);
            setConfirming(false);
          }}
          className={`${inputClass} font-mono`}
          placeholder="/downloads/Shows (current)"
          autoComplete="off"
          autoCapitalize="none"
          spellCheck={false}
          aria-describedby={`${uid}-folder-hint`}
        />
        <p id={`${uid}-folder-hint`} className={hintClass}>
          앱이 볼 수 있는 경로로 적어요. 있는 폴더여야 해요.
        </p>
      </div>

      <div className="flex min-w-0 flex-col gap-1.5">
        <Label htmlFor={`${uid}-archive`} className={labelClass}>
          보관 폴더
        </Label>
        <Input
          id={`${uid}-archive`}
          value={archive}
          onChange={(e) => setArchive(e.target.value)}
          className={`${inputClass} font-mono`}
          placeholder="/downloads/Shows"
          autoComplete="off"
          autoCapitalize="none"
          spellCheck={false}
          aria-describedby={`${uid}-archive-hint`}
        />
        <p id={`${uid}-archive-hint`} className={hintClass}>
          비워 둘 수 있어요. 수집 폴더와 같거나 그 안에 있으면 안 되고, 같은
          파일시스템에 있어야 해요.
        </p>
      </div>

      {conflict && (
        <section
          role="alert"
          className="flex flex-col gap-2 rounded-xl border border-hairline bg-surface-2 p-3.5"
        >
          <p className="text-sm font-bold">다른 곳에서 먼저 저장했어요.</p>
          <p className="text-[13px] leading-normal text-text-secondary">
            입력한 값은 그대로 두었어요. 아래는 지금 저장된 값이에요. 확인하고
            다시 저장할 수 있어요.
          </p>
          <dl className="m-0 grid grid-cols-[90px_minmax(0,1fr)] gap-x-3.5 gap-y-1.5 text-[13px] max-[480px]:grid-cols-1 max-[480px]:gap-y-0.5">
            <dt className="font-semibold text-text-muted">수집 폴더</dt>
            <dd className="m-0 min-w-0 font-mono text-xs break-all max-[480px]:mb-1.5">
              {conflict.folder ?? "없음"}
            </dd>
            <dt className="font-semibold text-text-muted">보관 폴더</dt>
            <dd className="m-0 min-w-0 font-mono text-xs break-all">
              {conflict.archive_folder ?? "없음"}
            </dd>
          </dl>
        </section>
      )}
      {error && (
        <p role="alert" className="text-[13px] font-semibold text-urgent">
          {error}
        </p>
      )}
      <p
        role="status"
        className="text-[13px] font-semibold text-text-secondary empty:hidden"
      >
        {saved && !error ? "저장했어요." : null}
      </p>

      {confirming ? (
        <div
          ref={confirmRef}
          tabIndex={-1}
          role="group"
          aria-labelledby={`${uid}-move`}
          className="flex flex-col gap-2.5 rounded-xl border border-hairline bg-surface-2 p-3.5 outline-offset-2"
        >
          <p id={`${uid}-move`} className="text-sm font-bold">
            수집 폴더를 바꿀까요?
          </p>
          <p className="text-[13px] leading-normal text-text-secondary">
            이미 받은 파일은 있던 자리에 그대로 있어요. 규칙의 저장 폴더는
            그대로 두고 새 수집 폴더를 기준으로 삼아요. 앞으로 추가하는 항목은
            새 폴더 아래에 저장해요.
          </p>
          <div className="flex flex-wrap gap-2">
            <button
              type="button"
              className={BTN.main}
              disabled={busy}
              onClick={() => void save()}
            >
              바꾸기
            </button>
            <button
              type="button"
              className={BTN.plain}
              disabled={busy}
              onClick={() => setConfirming(false)}
            >
              취소
            </button>
          </div>
        </div>
      ) : (
        <div className="flex flex-wrap items-center gap-2 border-t border-hairline-soft pt-4">
          <button
            type="submit"
            className={BTN.main}
            disabled={busy || folder.trim() === "" || (unchanged && !conflict)}
          >
            {busy ? "저장 중" : "저장"}
          </button>
          <button
            type="button"
            className={BTN.plain}
            disabled={busy || unchanged}
            onClick={() => {
              setFolder(known.folder ?? "");
              setArchive(known.archive_folder ?? "");
              setError(null);
              setConflict(null);
            }}
          >
            되돌리기
          </button>
        </div>
      )}
    </form>
  );
}
