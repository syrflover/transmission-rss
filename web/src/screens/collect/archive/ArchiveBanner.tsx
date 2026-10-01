import { useId, useState } from "react";

import { Button } from "@/components/ui/button";
import { ApiError } from "@/lib/api";
import { useCached } from "@/lib/cached";
import { cn } from "@/lib/utils";

import { KEYS, suggestionGone } from "../cache";
import { btnNeutral } from "../channels/styles";
import { fetchArchiveSuggestions, keepCollecting, type ArchiveSuggestion } from "./api";
import { archBorder, archFill, btnArch } from "./ArchiveSuggestions";
import { groundText } from "./ground";

interface ArchiveBannerProps {
  ruleId: string;
  /** The rule is being archived or restored now: the buttons wait. */
  busy: boolean;
  /** `보관`: send the rule's own `rule_archive` command, as the rule detail's archive button does. */
  onArchive: () => void;
}

/**
 * The archive suggestion on a rule's detail: `이 규칙을 보관할까요?`, why, what
 * archiving would do with the folder, and `보관` · `수집 유지`. `수집 유지`
 * keeps the rule as it is and remembers the grounds it saw, so the same
 * grounds do not suggest it again. Renders nothing for a rule that is not
 * suggested.
 */
export function ArchiveBanner({ ruleId, busy, onArchive }: ArchiveBannerProps) {
  const headingId = useId();
  const list = useCached<ArchiveSuggestion[]>(
    KEYS.archiveSuggestions,
    fetchArchiveSuggestions,
    "보관 제안을 불러오지 못했어요.",
  );
  const suggestion = list.data?.find((s) => s.rule_id === ruleId);
  const [keeping, setKeeping] = useState(false);
  const [error, setError] = useState<string | null>(null);

  if (suggestion === undefined) return null;

  const keep = async () => {
    setKeeping(true);
    setError(null);
    try {
      await keepCollecting(suggestion);
      suggestionGone(ruleId);
      // The server decides what is still suggested (a ground may have come up since).
      list.reload();
    } catch (e) {
      setError(e instanceof ApiError ? e.message : "수집 유지를 저장하지 못했어요. 다시 시도해 주세요.");
    } finally {
      setKeeping(false);
    }
  };

  return (
    <section
      aria-labelledby={headingId}
      data-testid="archive-banner"
      className={cn("flex min-w-0 flex-col gap-2.5 rounded-xl border px-3.5 py-3", archBorder, archFill)}
    >
      <h3 id={headingId} className="text-[15px] font-bold text-arch">
        이 규칙을 보관할까요?
      </h3>
      <div className="flex min-w-0 flex-col gap-1 text-[13px] leading-normal text-text-secondary">
        {suggestion.grounds.map((ground) => (
          <p key={ground.key} className="break-words">
            {groundText(ground, suggestion)}
          </p>
        ))}
        <p className="break-words">{suggestion.after}</p>
        <p className="break-words text-text-muted">
          {suggestion.state === "paused" ? "멈춰 있는 규칙이에요. " : ""}
          수집 유지를 고르면 규칙은 그대로 두고, 같은 까닭으로는 다시 묻지 않아요.
        </p>
      </div>
      <div className="flex flex-wrap items-center gap-2">
        <Button type="button" variant="ghost" className={btnArch} disabled={busy || keeping} onClick={onArchive}>
          보관
        </Button>
        <Button type="button" variant="ghost" className={btnNeutral} disabled={busy || keeping} onClick={() => void keep()}>
          {keeping ? "저장하는 중" : "수집 유지"}
        </Button>
      </div>
      {error !== null && (
        <p role="alert" className="text-xs leading-normal font-semibold text-urgent">
          {error}
        </p>
      )}
    </section>
  );
}
