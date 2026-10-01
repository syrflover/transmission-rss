import { useState } from "react";

import { ApiError } from "@/lib/api";

import { btnNeutral, hintClass } from "../channels/styles";
import { applyEpisode, type Rule } from "./api";

/**
 * What the app says about the rule's episode offset under its field: the
 * grounds of a value it set (the `자동` mark sits on the label), or a suggestion
 * with `적용`. Applying saves the offset at once as the user's own, like the
 * switches; a stale version takes the current rule and says so.
 *
 * `shown` is false while the field holds something other than the stored
 * offset: the grounds and the suggestion are about the stored one.
 */
export function EpisodeGrounds({
  rule,
  shown,
  disabled,
  onChanged,
}: {
  rule: Rule;
  shown: boolean;
  /** Something else is changing the rule (a save, an archive move). */
  disabled: boolean;
  /** The rule as the server has it after `적용`, or after the conflict that showed it. */
  onChanged: (rule: Rule) => void;
}) {
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);

  if (!shown) return null;
  const suggestion = rule.episode_suggestion;
  const grounds = rule.episode_auto ? (rule.episode_basis ?? "앱이 정한 값이에요.") : null;
  if (grounds === null && suggestion === null) return null;

  const apply = async (value: number) => {
    setBusy(true);
    setMessage(null);
    try {
      onChanged(await applyEpisode(rule, value));
    } catch (e) {
      if (e instanceof ApiError && e.code === "conflict" && e.current) {
        onChanged(e.current as Rule);
        setMessage("다른 곳에서 먼저 바꿨어요. 지금 상태를 보여드려요.");
      } else {
        setMessage(e instanceof ApiError ? e.message : "적용하지 못했어요. 잠시 뒤 다시 시도해 주세요.");
      }
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="flex min-w-0 flex-col gap-1.5" data-testid="episode-grounds">
      {grounds !== null && (
        <p className={hintClass} data-testid="episode-basis">
          {grounds} 직접 바꾸면 자동 표시가 사라지고, 앱은 그 값을 다시 바꾸지 않아요.
        </p>
      )}
      {suggestion !== null && (
        <div
          className="flex min-w-0 flex-col gap-2 rounded-lg border border-hairline bg-surface-2 p-2.5"
          data-testid="episode-suggestion"
        >
          <p className="text-[12.5px] leading-normal text-text-secondary">{suggestion.basis}</p>
          {suggestion.value !== null && (
            <div className="flex min-w-0 flex-wrap items-center gap-2">
              <span className="font-mono text-[13px]">{suggestion.value}</span>
              <button
                type="button"
                className={`${btnNeutral} max-[720px]:min-h-11`}
                disabled={busy || disabled}
                onClick={() => void apply(suggestion.value!)}
              >
                적용
              </button>
            </div>
          )}
        </div>
      )}
      {message !== null && (
        <p role="status" className={`${hintClass} font-semibold`}>
          {message}
        </p>
      )}
    </div>
  );
}
