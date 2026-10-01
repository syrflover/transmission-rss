import { useId, useState } from "react";

import { ApiError } from "@/lib/api";
import { cn } from "@/lib/utils";

import { switchRule, type Rule } from "./api";

export const SUBTITLES_NEED_VIDEO = "영상 받기를 켜야 자막을 받을 수 있어요";

function Switch({
  label,
  checked,
  disabled,
  describedBy,
  onChange,
}: {
  label: string;
  checked: boolean;
  disabled: boolean;
  describedBy?: string;
  onChange: (next: boolean) => void;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      aria-describedby={describedBy}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className={cn(
        "relative inline-flex h-6 w-11 flex-none items-center rounded-full border transition-colors outline-offset-2 focus-visible:outline-2 focus-visible:outline-focus",
        "max-[720px]:h-7 max-[720px]:w-12",
        checked ? "border-focus bg-focus" : "border-hairline bg-surface-3",
        "disabled:cursor-not-allowed disabled:opacity-50",
      )}
    >
      <span
        aria-hidden="true"
        className={cn(
          "absolute left-0.5 size-[18px] rounded-full bg-white shadow transition-transform max-[720px]:size-5",
          checked ? "translate-x-[22px] max-[720px]:translate-x-6" : "translate-x-0",
        )}
      />
    </button>
  );
}

/**
 * `영상 받기` and, for a subscription, `자막 받기`: applied at once, with no
 * save row. `영상 받기` off pauses the rule (nothing is collected and the work
 * folder stays); `자막 받기` waits for it. An archived rule shows both off and
 * cannot switch: it is restored first. A switch answered with a conflict takes
 * the current rule and says so.
 */
export function SwitchRows({
  rule,
  disabled,
  onChanged,
}: {
  rule: Rule;
  /** Something else is changing the rule (a save, an archive move). */
  disabled: boolean;
  /** The rule as the server has it after a switch, or after the conflict that showed it. */
  onChanged: (rule: Rule) => void;
}) {
  const uid = useId();
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);

  const archived = rule.state === "archived";
  const video = rule.state === "active";
  const subscription = rule.subscription;
  const subtitlesOn = video && subscription !== null && subscription.subtitles !== "none";

  const run = async (change: { video: boolean } | { subtitles: boolean }) => {
    setBusy(true);
    setMessage(null);
    try {
      onChanged(await switchRule(rule, change));
    } catch (e) {
      if (e instanceof ApiError && e.code === "conflict" && e.current) {
        onChanged(e.current as Rule);
        setMessage("다른 곳에서 먼저 바꿨어요. 지금 상태를 보여드려요. 다시 눌러 주세요.");
      } else {
        setMessage(e instanceof ApiError ? e.message : "바꾸지 못했어요. 잠시 뒤 다시 시도해 주세요.");
      }
    } finally {
      setBusy(false);
    }
  };

  const locked = busy || disabled;

  return (
    <div className="flex min-w-0 flex-col gap-3" role="group" aria-label="받기 설정" data-testid="switch-rows">
      <div className="flex min-w-0 items-center gap-3">
        <div className="min-w-0 flex-1">
          <p className="text-[13.5px] font-semibold" id={`${uid}-video`}>
            영상 받기
          </p>
          <p className="text-xs leading-normal text-text-muted">
            {archived
              ? "보관한 규칙이에요. 복원하면 켜져요."
              : video
                ? "새 항목을 받아요. 끄면 멈추고, 작품 폴더는 그대로 둬요."
                : "멈춰 있어요. 켜면 다음 RSS 확인부터 새 항목을 받아요."}
          </p>
        </div>
        <Switch
          label="영상 받기"
          checked={video}
          disabled={locked || archived}
          onChange={(next) => run({ video: next })}
        />
      </div>

      {subscription !== null && (
        <div className="flex min-w-0 items-center gap-3">
          <div className="min-w-0 flex-1">
            <p className="text-[13.5px] font-semibold">자막 받기</p>
            <p id={`${uid}-subs-note`} className="text-xs leading-normal text-text-muted">
              {!video
                ? SUBTITLES_NEED_VIDEO
                : subscription.subtitles === "none"
                  ? "자막을 받지 않아요. 켜면 정해 둔 제작자를 따라 받아요."
                  : "정해 둔 제작자의 새 회차 자막을 받아요."}
            </p>
          </div>
          <Switch
            label="자막 받기"
            checked={subtitlesOn}
            disabled={locked || !video}
            describedBy={`${uid}-subs-note`}
            onChange={(next) => run({ subtitles: next })}
          />
        </div>
      )}

      {message && (
        <p role="alert" className="text-[13px] font-semibold text-urgent">
          {message}
        </p>
      )}
    </div>
  );
}
