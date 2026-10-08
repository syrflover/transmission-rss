import { Button } from "@/components/ui/button";
import { useAfterDelay } from "@/lib/cached";
import { cn } from "@/lib/utils";

import { btnNeutral } from "../channels/styles";
import { ReceivedAs } from "../subs/add/PastItems";
import { PHASE_TEXT, type ReceivePhase } from "../subs/add/useReceive";
import type { Preview, PreviewItem, PreviewKind } from "./api";
import type { PreviewState } from "./usePreview";

const KIND: Record<PreviewKind, { label: string; badge: string }> = {
  mine: { label: "이 규칙이 받아요", badge: "border-ok text-ok" },
  earlier: { label: "앞 규칙이 가져가요", badge: "border-arch text-arch" },
  excluded: { label: "채널 제외 조건", badge: "border-hairline text-text-muted" },
  past: { label: "지난 회차", badge: "border-hairline text-text-secondary" },
};

const STORED: Record<string, string> = {
  received: "추가함",
  no_match: "규칙 불일치",
  excluded: "제외",
  duplicate: "중복",
  add_failed: "추가 실패",
  version_unknown: "버전 미상",
};

/**
 * How a past item of a subscription rule is received from its row: the rule
 * receives it when the user asks, with the command the subscribe flow uses.
 */
export interface PastReceive {
  /** Where the item's command is; `undefined` before `받기` was pressed. */
  phaseOf: (itemId: number) => ReceivePhase | undefined;
  onReceive: (itemId: number) => void;
  /** Why `받기` is not offered now, as a sentence; `null` when it is. */
  blocked: string | null;
}

/** The core action of a past item's row: receive it, and follow how that goes. */
function PastAction({ item, receive }: { item: PreviewItem; receive: PastReceive }) {
  if (receive.blocked) {
    return <p className="min-w-0 text-xs text-text-muted">{receive.blocked}</p>;
  }
  const phase = receive.phaseOf(item.id);
  if (!phase || phase.kind === "failed") {
    return (
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1.5">
        <Button type="button" variant="ghost" className={btnNeutral} onClick={() => receive.onReceive(item.id)}>
          {phase ? "다시 받기" : "받기"}
        </Button>
        {phase?.kind === "failed" && (
          <span role="alert" className="min-w-0 text-xs break-words text-urgent">
            {PHASE_TEXT.failed}. {phase.message}
          </span>
        )}
      </div>
    );
  }
  return (
    <span
      role="status"
      className={cn("text-xs font-semibold", phase.kind === "added" ? "text-ok" : "text-text-secondary")}
    >
      {PHASE_TEXT[phase.kind]}
    </span>
  );
}

function Row({ item, receive }: { item: PreviewItem; receive?: PastReceive }) {
  const kind = KIND[item.kind];
  return (
    <li className="flex min-w-0 flex-col gap-1 rounded-[10px] border border-hairline-soft bg-surface-2 px-3 py-2.5">
      <div className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1">
        <span
          className={cn(
            "inline-flex flex-none items-center rounded-full border px-2 py-px text-[11.5px] font-semibold",
            kind.badge,
          )}
        >
          {kind.label}
        </span>
        {item.masked && (
          <span className="flex-none rounded-full border border-hairline px-2 py-px text-[11.5px] font-semibold text-text-muted">
            가려진 제목
          </span>
        )}
        <span className="ml-auto flex-none text-[11.5px] text-text-muted">
          기록: {STORED[item.stored_result] ?? item.stored_result}
        </span>
      </div>
      <p className="min-w-0 text-[13px] leading-snug break-all" data-testid="preview-title">
        {item.title}
      </p>
      <ReceivedAs item={item} />
      {item.kind === "mine" && item.save_path && (
        <p className="min-w-0 font-mono text-xs break-all text-text-secondary">→ {item.save_path}</p>
      )}
      {item.kind === "past" && (
        <p className="min-w-0 text-xs text-text-secondary">
          {item.past_cause === "resumed"
            ? "규칙이 멈춰 있는 동안 올라온 항목이라 고르기 전에는 받지 않아요."
            : item.past_cause === "titled"
              ? "제목을 정하기 전에 올라온 항목이라 고르기 전에는 받지 않아요."
              : item.past_cause === "first_read"
                ? "채널을 처음 읽을 때 피드에 이미 있던 항목이라 고르기 전에는 받지 않아요."
                : "구독하기 전에 올라온 항목이라 고르기 전에는 받지 않아요."}
          {item.save_path && <span className="block font-mono break-all text-text-muted">→ {item.save_path}</span>}
        </p>
      )}
      {item.kind === "past" && receive && <PastAction item={item} receive={receive} />}
      {item.kind === "earlier" && item.taken_by && (
        <p className="min-w-0 text-xs break-all text-text-secondary">
          {item.taken_by.match ?? "제목 대기"} 규칙이 먼저 맞아서 그쪽 폴더로 가요.
          {item.save_path && <span className="block font-mono text-text-muted">→ {item.save_path}</span>}
        </p>
      )}
      {item.kind === "excluded" && item.excluded_by && (
        <p className="min-w-0 text-xs break-all text-text-secondary">
          제외 조건 <span className="font-mono">{item.excluded_by}</span> 때문에 받지 않아요.
        </p>
      )}
    </li>
  );
}

function Summary({ preview }: { preview: Preview }) {
  const { counts } = preview;
  if (counts.total === 0) {
    return (
      <p className="text-[13px] leading-normal text-text-secondary">
        이 채널의 수집 기록이 아직 없어서 비교할 항목이 없어요. worker가 채널을 한 번 읽으면 여기서 볼 수 있어요.
      </p>
    );
  }
  return (
    <p className="text-[13px] leading-normal text-text-secondary" data-testid="preview-summary">
      기록된 항목 {counts.total}개 중 이 규칙이 받는 항목 <strong>{counts.mine}개</strong>
      {counts.earlier > 0 && <>, 앞 규칙이 가져가는 항목 {counts.earlier}개</>}
      {counts.excluded > 0 && <>, 제외 조건에 걸리는 항목 {counts.excluded}개</>}
      {counts.past > 0 && <>, 고르기 전에는 받지 않는 지난 회차 {counts.past}개</>}
      {counts.mine + counts.earlier + counts.excluded + counts.past === 0 && <>. 맞는 항목이 없어요</>}
      {"."}
    </p>
  );
}

/**
 * What the rule as edited would do with the items the worker recorded for its
 * channel. The judgement comes from the server; this only shows it.
 */
export function RulePreview({ state, receive }: { state: PreviewState; receive?: PastReceive }) {
  const preview =
    state.state === "ready" ? state.preview : state.state === "loading" ? state.previous : null;
  const stale = state.state === "loading";
  // A quick answer never shows the loading line.
  const slow = useAfterDelay(!preview && state.state !== "failed");

  return (
    <section aria-labelledby="preview-heading" className="flex min-w-0 flex-col gap-2.5">
      <div className="flex items-baseline justify-between gap-3">
        <h3 id="preview-heading" className="text-[15px] font-bold">
          미리보기
        </h3>
        <span className="text-xs text-text-muted" role="status">
          {stale ? "확인하는 중" : "저장하기 전에 판정만 보여줘요"}
        </span>
      </div>

      {state.state === "failed" && (
        <p role="alert" className="text-[13px] font-semibold text-urgent">
          {state.message}
        </p>
      )}

      {preview?.error && (
        <p role="alert" className="text-[13px] leading-normal font-semibold text-urgent" data-testid="preview-error">
          {preview.error.message}
          <span className="mt-0.5 block font-mono text-xs font-normal break-all text-text-muted">
            {preview.error.detail}
          </span>
        </p>
      )}

      {preview && !preview.error && (
        <div className={cn("flex min-w-0 flex-col gap-2.5", stale && "opacity-60")}>
          <Summary preview={preview} />
          {preview.items.length > 0 && (
            <ul className="m-0 flex list-none flex-col gap-2 p-0">
              {preview.items.map((item) => (
                <Row key={item.id} item={item} receive={receive} />
              ))}
            </ul>
          )}
          {preview.truncated && (
            <p className="text-xs text-text-muted">맞는 항목이 많아서 최근 {preview.items.length}개만 보여줘요.</p>
          )}
          {preview.masked_total > 0 && (
            <p className="text-xs leading-normal text-text-muted">
              기록의 제목 {preview.masked_total}개는 채널 주소의 비밀 값이 가려져 있어요. 원래 제목과 다를 수 있어서 worker의 판정과 다르게 보일 수 있어요.
            </p>
          )}
        </div>
      )}

      {slow && <p className="text-[13px] text-text-muted">미리보기를 불러오는 중이에요.</p>}
    </section>
  );
}
