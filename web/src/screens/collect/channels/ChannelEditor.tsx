import { useEffect, useId, useRef, useState, type FormEvent } from "react";

import { ApiError } from "@/lib/api";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";

import { LockIcon } from "../icons";
import {
  createChannel,
  deleteChannel,
  saveChannel,
  type Channel,
  type ChannelDraft,
  type QueryParam,
} from "./api";
import { queryNames, queryPairs } from "./query";
import {
  btnDanger,
  btnDangerSolid,
  btnNeutral,
  btnPrimary,
  hintClass,
  inputClass,
  labelClass,
} from "./styles";

interface ChannelEditorProps {
  /** The channel being edited, or `null` to add a new one. */
  channel: Channel | null;
  onSaved: (channel: Channel) => void;
  onCancel: () => void;
  onDeleted?: (channel: Channel, removedRules: number) => void;
}

/** Shown wherever a secret can be re-entered: in the editor and under the channels tab. */
export const SECRET_REENTRY_HINT =
  "비밀 값은 저장한 뒤 다시 보여주지 않아요. 바꾸려면 주소의 그 값 자리에 새 값을 넣고, 그대로 두려면 비워 두세요.";
const NEW_CHANNEL_HINT =
  "주소의 모든 값은 비밀로 두고 시작해요. 공개해도 되는 값만 아래에서 풀어 주세요.";

function statusOf(
  name: string,
  secret: boolean,
  url: string,
  stored: QueryParam | undefined,
): string | null {
  const typed = queryPairs(url).some((pair) => pair.name === name && pair.value !== "" && pair.value !== "***");
  const kept = stored?.secret === true && stored.filled;
  if (secret) {
    if (typed) return kept ? "새 값으로 변경" : "값 입력함";
    return kept ? "저장된 값 유지" : "입력 전";
  }
  // Unsetting a stored secret that was left blank shows the stored value from now on.
  if (kept && !typed) return "저장된 값이 목록과 내보내기에 그대로 보여요";
  return null;
}

function messageOf(error: unknown): string {
  return error instanceof ApiError
    ? error.message
    : "저장하지 못했어요. 잠시 뒤 다시 시도해 주세요.";
}

/**
 * Add / edit form of one channel, in place. It keeps what the user typed
 * through failures: a conflict shows the server's current channel next to the
 * input and the next save is sent against that version.
 */
export function ChannelEditor({ channel, onSaved, onCancel, onDeleted }: ChannelEditorProps) {
  const uid = useId();
  const isNew = channel === null;

  const [url, setUrl] = useState(channel?.edit_url ?? "");
  const [baseDir, setBaseDir] = useState(channel?.base_dir ?? "");
  const [excludes, setExcludes] = useState((channel?.excludes ?? []).join("\n"));
  const [pastSearch, setPastSearch] = useState(channel?.past_search ?? "");
  // A name without an entry is secret, like on the server.
  const [flags, setFlags] = useState<Record<string, boolean>>(() =>
    Object.fromEntries((channel?.query ?? []).map((q) => [q.name, q.secret])),
  );
  // What the server last told us about the channel.
  const [known, setKnown] = useState<Channel | null>(channel);

  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [conflict, setConflict] = useState<Channel | null>(null);
  const [confirmingDelete, setConfirmingDelete] = useState(false);

  const urlRef = useRef<HTMLInputElement>(null);
  const confirmRef = useRef<HTMLDivElement>(null);
  useEffect(() => urlRef.current?.focus(), []);
  useEffect(() => {
    if (confirmingDelete) confirmRef.current?.focus();
  }, [confirmingDelete]);

  const names = queryNames(url);
  const isSecret = (name: string) => flags[name] ?? true;

  const draft = (): ChannelDraft => ({
    url,
    base_dir: baseDir,
    excludes: excludes.split("\n"),
    secret: Object.fromEntries(names.map((name) => [name, isSecret(name)])),
    past_search: pastSearch,
  });

  /** A 409 keeps the input, remembers the server's channel and its version. */
  const handleConflict = (e: unknown): boolean => {
    if (e instanceof ApiError && e.code === "conflict" && e.current) {
      const current = e.current as Channel;
      setConflict(current);
      setKnown(current);
      return true;
    }
    return false;
  };

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      const saved = known
        ? await saveChannel(known.id, known.version, draft())
        : await createChannel(draft());
      onSaved(saved);
    } catch (e) {
      if (handleConflict(e)) {
        setConfirmingDelete(false);
      } else {
        setConflict(null);
        setError(messageOf(e));
      }
      setBusy(false);
    }
  };

  const remove = async () => {
    if (!known || busy) return;
    setBusy(true);
    setError(null);
    try {
      const removed = await deleteChannel(known.id, known.version, known.rule_count);
      onDeleted?.(known, removed);
    } catch (e) {
      if (handleConflict(e)) {
        setConfirmingDelete(false);
      } else {
        setConflict(null);
        setError(messageOf(e));
      }
      setBusy(false);
    }
  };

  return (
    <form onSubmit={submit} className="flex flex-col gap-3.5" noValidate>
      <div className="flex min-w-0 flex-col gap-1.5">
        <Label htmlFor={`${uid}-url`} className={labelClass}>
          RSS 주소
        </Label>
        <Input
          ref={urlRef}
          id={`${uid}-url`}
          value={url}
          onChange={(e) => setUrl(e.target.value)}
          className={`${inputClass} font-mono`}
          placeholder="https://…/rss?token=…"
          inputMode="url"
          autoComplete="off"
          autoCapitalize="none"
          spellCheck={false}
          aria-describedby={`${uid}-url-hint`}
        />
        <p id={`${uid}-url-hint`} className={hintClass}>
          {isNew ? NEW_CHANNEL_HINT : SECRET_REENTRY_HINT}
        </p>
      </div>

      {names.length > 0 && (
        <fieldset className="flex min-w-0 flex-col gap-1.5 border-0 p-0">
          <legend className={`${labelClass} mb-1.5 p-0`}>주소의 값</legend>
          <ul className="m-0 flex list-none flex-col gap-1.5 p-0">
            {names.map((name) => {
              const secret = isSecret(name);
              const status = statusOf(
                name,
                secret,
                url,
                known?.query.find((q) => q.name === name),
              );
              return (
                <li
                  key={name}
                  className="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-1 rounded-[10px] border border-hairline-soft bg-surface-2 px-3 py-2"
                >
                  <label className="flex min-h-6 min-w-0 cursor-pointer items-center gap-2 text-[13.5px]">
                    <input
                      type="checkbox"
                      checked={secret}
                      onChange={(e) => setFlags({ ...flags, [name]: e.target.checked })}
                      className="size-[18px] flex-none accent-focus"
                    />
                    {secret && <LockIcon className="size-3.5 flex-none text-text-muted" />}
                    <span className="min-w-0 font-semibold">{name} 비밀</span>
                  </label>
                  {status && <span className={hintClass}>{status}</span>}
                </li>
              );
            })}
          </ul>
        </fieldset>
      )}

      <div className="flex min-w-0 flex-col gap-1.5">
        <Label htmlFor={`${uid}-dir`} className={labelClass}>
          기본 저장 폴더
        </Label>
        <Input
          id={`${uid}-dir`}
          value={baseDir}
          onChange={(e) => setBaseDir(e.target.value)}
          className={inputClass}
          placeholder="/media/anime"
          autoComplete="off"
          spellCheck={false}
        />
      </div>

      <div className="grid min-w-0 grid-cols-2 gap-3.5 max-[720px]:grid-cols-1">
        <div className="flex min-w-0 flex-col gap-1.5">
          <Label htmlFor={`${uid}-ex`} className={labelClass}>
            제외 조건
          </Label>
          <Textarea
            id={`${uid}-ex`}
            value={excludes}
            onChange={(e) => setExcludes(e.target.value)}
            className={`${inputClass} min-h-[72px] font-mono`}
            autoComplete="off"
            spellCheck={false}
            aria-describedby={`${uid}-ex-hint`}
          />
          <p id={`${uid}-ex-hint`} className={hintClass}>
            한 줄에 조건 하나씩 적어요.
          </p>
        </div>
        <div className="flex min-w-0 flex-col gap-1.5">
          <Label htmlFor={`${uid}-ps`} className={labelClass}>
            지난 회차 검색 형식
          </Label>
          <Input
            id={`${uid}-ps`}
            value={pastSearch}
            onChange={(e) => setPastSearch(e.target.value)}
            className={`${inputClass} font-mono`}
            placeholder="[SubsPlease] {match} 1080p"
            autoComplete="off"
            spellCheck={false}
            aria-describedby={`${uid}-ps-hint`}
          />
          <p id={`${uid}-ps-hint`} className={hintClass}>
            비우면 검색할 때 검색어를 직접 넣어요.
          </p>
        </div>
      </div>

      {conflict && <ConflictNotice current={conflict} />}
      {error && (
        <p role="alert" className="text-[13px] font-semibold text-urgent">
          {error}
        </p>
      )}

      {confirmingDelete && known ? (
        <div
          ref={confirmRef}
          tabIndex={-1}
          role="group"
          aria-labelledby={`${uid}-del`}
          className="flex flex-col gap-2.5 rounded-xl border border-[color-mix(in_srgb,var(--accent-urgent)_50%,transparent)] p-3.5 outline-offset-2"
        >
          <p id={`${uid}-del`} className="text-sm font-bold">
            이 채널을 삭제할까요?
          </p>
          <p className="text-[13px] leading-normal text-text-secondary">
            {known.rule_count > 0
              ? `이 채널과 규칙 ${known.rule_count}개를 삭제해요.`
              : "이 채널을 삭제해요."}{" "}
            이미 받은 파일과 수집 기록은 지우지 않아요.
          </p>
          <div className="flex flex-wrap gap-2">
            <Button type="button" variant="ghost" className={btnDangerSolid} disabled={busy} onClick={remove}>
              {busy ? "삭제 중" : "삭제"}
            </Button>
            <Button
              type="button"
              variant="ghost"
              className={btnNeutral}
              disabled={busy}
              onClick={() => setConfirmingDelete(false)}
            >
              취소
            </Button>
          </div>
        </div>
      ) : (
        <div className="flex flex-wrap items-center gap-2 border-t border-hairline-soft pt-3.5">
          <Button type="submit" variant="ghost" className={btnPrimary} disabled={busy}>
            {busy ? "저장 중" : "저장"}
          </Button>
          <Button type="button" variant="ghost" className={btnNeutral} disabled={busy} onClick={onCancel}>
            취소
          </Button>
          {known && (
            <Button
              type="button"
              variant="ghost"
              className={`${btnDanger} ml-auto`}
              disabled={busy}
              onClick={() => setConfirmingDelete(true)}
            >
              삭제
            </Button>
          )}
        </div>
      )}
    </form>
  );
}

/** The server's channel after someone else saved first, next to the user's untouched input. */
function ConflictNotice({ current }: { current: Channel }) {
  return (
    <section
      role="alert"
      className="flex flex-col gap-2 rounded-xl border border-hairline bg-surface-2 p-3.5"
    >
      <p className="text-sm font-bold">다른 곳에서 먼저 저장했어요.</p>
      <p className="text-[13px] leading-normal text-text-secondary">
        입력한 값은 그대로 두었어요. 아래는 지금 저장된 값이에요. 확인하고 다시 저장할 수 있어요.
      </p>
      <dl className="m-0 grid grid-cols-[110px_minmax(0,1fr)] gap-x-3.5 gap-y-1.5 text-[13px] max-[480px]:grid-cols-1 max-[480px]:gap-y-0.5">
        <dt className="font-semibold text-text-muted">주소</dt>
        <dd className="m-0 min-w-0 font-mono text-xs break-all max-[480px]:mb-1.5">{current.masked_url}</dd>
        <dt className="font-semibold text-text-muted">기본 저장 폴더</dt>
        <dd className="m-0 min-w-0 max-[480px]:mb-1.5">{current.base_dir}</dd>
        <dt className="font-semibold text-text-muted">제외 조건</dt>
        <dd className="m-0 min-w-0 font-mono text-xs break-all max-[480px]:mb-1.5">
          {current.excludes.length > 0 ? current.excludes.join(", ") : "없음"}
        </dd>
        <dt className="font-semibold text-text-muted">지난 회차 검색</dt>
        <dd className="m-0 min-w-0 font-mono text-xs break-all">{current.past_search ?? "형식 없음"}</dd>
      </dl>
    </section>
  );
}
