import { useEffect, useId, useState } from "react";
import { Link } from "react-router-dom";

import { Button } from "@/components/ui/button";
import { useCached } from "@/lib/cached";
import { cn } from "@/lib/utils";
import type { ArchiveSuggestion } from "@/lib/api-types";

import { workPath } from "@/screens/library/api";

import { KEYS } from "../cache";
import { btnNeutral } from "../channels/styles";
import { fetchArchiveSuggestions, getCurrentBatch, startBatch } from "./api";
import { batches } from "./batches";
import type { Run } from "./batchRun";
import { groundText, suggestionTitle } from "./ground";

/** The accent of the suggestions: purple, the calm color of `보관`, not a warning. */
export const archBorder = "border-[color-mix(in_srgb,var(--accent-arch)_45%,transparent)]";
export const archFill = "bg-[color-mix(in_srgb,var(--accent-arch)_7%,transparent)]";
export const btnArch =
  "h-auto min-h-9 rounded-full border border-[color-mix(in_srgb,var(--accent-arch)_55%,transparent)] bg-surface-1 px-[15px] text-[13px] font-semibold text-arch hover:border-arch hover:bg-[color-mix(in_srgb,var(--accent-arch)_12%,transparent)] dark:hover:bg-[color-mix(in_srgb,var(--accent-arch)_12%,transparent)] max-[720px]:min-h-10";

/**
 * The archive suggestions at the top of the 구독 tab, beside the title
 * candidates (purple, not red: nothing here is urgent). One suggestion is a
 * line with `보관`. Several fold into a count with `모두 보관` and `선택 보관`;
 * the latter opens a checklist with every rule selected, and `N개 보관`
 * archives the checked ones one after another. They go to the server as one
 * `rule_archive_batch` command, which archives them in order as a rule's own
 * `보관` does (each rule is deleted, and a work folder shared by several of
 * them moves with the last that collects) and goes on whether this tab is
 * shown or not;
 * the screen shows what the tab's batch tracker follows by the events the
 * server sends (it keeps following when the screen is closed, so the caches
 * still learn what was archived), and finds the batch again when it opens
 * during one. Renders nothing while there are none.
 */
export function ArchiveSuggestions({ className, onArchived }: { className?: string; onArchived: () => void }) {
  const headingId = useId();
  const list = useCached<ArchiveSuggestion[]>(
    KEYS.archiveSuggestions,
    fetchArchiveSuggestions,
    "보관 제안을 불러오지 못했어요.",
  );
  const suggestions = list.data ?? [];
  const [choosing, setChoosing] = useState(false);
  const [unchecked, setUnchecked] = useState<ReadonlySet<string>>(new Set());
  const tracker = batches();
  const [run, setRun] = useState<Run | null>(tracker.run);
  // What the batch calls at its end, which may come after this screen is gone: the latest props, not a render's.
  tracker.setHooks({ archived: onArchived, reload: list.reload });

  // The tracker follows the batch whether or not this screen is shown; the screen only shows what it says.
  useEffect(() => {
    let alive = true;
    setRun(tracker.run);
    const stop = tracker.subscribe((update) => {
      setRun(update.run);
      if (update.ended) setChoosing(false);
    });
    // A batch the server runs when the screen opens (it went on while the screen was closed, or in another tab):
    // show it and follow it.
    if (!tracker.active) {
      void getCurrentBatch()
        .then((view) => {
          if (alive && view !== null) tracker.adopt(view);
        })
        .catch(() => {
          // Nothing is shown for a batch that cannot be read; a new one is refused while it runs.
        });
    }
    return () => {
      alive = false;
      stop();
    };
  }, [tracker]);

  const running = run !== null && run.current !== null;
  const selected = suggestions.filter((s) => !unchecked.has(s.rule_id));

  /**
   * Archives the rules in the order given, stopping at the first that does not end well. The server does it as one
   * command, so it goes on when this tab is hidden or the screen is closed.
   */
  const archive = async (rules: ArchiveSuggestion[]) => {
    if (running || tracker.active || rules.length === 0) return;
    const entries = rules.map((rule) => ({ rule_id: rule.rule_id, name: suggestionTitle(rule) }));
    await tracker.start(entries, () => startBatch(entries));
  };

  if (suggestions.length === 0 && run === null) {
    if (list.data === undefined && list.error !== null) {
      return (
        <p role="alert" className={cn("text-[13px] font-semibold text-urgent", className)}>
          {list.error}
        </p>
      );
    }
    return null;
  }

  const one = suggestions.length === 1 ? suggestions[0] : null;

  return (
    <section
      aria-labelledby={headingId}
      data-testid="archive-suggestions"
      className={cn(
        "flex min-w-0 flex-col gap-3 rounded-card border p-4 max-[480px]:p-3.5",
        archBorder,
        archFill,
        className,
      )}
    >
      <div className="flex min-w-0 flex-wrap items-center justify-between gap-x-4 gap-y-2.5">
        <h2 id={headingId} className="text-[15px] font-bold text-arch">
          {suggestions.length > 1 ? `보관 제안 ${suggestions.length}개` : "보관 제안"}
        </h2>
        {suggestions.length > 1 && (
          <div className="flex flex-wrap items-center gap-2" data-testid="archive-fold">
            <Button
              type="button"
              variant="ghost"
              className={btnArch}
              disabled={running}
              onClick={() => void archive(suggestions)}
            >
              모두 보관
            </Button>
            <Button
              type="button"
              variant="ghost"
              className={btnNeutral}
              aria-expanded={choosing}
              aria-controls={`${headingId}-list`}
              disabled={running}
              onClick={() => setChoosing((was) => !was)}
            >
              {choosing ? "접기" : "선택 보관"}
            </Button>
          </div>
        )}
      </div>
      {suggestions.length > 0 && (
        <p className="min-w-0 text-[13px] leading-normal text-text-secondary">
          방영이 끝났거나 오래 새 항목이 없는 규칙이에요. 보관하면 규칙을 지우고 작품 폴더를 보관 폴더로 옮겨요. 이미 받은
          파일과 수집 기록은 남아요.
        </p>
      )}

      {run !== null && <RunStatus run={run} onClose={() => tracker.dismiss()} />}

      {one !== null && (
        <ul className="m-0 flex list-none flex-col gap-2.5 p-0">
          <li
            data-testid="archive-suggestion"
            className="flex min-w-0 flex-col gap-2 rounded-[10px] border border-hairline-soft bg-surface-1 px-3.5 py-3"
          >
            <SuggestionText suggestion={one} />
            <div className="flex flex-wrap items-center gap-2">
              <Button
                type="button"
                variant="ghost"
                className={btnArch}
                disabled={running}
                onClick={() => void archive([one])}
              >
                보관
              </Button>
            </div>
          </li>
        </ul>
      )}

      {suggestions.length > 1 && choosing && (
        <div id={`${headingId}-list`} className="flex min-w-0 flex-col gap-2.5">
          <ul className="m-0 flex list-none flex-col gap-2.5 p-0">
            {suggestions.map((suggestion) => {
              const checked = !unchecked.has(suggestion.rule_id);
              return (
                <li key={suggestion.rule_id} data-testid="archive-suggestion">
                  <label className="flex min-w-0 cursor-pointer items-start gap-3 rounded-[10px] border border-hairline-soft bg-surface-1 px-3.5 py-3">
                    <input
                      type="checkbox"
                      className="mt-0.5 size-[18px] shrink-0 accent-(--accent-arch)"
                      checked={checked}
                      disabled={running}
                      onChange={() =>
                        setUnchecked((was) => {
                          const next = new Set(was);
                          if (checked) next.add(suggestion.rule_id);
                          else next.delete(suggestion.rule_id);
                          return next;
                        })
                      }
                    />
                    <SuggestionText suggestion={suggestion} link={false} />
                  </label>
                </li>
              );
            })}
          </ul>
          <div className="flex flex-wrap items-center gap-2">
            <Button
              type="button"
              variant="ghost"
              className={btnArch}
              disabled={running || selected.length === 0}
              onClick={() => void archive(selected)}
            >
              {selected.length}개 보관
            </Button>
          </div>
        </div>
      )}
    </section>
  );
}

const workLink =
  "shrink-0 rounded-sm text-xs font-semibold text-text-secondary underline underline-offset-4 outline-offset-2 hover:text-text-primary focus-visible:outline-2 focus-visible:outline-focus";

/**
 * Where a run is: the rule in turn while it goes on; once it ended, how many rules it archived, what became of each
 * one's work folder with a link to the work, and why it stopped.
 */
function RunStatus({ run, onClose }: { run: Run; onClose: () => void }) {
  const done = run.current === null;
  const text = done
    ? run.error === null
      ? `${run.finished}개를 보관했어요.`
      : `${run.finished}개를 보관하고 멈췄어요. ${run.error}`
    : `규칙을 지우고 작품 폴더를 옮기는 중이에요. ‘${run.current}’ (${Math.min(run.finished + 1, run.total)}/${run.total})`;
  return (
    <div
      role="status"
      data-testid="archive-run"
      className={cn(
        "flex min-w-0 flex-col gap-2 rounded-[10px] border bg-surface-1 px-3.5 py-2.5 text-[13px] leading-normal",
        run.error === null ? "border-hairline" : "border-[color-mix(in_srgb,var(--accent-urgent)_50%,transparent)]",
      )}
    >
      <div className="flex min-w-0 flex-wrap items-center justify-between gap-x-3 gap-y-2">
        <p className={cn("min-w-0 flex-1 break-words font-semibold", run.error === null ? "text-text-primary" : "text-urgent")}>
          {text}
          {run.errorWorkId !== null && (
            <Link to={workPath(run.errorWorkId)} className={cn(workLink, "ml-1.5")}>
              작품 보기
            </Link>
          )}
        </p>
        {done && (
          <Button type="button" variant="ghost" className={btnNeutral} onClick={onClose}>
            닫기
          </Button>
        )}
      </div>
      {run.results.length > 0 && (
        <ul aria-label="보관한 규칙" className="m-0 flex list-none flex-col gap-1 p-0">
          {run.results.map((result) => (
            <li key={result.ruleId} className="flex min-w-0 flex-wrap items-baseline gap-x-2 gap-y-0.5 text-xs text-text-secondary">
              <span className="min-w-0 flex-1 basis-56 break-words">{result.text}</span>
              {result.workId !== null && (
                <Link to={workPath(result.workId)} className={workLink}>
                  작품 보기
                </Link>
              )}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

/** One suggestion in words: the work, where it comes from, why it is suggested and what archiving does. */
function SuggestionText({ suggestion, link = true }: { suggestion: ArchiveSuggestion; link?: boolean }) {
  const title = suggestionTitle(suggestion);
  return (
    <div className="flex min-w-0 flex-1 flex-col gap-1.5">
      <p className="min-w-0 text-[15px] leading-snug font-bold break-words">
        {link ? (
          <Link
            to={`/collect/rules?rule=${encodeURIComponent(suggestion.rule_id)}`}
            className="rounded-sm underline-offset-4 outline-offset-2 hover:underline focus-visible:outline-2 focus-visible:outline-focus"
          >
            {title}
          </Link>
        ) : (
          title
        )}
        {suggestion.state === "paused" && <span className="ml-1.5 text-xs font-semibold text-text-muted">멈춤</span>}
      </p>
      <p className="min-w-0 text-xs leading-normal break-all text-text-muted">
        {suggestion.channel_name ?? suggestion.channel_host} · {suggestion.directory || "수집 폴더"}
      </p>
      {suggestion.grounds.map((ground) => (
        <p key={ground.key} className="min-w-0 text-[13px] leading-normal break-words text-text-secondary">
          {groundText(ground, suggestion)}
        </p>
      ))}
      <p className="min-w-0 text-xs leading-normal break-words text-text-muted">{suggestion.after}</p>
    </div>
  );
}
