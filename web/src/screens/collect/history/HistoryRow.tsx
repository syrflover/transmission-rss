import { useId, useState, type MouseEvent } from "react";
import { Link } from "react-router-dom";

import { Button } from "@/components/ui/button";

import { btnAction } from "../channels/styles";
import { nameTitleLink } from "../subs/Candidates";
import type { HistoryItem } from "./api";
import { RESULT_LABEL } from "./filters";
import { clock } from "./format";
import { ChevronIcon, PlusIcon } from "./icons";
import { RetryActions } from "./RetryActions";
import { useRetry, type RetryPhase } from "./useRetry";

/** The new-rule screen (ticket 0006), opened with the channel and the item's title filled in. */
export function newRuleLink(item: Pick<HistoryItem, "channel_id" | "title">): string {
  return `/collect/rules/new?channel=${encodeURIComponent(item.channel_id)}&match=${encodeURIComponent(item.title)}`;
}

/** Why a row was not received, for the expanded part. */
function whyNot(item: HistoryItem): string {
  switch (item.result) {
    case "no_match":
      return "어떤 규칙에도 맞지 않아서 받지 않았어요.";
    case "excluded":
      return "채널의 제외 단어에 걸려서 받지 않았어요.";
    case "add_failed":
      return "Transmission에 넣지 못해서 받지 못했어요.";
    default:
      return "";
  }
}

function inProgress(phase: RetryPhase): boolean {
  return phase.kind === "sending" || phase.kind === "waiting" || phase.kind === "unconfirmed";
}

/** The dimmed line under the title: what the row is doing or why it ended as it did. */
function statusLine(item: HistoryItem, phase: RetryPhase): { text: string; urgent: boolean } | null {
  switch (phase.kind) {
    case "sending":
      return { text: "접수하는 중이에요.", urgent: false };
    case "waiting":
      return { text: "접수됐어요. Transmission에 넣는 중이에요.", urgent: false };
    case "unconfirmed":
      return {
        text: phase.lost
          ? "접수됐는지 확인했더니 서버에 요청이 없어요. 같은 요청을 다시 보낼 수 있어요."
          : "접수됐는지 확인하지 못했어요. 결과를 알 수 없어 다시 확인하고 있어요.",
        urgent: false,
      };
    case "ended":
      if (phase.message === "") break;
      return { text: phase.message, urgent: phase.failed };
    default:
      break;
  }
  switch (item.result) {
    case "received":
      return {
        text: [item.by_hand ? "직접 추가함" : item.rule_label ? `규칙 ‘${item.rule_label}’` : "", item.reason ?? ""]
          .filter((part) => part !== "")
          .join(" · "),
        urgent: false,
      };
    case "duplicate":
      return { text: item.reason ?? "Transmission에 이미 있어요.", urgent: false };
    case "add_failed":
      return item.reason ? { text: item.reason, urgent: true } : null;
    default:
      return null;
  }
}

const chipBase =
  "inline-flex items-center gap-1 rounded-full border px-2.5 py-0.5 text-xs font-bold whitespace-nowrap";

interface HistoryRowProps {
  item: HistoryItem;
  onItem: (item: HistoryItem) => void;
}

/**
 * One history row. What happened comes first (the result chip, the title);
 * the channel and time sit dimmed beside it. A row that Transmission does not
 * hold expands to say why it was not added, and to offer `규칙 생성` and, for an
 * item a rule picked and failed to add, `다시 받기` (or why that is missing).
 */
export function HistoryRow({ item, onItem }: HistoryRowProps) {
  const [open, setOpen] = useState(false);
  const { phase, submit, resend, recheck } = useRetry(item, onItem);
  const detailId = useId();
  const busy = inProgress(phase);
  // A row Transmission holds has nothing to offer; the others say why they were not added.
  const expandable = item.result !== "received" && item.result !== "duplicate";
  const status = statusLine(item, phase);
  const failed = item.result === "add_failed" || (phase.kind === "ended" && phase.failed);

  const chip = busy ? (
    <span className={`${chipBase} border-focus text-focus`}>추가하는 중</span>
  ) : (
    <span
      className={`${chipBase} ${failed ? "border-urgent text-urgent" : "border-hairline bg-surface-2 text-text-primary"}`}
    >
      {RESULT_LABEL[item.result]}
    </span>
  );

  const toggleFromRow = (event: MouseEvent) => {
    if (!expandable) return;
    if ((event.target as HTMLElement).closest("button, a, input, label, form")) return;
    setOpen((was) => !was);
  };

  return (
    <li
      data-history-id={item.id}
      data-result={item.result}
      onClick={toggleFromRow}
      className={`grid grid-cols-[104px_minmax(0,1fr)_auto_32px] items-start gap-x-3 gap-y-1 border-b border-hairline-soft py-2.5 pr-2 pl-2.5 max-[720px]:grid-cols-[minmax(0,1fr)_32px] max-[720px]:pr-0.5 max-[720px]:pl-1 ${
        expandable ? "cursor-pointer hover:bg-surface-1" : ""
      } ${open ? "bg-surface-1" : ""}`}
    >
      <div className="contents max-[720px]:col-start-1 max-[720px]:row-start-1 max-[720px]:flex max-[720px]:flex-wrap max-[720px]:items-center max-[720px]:gap-x-2 max-[720px]:gap-y-1">
        <span className="col-start-1 row-start-1 mt-px justify-self-start max-[720px]:mt-0">{chip}</span>
        <span className="col-start-3 row-start-1 mt-0.5 text-xs whitespace-nowrap text-text-muted max-[720px]:mt-0 max-[720px]:whitespace-normal">
          {item.channel_name}
          {item.channel_deleted ? " (삭제됨)" : ""}
          <span aria-hidden="true"> · </span>
          <time dateTime={new Date(item.first_seen_at).toISOString()}>{clock(item.first_seen_at)}</time>
        </span>
      </div>

      <div className="col-start-2 row-start-1 min-w-0 max-[720px]:col-start-1 max-[720px]:row-start-2">
        <p className="line-clamp-2 text-[13px] leading-[1.45] font-medium [overflow-wrap:anywhere] text-text-primary">
          {item.title}
        </p>
        {item.name_title !== null && (
          <p className="mt-1">
            <Link
              to={nameTitleLink(item.channel_id, item.name_title.work, item.name_title.folder)}
              data-testid="name-title-link"
              className="inline-flex min-h-6 items-center rounded-sm text-xs font-semibold text-focus underline-offset-4 outline-offset-2 hover:underline focus-visible:outline-2 focus-visible:outline-focus"
            >
              제목 대기 구독에 잇기
            </Link>
          </p>
        )}
        <p
          role={phase.kind === "idle" ? undefined : "status"}
          className={`mt-0.5 text-xs leading-[1.45] [overflow-wrap:anywhere] empty:hidden ${
            status?.urgent ? "text-urgent" : "text-text-secondary"
          }`}
        >
          {status?.text}
        </p>
      </div>

      {expandable ? (
        <button
          type="button"
          aria-expanded={open}
          aria-controls={detailId}
          aria-label={`${item.title}: 할 수 있는 일 ${open ? "접기" : "보기"}`}
          onClick={() => setOpen((was) => !was)}
          className="col-start-4 row-start-1 -mt-1 inline-flex size-[30px] items-center justify-center rounded-lg text-text-muted hover:bg-surface-2 hover:text-text-primary max-[720px]:col-start-2 max-[720px]:row-span-2 max-[720px]:row-start-1 max-[720px]:self-center"
        >
          <ChevronIcon className={`size-[15px] transition-transform ${open ? "rotate-90" : ""}`} />
        </button>
      ) : null}

      {expandable && open ? (
        <div
          id={detailId}
          className="col-start-2 col-end-[-1] flex flex-col gap-3 pt-1 pb-1.5 max-[720px]:col-start-1 max-[720px]:col-end-[-1]"
        >
          <p className="text-[12.5px] leading-[1.5] text-text-secondary">{whyNot(item)}</p>
          {item.can_retry ? (
            <RetryActions phase={phase} onSubmit={submit} onResend={resend} onRecheck={recheck}>
              <NewRuleButton item={item} />
            </RetryActions>
          ) : (
            <>
              {item.retry_blocked !== null && <p className="text-xs text-text-muted">{item.retry_blocked}</p>}
              <div className="flex flex-wrap gap-2">
                <NewRuleButton item={item} />
              </div>
            </>
          )}
        </div>
      ) : null}
    </li>
  );
}

function NewRuleButton({ item }: { item: HistoryItem }) {
  return (
    <Button asChild variant="ghost" className={btnAction}>
      <Link to={newRuleLink(item)}>
        <PlusIcon className="size-[15px]" />
        규칙 생성
      </Link>
    </Button>
  );
}
