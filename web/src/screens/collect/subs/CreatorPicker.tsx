import { useId, useState } from "react";

import { Button } from "@/components/ui/button";
import { useCached } from "@/lib/cached";

import { btnAction, btnNeutral } from "../channels/styles";
import { fetchCreators, type Creators } from "./api";

const radio = "mt-0.5 size-[18px] flex-none accent-focus";
const option =
  "flex min-w-0 cursor-pointer items-start gap-3 rounded-[10px] border border-hairline-soft bg-surface-2 px-3.5 py-2.5 has-[:checked]:border-focus has-[:focus-visible]:outline-2 has-[:focus-visible]:outline-offset-2 has-[:focus-visible]:outline-focus";

/**
 * `제작자 변경`: one of the creators Anissia lists for the anime, or `제작자 미정`.
 * Shared by the rule detail and the work detail's head, which change the same
 * value. `current` is the creator now (`null` for `제작자 미정`); `onApply`
 * sends the change and throws the sentence to show when it fails. Creators the
 * app cannot list (Anissia is down) leave `제작자 미정` and the creator now.
 */
export function CreatorPicker({
  animeNo,
  current,
  onApply,
  onCancel,
}: {
  animeNo: number;
  current: string | null;
  onApply: (creator: string | null) => Promise<void>;
  onCancel: () => void;
}) {
  const uid = useId();
  const list = useCached<Creators>(
    `collect:creators:${animeNo}`,
    (signal) => fetchCreators(animeNo, signal),
    "자막 제작자를 불러오지 못했어요.",
  );
  const [picked, setPicked] = useState<string | null>(current);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const names = list.data?.creators.map((c) => c.name) ?? [];
  // The creator now stays a choice even when Anissia no longer lists it.
  const choices = current !== null && !names.includes(current) ? [current, ...names] : names;

  const apply = async () => {
    setBusy(true);
    setError(null);
    try {
      await onApply(picked);
    } catch (e) {
      setError(e instanceof Error ? e.message : "바꾸지 못했어요. 잠시 뒤 다시 시도해 주세요.");
      setBusy(false);
    }
  };

  return (
    <fieldset className="m-0 flex min-w-0 flex-col gap-2 border-0 p-0" data-testid="creator-picker">
      <legend className="mb-1 p-0 text-[13px] font-bold">자막 제작자를 골라요</legend>

      {list.data === undefined && list.error === null && list.slow && (
        <p className="text-[13px] text-text-muted">자막 제작자를 불러오는 중이에요.</p>
      )}
      {list.error !== null && (
        <div className="flex flex-wrap items-center gap-2.5">
          <p role="alert" className="text-[13px] leading-normal font-semibold text-urgent">
            {list.error}
          </p>
          <Button type="button" variant="ghost" className={btnNeutral} onClick={list.reload}>
            재시도
          </Button>
        </div>
      )}
      {list.data !== undefined && choices.length === 0 && (
        <p className="text-[13px] leading-normal text-text-muted">Anissia에 등록된 자막 제작자가 없어요.</p>
      )}

      {choices.map((name) => (
        <label key={name} className={option}>
          <input
            type="radio"
            name={`${uid}-creator`}
            className={radio}
            checked={picked === name}
            onChange={() => setPicked(name)}
          />
          <span className="min-w-0 text-[14px] font-semibold break-words">{name}</span>
        </label>
      ))}
      <label className={option}>
        <input
          type="radio"
          name={`${uid}-creator`}
          className={radio}
          checked={picked === null}
          onChange={() => setPicked(null)}
        />
        <span className="text-[14px] font-semibold">제작자 미정</span>
      </label>

      {error && (
        <p role="alert" className="text-[13px] font-semibold text-urgent">
          {error}
        </p>
      )}
      <div className="flex flex-wrap gap-2">
        <Button type="button" variant="ghost" className={btnAction} disabled={busy || picked === current} onClick={apply}>
          {busy ? "바꾸는 중" : "바꾸기"}
        </Button>
        <Button type="button" variant="ghost" className={btnNeutral} disabled={busy} onClick={onCancel}>
          취소
        </Button>
      </div>
    </fieldset>
  );
}
