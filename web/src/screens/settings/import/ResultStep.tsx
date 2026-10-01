import { CARD } from "../parts";
import { ActionBar, Banner, BTN, Facts, Labeled } from "../parts";
import { RuleName } from "./ReviewStep";
import type { ImportFlow } from "./useImportFlow";

/** What became of the subscription suggestions the user checked. */
function Subscriptions({ flow }: { flow: ImportFlow }) {
  const subscriptions = flow.result?.subscriptions;
  const preview = flow.preview;
  if (!subscriptions || !preview) return null;
  const { created, not_created: notCreated, unavailable } = subscriptions;
  if (created.length === 0 && notCreated.length === 0) return null;
  const phrase = (channel: number, rule: number) =>
    preview.channels.find((candidate) => candidate.index === channel)?.rules[rule]?.match ?? null;
  const unknown = created.filter((sub) => !sub.schedule_known).length;
  const unknownInOther = created.filter((sub) => !sub.schedule_known && !sub.schedule_from_comment).length;

  return (
    <section aria-labelledby="result-subscriptions" className="flex flex-col gap-3">
      <h3 id="result-subscriptions" className="m-0 text-base font-bold">
        구독
      </h3>
      {created.length > 0 && (
        <>
          <p className="text-[13.5px] leading-relaxed text-text-secondary">
            체크한 제안 {created.length}개를 구독으로 가져왔어요. 구독은 가져올 때 아무것도 받지 않았어요. 채널을
            처음 읽을 때 피드에 이미 있던 항목과 구독 전에 기록된 항목은 규칙 상세의 지난 회차에서 직접 골라 받을 수
            있어요. 구독이 아닌 규칙은 다음 RSS 확인부터 맞는 항목을 받아요.
          </p>
          <ul aria-label="구독으로 가져온 규칙" className="m-0 flex list-none flex-col gap-1.5 p-0 text-[13.5px]">
            {created.map((sub) => (
              <li key={`${sub.channel}:${sub.rule}`} className="flex flex-wrap items-baseline gap-x-2.5 gap-y-0.5">
                <RuleName phrase={phrase(sub.channel, sub.rule)} />
                <span className="text-xs text-text-muted">
                  {sub.subject ?? (sub.schedule_from_comment ? "주석의 요일·시간" : "요일·시간 미정")} ·{sub.creator ? `${sub.creator} 따라 받기` : "제작자 미정"}
                </span>
              </li>
            ))}
          </ul>
        </>
      )}
      {unknown > 0 && (
        <p className="text-[13.5px] leading-relaxed text-text-secondary">
          {unavailable ?? "Anissia 편성표에서 찾지 못한 작품이 있어요."} 방영 요일과 시간을 읽지 못한 구독 {unknown}개는
          그대로 만들고, 앱이 Anissia에서 다시 읽어 채울 때까지{" "}
          {unknownInOther === 0
            ? "주석에 적힌 요일과 시간으로 보여요."
            : unknownInOther === unknown
              ? "요일 자리에 기타로 보여요."
              : `주석에 적힌 요일과 시간으로 보여요(주석에 요일이 없는 ${unknownInOther}개는 기타).`}
        </p>
      )}
      {notCreated.length > 0 && (
        <>
          <p className="text-[13.5px] leading-relaxed text-text-secondary">
            체크했지만 구독으로 만들지 않은 제안 {notCreated.length}개는 규칙만 가져왔어요.
          </p>
          <ul aria-label="구독으로 만들지 않은 제안" className="m-0 flex list-none flex-col gap-1.5 p-0 text-[13.5px]">
            {notCreated.map((sub) => (
              <li key={`${sub.channel}:${sub.rule}`} className="flex flex-wrap items-baseline gap-x-2.5 gap-y-0.5">
                <RuleName phrase={phrase(sub.channel, sub.rule)} />
                <span className="text-xs text-text-muted">{sub.reason}</span>
              </li>
            ))}
          </ul>
        </>
      )}
    </section>
  );
}

export function ResultStep({ flow }: { flow: ImportFlow }) {
  const result = flow.result;
  if (!result) return null;
  const { counts } = result;
  const removedChannels = result.replaced.filter((channel) => channel.removed_rules.length > 0);

  return (
    <div className="flex flex-col gap-6">
      <Banner tone="done" role="status" title="가져오기를 마쳤어요">
        채널과 규칙, 체크한 구독만 앱에 저장했어요. 아무것도 받지 않았고, 다운로드를 시작하거나 자막을 적용하거나 파일을
        바꾸거나 정리하지 않았으며, 과거 승인도 되살리지 않았어요.
      </Banner>

      <section aria-labelledby="result-counts" className="flex flex-col gap-3">
        <h3 id="result-counts" className="m-0 text-base font-bold">
          가져온 결과
        </h3>
        <Facts className="gap-y-2">
          <Labeled label="추가한 채널">{counts.channels_added}개</Labeled>
          <Labeled label="교체한 채널">{counts.channels_replaced}개</Labeled>
          <Labeled label="건너뛴 채널">{counts.channels_skipped}개</Labeled>
          <Labeled label="가져오지 않은 채널">{counts.channels_not_imported}개</Labeled>
          <Labeled label="그대로 남은 채널">{counts.channels_unchanged}개</Labeled>
        </Facts>
        <Facts className="gap-y-2">
          <Labeled label="추가한 규칙">{counts.rules_added}개</Labeled>
          <Labeled label="값만 바꾼 규칙">{counts.rules_kept}개</Labeled>
          <Labeled label="없앤 규칙">{counts.rules_removed}개</Labeled>
          <Labeled label="구독으로 가져온 규칙">{counts.subscriptions_created}개</Labeled>
        </Facts>
        <ul className="m-0 flex list-none flex-col gap-2 p-0 text-[13.5px] leading-relaxed text-text-secondary">
          {result.added.map((channel) => (
            <li key={channel.id}>
              <b className="text-text-primary break-all">{channel.url}</b> 채널과 규칙 {channel.rule_count}개를 새로
              추가했어요.
            </li>
          ))}
          {result.replaced.map((channel) => (
            <li key={channel.id}>
              <b className="text-text-primary break-all">{channel.url}</b> 채널을 파일 내용으로 바꿨어요. 규칙{" "}
              {channel.kept_rules}개는 지금 규칙의 ID를 유지한 채 값만 바꿨고, {channel.added_rules}개를 새로 더했고,{" "}
              {channel.removed_rules.length}개를 없앴어요.
            </li>
          ))}
          {result.collect_folder_set !== null && (
            <li>
              수집 폴더를 <b className="text-text-primary font-mono break-all">{result.collect_folder_set}</b>로
              정했어요.
            </li>
          )}
          {result.not_imported.map((channel) => (
            <li key={channel.index}>
              <b className="text-text-primary break-all">{channel.url}</b> 채널은 가져오지 않았어요. {channel.reason}
            </li>
          ))}
          {result.skipped.map((channel) => (
            <li key={channel.index}>
              <b className="text-text-primary break-all">{channel.url}</b> 채널은 건너뛰어서 아무것도 바꾸지 않았어요.
            </li>
          ))}
        </ul>
      </section>

      <Subscriptions flow={flow} />

      {removedChannels.length > 0 && (
        <section aria-labelledby="result-removed" className="flex flex-col gap-3">
          <h3 id="result-removed" className="m-0 text-base font-bold">
            없앤 규칙
          </h3>
          <p className="text-[13.5px] leading-relaxed text-text-secondary">
            아래 규칙은 파일에 없어서 없앴어요. 그 규칙으로 받은 파일과 보관 기록은 남아 있어요.
          </p>
          {removedChannels.map((channel) => (
            <div key={channel.id} className={`${CARD} overflow-hidden`}>
              <h4 className="m-0 border-b border-hairline-soft px-4 py-3 text-sm font-bold break-all">
                {channel.url}
              </h4>
              <ul aria-label="없앤 규칙" className="m-0 flex list-none flex-col gap-1.5 p-0 px-4 py-3 text-[13.5px]">
                {channel.removed_rules.map((rule, i) => (
                  <li key={i} className="flex flex-wrap items-baseline gap-x-2.5 gap-y-0.5">
                    <RuleName phrase={rule.match} />
                    <span className="text-xs text-text-muted">{rule.directory || "저장 폴더 없음"}</span>
                  </li>
                ))}
              </ul>
            </div>
          ))}
        </section>
      )}

      <ActionBar
        buttons={
          <button type="button" className={BTN.plain} onClick={flow.restart}>
            파일 선택으로
          </button>
        }
      >
        다른 파일을 이어서 가져오려면 파일 선택으로 돌아가요.
      </ActionBar>
    </div>
  );
}
