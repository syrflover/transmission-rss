import { Link } from "react-router-dom";

import { Button } from "@/components/ui/button";
import { useCached } from "@/lib/cached";

import { KEYS } from "../cache";
import { btnAction } from "../channels/styles";
import { nameTitleLink } from "../subs/Candidates";
import { fetchCandidates, type TitleCandidate } from "../subs/api";
import type { Rule } from "./api";

const calm = "rounded-xl border border-hairline bg-surface-2 px-3.5 py-3 text-[13px] leading-normal text-text-secondary";
const blue =
  "flex min-w-0 flex-col gap-2.5 rounded-xl border border-[color-mix(in_srgb,var(--focus-ring)_45%,transparent)] bg-[color-mix(in_srgb,var(--focus-ring)_7%,transparent)] px-3.5 py-3 text-[13px] leading-normal text-text-secondary";

/**
 * The status banner of a subscription that waits for its title (`match` is
 * `null`): why it receives nothing, and what comes next. When works have
 * appeared in the channel, it says so and opens the page that gives one to
 * this subscription.
 */
export function WaitingBanner({ rule }: { rule: Rule }) {
  const list = useCached<TitleCandidate[]>(KEYS.candidates, fetchCandidates, "제목 후보를 불러오지 못했어요.");

  if (rule.state === "paused") {
    return (
      <p className={calm}>
        이 구독은 일치 문구 없이 제목을 기다리고 있어요. 영상 받기가 꺼져 있어서 새 작품 제목이 나타나도 제목 후보로 알리지 않아요. 다시 켜면
        그 뒤에 나타나는 제목부터 알려요.
      </p>
    );
  }

  const mine = (list.data ?? []).filter((c) => c.channel_id === rule.channel_id && c.waiting.some((w) => w.rule_id === rule.id));
  if (mine.length > 0) {
    const first = mine[0];
    return (
      <div className={blue} data-testid="title-candidate-banner">
        <p>
          제목 후보가 {mine.length}개 있어요. 이 구독은 아직 일치 문구가 없어서 아무것도 받지 않아요. 후보를 이 구독의 제목으로 정하면 그
          제목의 다음 회차부터 자동으로 받고, 이미 기록된 지난 항목은 미리보기에서 고른 것만 받아요.
        </p>
        <ul className="m-0 flex list-none flex-col gap-1 p-0">
          {mine.slice(0, 3).map((c) => (
            <li key={c.key} className="min-w-0 break-all font-semibold text-text-primary">
              {c.work}
            </li>
          ))}
          {mine.length > 3 && <li className="text-xs text-text-muted">외 {mine.length - 3}개</li>}
        </ul>
        <div className="flex flex-wrap gap-2">
          <Button asChild type="button" variant="ghost" className={btnAction}>
            <Link to={nameTitleLink(first.channel_id, first.work, first.folder)}>{mine.length > 1 ? "첫 후보 정하기" : "정하기"}</Link>
          </Button>
          <Button asChild type="button" variant="ghost" className={btnAction}>
            <Link to="/collect/subs">구독 탭에서 보기</Link>
          </Button>
        </div>
      </div>
    );
  }

  return (
    <p className={calm}>
      이 구독은 아직 첫 화 전이라 일치 문구가 없어요. 그래서 아무것도 받지 않아요. 이 채널에 새 작품 제목이 처음 나타나면 구독 탭에 제목 후보로
      알려요. 후보를 이 구독의 제목으로 정하면 그 제목의 다음 회차부터 자동으로 받아요.
    </p>
  );
}
