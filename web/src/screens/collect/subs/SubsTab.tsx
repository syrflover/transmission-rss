import { Link, Route, Routes } from "react-router-dom";

import { Button } from "@/components/ui/button";
import { useCached } from "@/lib/cached";

import { EmptyState } from "../../ScreenFrame";
import { KEYS } from "../cache";
import { btnAction, btnNeutral } from "../channels/styles";
import { PlusIcon } from "../icons";
import { AddSubscription } from "./add/AddSubscription";
import { Candidates } from "./Candidates";
import { NameTitle } from "./title/NameTitle";
import { listSubscriptions, type SubscriptionItem, type SubscriptionList } from "./api";
import { SubscriptionCard } from "./SubscriptionCard";

const grid = "m-0 grid list-none grid-cols-[repeat(auto-fill,minmax(min(100%,300px),1fr))] gap-3.5 p-0";

/**
 * The 구독 tab: the title candidates at the top, the subscriptions of this
 * quarter with the way in from the schedule, and the coming quarter's below.
 * `/collect/subs/add` is the subscribe flow and `/collect/subs/title` gives a
 * candidate to a subscription waiting for its title.
 */
export function SubsTab() {
  return (
    <Routes>
      <Route path="add" element={<AddSubscription />} />
      <Route path="title" element={<NameTitle />} />
      <Route path="*" element={<SubscriptionsView />} />
    </Routes>
  );
}

function quarterLabel(quarter: SubscriptionList["quarter"]): string {
  return `${quarter.year}년 ${quarter.number}분기`;
}

function SubscriptionsView() {
  const list = useCached<SubscriptionList>(KEYS.subscriptions, listSubscriptions, "구독을 불러오지 못했어요.");
  const data = list.data;
  const now: SubscriptionItem[] = data ? data.subscriptions.filter((s) => !s.upcoming) : [];
  const later: SubscriptionItem[] = data ? data.subscriptions.filter((s) => s.upcoming) : [];

  return (
    <div className="flex flex-col gap-4">
      <div className="flex min-h-9 flex-wrap items-center justify-between gap-x-4 gap-y-2.5 max-[720px]:min-h-10">
        <p className="min-w-[220px] flex-1 text-[13px] text-text-muted">
          {data ? `${quarterLabel(data.quarter)} 구독` : "이번 분기 구독"}
        </p>
        <Button asChild type="button" variant="ghost" className={btnAction}>
          <Link to="/collect/subs/add">
            <PlusIcon className="size-[15px]" />
            편성표에서 추가
          </Link>
        </Button>
      </div>

      <Candidates />

      {data === undefined && list.error === null && list.slow && (
        <p className="text-[13px] text-text-muted">구독을 불러오는 중이에요.</p>
      )}

      {data === undefined && list.error !== null && (
        <div className="flex flex-col items-start gap-2.5">
          <p role="alert" className="text-[13px] font-semibold text-urgent">
            {list.error}
          </p>
          <Button type="button" variant="ghost" className={btnNeutral} onClick={list.reload}>
            재시도
          </Button>
        </div>
      )}

      {data !== undefined && now.length === 0 && later.length === 0 && (
        <EmptyState>
          아직 구독한 작품이 없어요. 편성표에서 작품을 고르면 받을 채널과 저장 폴더를 정하고 구독할 수 있어요.
        </EmptyState>
      )}

      {data !== undefined && now.length > 0 && (
        <ul className={grid} aria-label={`${quarterLabel(data.quarter)} 구독`}>
          {now.map((item) => (
            <SubscriptionCard key={item.rule_id} item={item} />
          ))}
        </ul>
      )}

      {data !== undefined && now.length === 0 && later.length > 0 && (
        <p className="text-[13px] text-text-muted">이번 분기에 방영하는 구독은 아직 없어요.</p>
      )}

      {later.length > 0 && (
        <section aria-labelledby="later-subs-heading" className="flex min-w-0 flex-col gap-3">
          <div className="flex flex-wrap items-center justify-between gap-x-4 gap-y-2">
            <h2 id="later-subs-heading" className="text-[15px] font-bold">
              다음 분기 구독
            </h2>
            <Button asChild type="button" variant="ghost" className={btnNeutral}>
              <Link to="/collect/subs/add?week=8">신작에서 추가</Link>
            </Button>
          </div>
          <p className="min-w-0 text-[13px] leading-normal text-text-muted">
            다음 분기 작품은 Anissia의 신작에서 골라요. 첫 화가 채널에 올라오기 전이면 제목 대기로 구독해 두고, 새 작품 제목이 나타나면 위에
            제목 후보로 알려요.
          </p>
          <ul className={grid}>
            {later.map((item) => (
              <SubscriptionCard key={item.rule_id} item={item} />
            ))}
          </ul>
        </section>
      )}
    </div>
  );
}
