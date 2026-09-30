import { Button } from "@/components/ui/button";
import { ago, dateTime } from "@/lib/time";

import { btnNeutral } from "../channels/styles";
import type { Rule } from "./api";
import { parseEpisode, type Draft } from "./draft";

/**
 * The four lines at the top of every rule, filled the same way for each so the
 * poster keeps one size. Broadcast, subtitle group and progress come from the
 * subscription and Anissia link that do not exist yet; only the last
 * collection is known today.
 */
export function RuleSummary({ title, lastReceivedAt }: { title: string; lastReceivedAt: number | null }) {
  const lines: [string, string][] = [
    ["방영", "미정"],
    ["제작자", "미정"],
    ["진행", "아직 없음"],
    ["마지막 수집", lastReceivedAt === null ? "아직 없음" : `${ago(lastReceivedAt)} (${dateTime(lastReceivedAt)})`],
  ];
  return (
    <>
      <div
        aria-hidden="true"
        className="col-start-1 row-span-2 row-start-1 flex aspect-[2/3] w-full items-center justify-center self-start rounded-xl border border-hairline-soft bg-surface-3 text-3xl font-bold text-text-muted max-[720px]:row-span-1 max-[720px]:row-start-2"
      >
        {Array.from(title)[0] ?? ""}
      </div>
      <dl
        aria-label="규칙 요약"
        className="col-start-2 row-start-2 m-0 grid min-w-0 grid-cols-[76px_minmax(0,1fr)] content-start gap-x-3 gap-y-2 text-[13px] max-[720px]:row-start-2"
      >
        {lines.map(([label, value]) => (
          <div key={label} className="contents">
            <dt className="font-semibold text-text-muted">{label}</dt>
            <dd className="m-0 min-w-0 text-text-secondary">{value}</dd>
          </div>
        ))}
      </dl>
    </>
  );
}

const CHANGED_FIELDS: { label: string; server: (r: Rule) => string; mine: (d: Draft) => string }[] = [
  { label: "일치 문구", server: (r) => r.match ?? "(비어 있음)", mine: (d) => d.match || "(비어 있음)" },
  { label: "정규식", server: (r) => (r.regex ? "켬" : "끔"), mine: (d) => (d.regex ? "켬" : "끔") },
  {
    label: "대소문자",
    server: (r) => (r.case_insensitive ? "무시" : "구분"),
    mine: (d) => (d.case_insensitive ? "무시" : "구분"),
  },
  { label: "저장 폴더", server: (r) => r.directory || "(비어 있음)", mine: (d) => d.directory.trim() || "(비어 있음)" },
  {
    label: "회차 변환",
    server: (r) => String(r.episode),
    mine: (d) => (parseEpisode(d.episode) === null ? d.episode : String(parseEpisode(d.episode))),
  },
  {
    label: "상태",
    server: (r) => (r.state === "archived" ? "보관됨" : "수집 중"),
    mine: (d) => (d.state === "archived" ? "보관됨" : "수집 중"),
  },
];

/** The server's rule after someone else saved first, beside the untouched input. */
export function ConflictNotice({ current, draft }: { current: Rule; draft: Draft }) {
  const differing = CHANGED_FIELDS.filter((f) => f.server(current) !== f.mine(draft));
  return (
    <section role="alert" className="flex min-w-0 flex-col gap-2 rounded-xl border border-hairline bg-surface-2 p-3.5">
      <p className="text-sm font-bold">다른 곳에서 먼저 저장했어요.</p>
      <p className="text-[13px] leading-normal text-text-secondary">
        입력한 값은 그대로 두었어요. 지금 저장된 값과 다른 항목은 아래와 같아요. 확인하고 다시 저장할 수 있어요.
      </p>
      {differing.length > 0 ? (
        <dl className="m-0 grid grid-cols-[92px_minmax(0,1fr)] gap-x-3 gap-y-2 text-[13px] max-[480px]:grid-cols-1 max-[480px]:gap-y-0.5">
          {differing.map((f) => (
            <div key={f.label} className="contents">
              <dt className="font-semibold text-text-muted">{f.label}</dt>
              <dd className="m-0 min-w-0 break-all max-[480px]:mb-1.5">
                <span className="block">
                  <span className="text-text-muted">저장된 값 </span>
                  <span data-testid="conflict-server">{f.server(current)}</span>
                </span>
                <span className="block">
                  <span className="text-text-muted">내 입력 </span>
                  <span className="font-semibold">{f.mine(draft)}</span>
                </span>
              </dd>
            </div>
          ))}
        </dl>
      ) : (
        <p className="text-[13px] text-text-secondary">저장된 값이 입력과 같아요. 저장하면 이 화면이 최신 버전이 돼요.</p>
      )}
    </section>
  );
}

/** Moves the rule within its channel's check order (applied when saved). */
export function OrderRow({
  siblings,
  position,
  onMove,
}: {
  /** The channel's rules in check order, this one included. */
  siblings: Rule[];
  /** Where the edited rule stands among them, from 0. */
  position: number;
  onMove: (to: number) => void;
}) {
  const others = siblings.length;
  return (
    <div className="flex min-w-0 flex-col gap-2">
      <div className="flex flex-wrap items-center justify-between gap-x-3 gap-y-2">
        <p className="text-[13px] font-semibold" data-testid="order-position">
          이 채널에서 {position + 1}번째로 검사해요 <span className="font-normal text-text-muted">({others}개 중)</span>
        </p>
        <div className="flex gap-2">
          <Button
            type="button"
            variant="ghost"
            className={btnNeutral}
            disabled={position <= 0}
            onClick={() => onMove(position - 1)}
          >
            앞으로
          </Button>
          <Button
            type="button"
            variant="ghost"
            className={btnNeutral}
            disabled={position >= others - 1}
            onClick={() => onMove(position + 1)}
          >
            뒤로
          </Button>
        </div>
      </div>
      <p className="text-xs leading-normal text-text-muted">
        두 규칙이 같은 항목에 맞으면 앞 규칙이 가져가요. 순서는 저장할 때 바뀌어요.
      </p>
    </div>
  );
}
