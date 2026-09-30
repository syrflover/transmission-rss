import { CARD } from "../parts";
import { ActionBar, Banner, BTN, Facts, Labeled } from "../parts";
import { RuleName } from "./ReviewStep";
import type { ImportFlow } from "./useImportFlow";

export function ResultStep({ flow }: { flow: ImportFlow }) {
  const result = flow.result;
  if (!result) return null;
  const { counts } = result;
  const removedChannels = result.replaced.filter((channel) => channel.removed_rules.length > 0);

  return (
    <div className="flex flex-col gap-6">
      <Banner tone="done" role="status" title="가져오기를 마쳤어요">
        채널과 규칙만 앱에 저장했어요. 가져오기는 다운로드를 시작하거나, 자막을 적용하거나, 파일을 바꾸거나 정리하지
        않아요.
      </Banner>

      <section aria-labelledby="result-counts" className="flex flex-col gap-3">
        <h3 id="result-counts" className="m-0 text-base font-bold">
          가져온 결과
        </h3>
        <Facts className="gap-y-2">
          <Labeled label="추가한 채널">{counts.channels_added}개</Labeled>
          <Labeled label="교체한 채널">{counts.channels_replaced}개</Labeled>
          <Labeled label="건너뛴 채널">{counts.channels_skipped}개</Labeled>
          <Labeled label="그대로 남은 채널">{counts.channels_unchanged}개</Labeled>
        </Facts>
        <Facts className="gap-y-2">
          <Labeled label="추가한 규칙">{counts.rules_added}개</Labeled>
          <Labeled label="값만 바꾼 규칙">{counts.rules_kept}개</Labeled>
          <Labeled label="없앤 규칙">{counts.rules_removed}개</Labeled>
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
          {result.skipped.map((channel) => (
            <li key={channel.index}>
              <b className="text-text-primary break-all">{channel.url}</b> 채널은 건너뛰어서 아무것도 바꾸지 않았어요.
            </li>
          ))}
        </ul>
      </section>

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
