import { cn } from "@/lib/utils";

import { AlertIcon, BackIcon, CheckIcon, ChevronIcon } from "../icons";
import { ActionBar, Banner, BTN, CARD, Facts, Labeled, Tag } from "../parts";
import type { ChannelView, Decision, RuleView } from "./types";
import type { ImportFlow } from "./useImportFlow";

const OPTIONS: { value: Decision; label: string; describe: (channel: ChannelView) => string }[] = [
  {
    value: "replace",
    label: "교체",
    describe: (channel) => {
      const gone = channel.existing?.removed_rules.length ?? 0;
      return gone
        ? `채널을 파일 내용으로 통째로 바꿔요. 파일에 없는 지금 규칙 ${gone}개는 없어지고, 그 규칙으로 받은 파일과 보관 기록은 남아요.`
        : "채널을 파일 내용으로 통째로 바꿔요. 파일에 없는 지금 규칙은 없어요.";
    },
  },
  {
    value: "add",
    label: "추가",
    describe: () => "채널과 규칙을 새 ID를 붙인 복사본으로 더해요. 기존 채널과 규칙은 건드리지 않아요.",
  },
  {
    value: "skip",
    label: "건너뛰기",
    describe: () => "이 채널은 가져오지 않아요.",
  },
];

/** A rule's episode offset with a real minus sign. */
function signed(episode: number): string {
  return episode < 0 ? `−${-episode}` : String(episode);
}

/** A rule's name is its match phrase; a rule without one is still waiting for its title. */
export function RuleName({ phrase }: { phrase: string | null }) {
  return phrase === null ? <Tag>제목 대기</Tag> : <span className="font-bold break-words">{phrase}</span>;
}

function RemovedRules({ channel }: { channel: ChannelView }) {
  const removed = channel.existing?.removed_rules ?? [];
  if (removed.length === 0) {
    return <p className="text-[13px] text-text-muted">교체해도 없어지는 규칙은 없어요.</p>;
  }
  return (
    <div className="rounded-lg border border-hairline bg-surface-2 p-3">
      <p className="mb-2 text-[13px] font-bold text-text-primary">교체하면 없어지는 규칙 {removed.length}개예요.</p>
      <ul aria-label="없어지는 규칙" className="m-0 flex list-none flex-col gap-1.5 p-0 text-[13.5px]">
        {removed.map((rule, i) => (
          <li key={i} className="flex flex-wrap items-baseline gap-x-2.5 gap-y-0.5">
            <RuleName phrase={rule.match} />
            <span className="text-xs text-text-muted">{rule.directory || "저장 폴더 없음"}</span>
          </li>
        ))}
      </ul>
    </div>
  );
}

function ChannelCard({ channel, flow }: { channel: ChannelView; flow: ImportFlow }) {
  const chosen = flow.choices[channel.index];
  const existing = channel.existing;

  return (
    <div className={cn(CARD, "overflow-hidden")}>
      <header className="flex flex-col gap-2 border-b border-hairline-soft px-4 py-3">
        <h4 className="m-0 text-sm font-bold break-all">{channel.url}</h4>
        <Facts>
          <Labeled label="저장 폴더">{channel.directory}</Labeled>
          <Labeled label="파일의 규칙">{channel.rules.length}개</Labeled>
          {existing ? <Labeled label="지금 규칙">{existing.rule_count}개</Labeled> : <Tag>새 채널</Tag>}
        </Facts>
        {channel.excludes.length > 0 && (
          <Facts>
            <span className="text-[13px] text-text-muted">제외 조건</span>
            {channel.excludes.map((text) => (
              <Tag key={text}>{text}</Tag>
            ))}
          </Facts>
        )}
      </header>
      <div className="flex flex-col gap-3 px-4 py-3.5">
        {existing ? (
          <>
            <fieldset className="m-0 min-w-0 border-0 p-0">
              <legend className="mb-2 p-0 text-[13px] font-bold text-text-primary">가져오는 방법</legend>
              <div className="grid grid-cols-3 gap-2.5 max-[900px]:grid-cols-1">
                {OPTIONS.map((option) => {
                  const selected = chosen === option.value;
                  return (
                    <label
                      key={option.value}
                      className={cn(
                        "flex cursor-pointer flex-col gap-1 rounded-lg border p-3 has-[:focus-visible]:outline-2 has-[:focus-visible]:outline-offset-2 has-[:focus-visible]:outline-focus",
                        selected ? "border-focus bg-surface-3" : "border-hairline hover:bg-surface-2",
                      )}
                    >
                      <span className="flex items-center gap-2">
                        <input
                          type="radio"
                          name={`choice-${channel.index}`}
                          value={option.value}
                          checked={selected}
                          onChange={() => flow.choose(channel.index, option.value)}
                          className="size-4 flex-none accent-(--focus-ring)"
                        />
                        <span className={cn("text-sm", selected ? "font-bold" : "font-medium")}>{option.label}</span>
                      </span>
                      <span className="text-[13px] leading-relaxed text-text-secondary">
                        {option.describe(channel)}
                      </span>
                    </label>
                  );
                })}
              </div>
            </fieldset>
            {chosen === "replace" && <RemovedRules channel={channel} />}
          </>
        ) : (
          <p className="text-[13px] leading-relaxed text-text-muted">
            이 채널과 규칙 {channel.rules.length}개를 파일의 순서 그대로 새로 추가해요.
          </p>
        )}
      </div>
    </div>
  );
}

function RuleRow({ rule, no, replacing }: { rule: RuleView; no: number; replacing: boolean }) {
  return (
    <li className="flex gap-3 border-t border-hairline-soft px-4 py-3 first:border-t-0">
      <span className="w-6 flex-none pt-0.5 text-right text-[13px] text-text-muted tabular-nums">{no}</span>
      <div className="flex min-w-0 flex-1 flex-col gap-1.5">
        <div className="text-sm">
          <RuleName phrase={rule.match} />
        </div>
        <Facts>
          <Labeled label="저장 폴더">{rule.directory || "채널 폴더"}</Labeled>
          <Labeled label="회차 변환">{signed(rule.episode)}</Labeled>
        </Facts>
        <Facts>
          {rule.regex && <Tag>정규식</Tag>}
          {rule.case_insensitive && <Tag>대소문자 무시</Tag>}
          {replacing && rule.keeps_existing_rule && <Tag icon={CheckIcon}>지금 규칙 유지</Tag>}
          {rule.invalid_regex && (
            <Tag icon={AlertIcon} tone="warn">
              정규식을 읽을 수 없음
            </Tag>
          )}
        </Facts>
        {rule.invalid_regex && (
          <p className="text-[13px] text-text-muted">정규식을 읽을 수 없어서 이 규칙은 어떤 제목에도 맞지 않아요.</p>
        )}
      </div>
    </li>
  );
}

function RulesFold({ channel, flow }: { channel: ChannelView; flow: ImportFlow }) {
  const replacing = flow.choices[channel.index] === "replace";
  const skipped = flow.choices[channel.index] === "skip";
  return (
    <details open className={cn(CARD, "group overflow-hidden", skipped && "opacity-70")}>
      <summary className="flex cursor-pointer list-none flex-wrap items-center gap-x-3 gap-y-1.5 px-4 py-3 [&::-webkit-details-marker]:hidden">
        <ChevronIcon className="size-4 flex-none text-text-muted transition-transform group-open:rotate-90" />
        <span className="min-w-0 flex-1 basis-56 text-sm font-bold break-all">{channel.url}</span>
        <Facts>
          <Labeled label="평가 순서">
            {channel.rules.length === 0 ? "규칙 없음" : `1번에서 ${channel.rules.length}번`}
          </Labeled>
          {skipped && <Tag tone="pending">건너뜀</Tag>}
        </Facts>
      </summary>
      {channel.rules.length > 0 && (
        <ol className="m-0 list-none border-t border-hairline-soft p-0">
          {channel.rules.map((rule, i) => (
            <RuleRow key={i} rule={rule} no={i + 1} replacing={replacing} />
          ))}
        </ol>
      )}
    </details>
  );
}

export function ReviewStep({ flow }: { flow: ImportFlow }) {
  const preview = flow.preview;
  if (!preview) return null;

  const ruleCount = preview.channels.reduce((sum, channel) => sum + channel.rules.length, 0);
  const conflicts = preview.conflict_count;
  const counts = { replace: 0, add: 0, skip: 0 };
  for (const decision of Object.values(flow.choices)) counts[decision] += 1;
  const fresh = preview.channels.length - conflicts;

  const summary =
    flow.undecided > 0 ? (
      <>이미 있는 채널마다 가져오는 방법을 선택해 주세요. 아직 {flow.undecided}개 채널이 남았어요.</>
    ) : (
      <Facts>
        {fresh > 0 && <Labeled label="새 채널">{fresh}개 추가</Labeled>}
        {counts.replace > 0 && <Labeled label="교체">{counts.replace}개 채널</Labeled>}
        {counts.add > 0 && <Labeled label="추가">{counts.add}개 채널</Labeled>}
        {counts.skip > 0 && <Labeled label="건너뛰기">{counts.skip}개 채널</Labeled>}
      </Facts>
    );

  return (
    <div className="flex flex-col gap-6">
      <Banner tone="calm" title="기존 YAML 파일이에요">
        채널 {preview.channels.length}개와 규칙 {ruleCount}개가 들어 있고, 채널 순서와 규칙 평가 순서는 파일 그대로
        가져와요.{" "}
        {conflicts > 0
          ? `지금 설정에 파일과 같은 주소의 채널이 ${conflicts}개 있어서, 그 채널마다 어떻게 가져올지 선택해 주세요. 나머지 채널은 묻지 않고 새로 추가해요.`
          : "지금 같은 주소의 채널이 없어서 파일의 채널을 모두 새로 추가해요."}{" "}
        채널 주소의 쿼리 값은 모두 비밀로 저장하고 화면에는 가려서 보여줘요.
      </Banner>

      {flow.applyError && (
        <Banner
          tone="fail"
          role="alert"
          title={flow.applyError.stale ? "검토한 내용이 바뀌었어요" : "가져오지 못했어요"}
          actions={
            flow.applyError.stale ? (
              <button type="button" className={BTN.plain} onClick={() => void flow.reviewAgain()}>
                다시 검토
              </button>
            ) : undefined
          }
        >
          {flow.applyError.message} 설정은 하나도 바뀌지 않았어요.
        </Banner>
      )}

      <section aria-labelledby="import-channels" className="flex flex-col gap-3">
        <h3 id="import-channels" className="m-0 text-base font-bold">
          채널
        </h3>
        <div className="flex flex-col gap-3">
          {preview.channels.map((channel) => (
            <ChannelCard key={channel.index} channel={channel} flow={flow} />
          ))}
        </div>
      </section>

      <ActionBar
        buttons={
          <>
            <button type="button" className={BTN.plain} onClick={flow.restart}>
              <BackIcon className="size-4" />
              이전
            </button>
            <button
              type="button"
              className={BTN.main}
              disabled={flow.undecided > 0 || flow.applying}
              onClick={() => void flow.apply()}
            >
              <CheckIcon className="size-4" />
              {flow.applying ? "가져오는 중" : "가져오기"}
            </button>
          </>
        }
      >
        <div aria-live="polite">{summary}</div>
      </ActionBar>

      <section aria-labelledby="import-rules" className="flex flex-col gap-3">
        <h3 id="import-rules" className="m-0 text-base font-bold">
          규칙
        </h3>
        <p className="text-[13.5px] leading-relaxed text-text-secondary">
          규칙은 위에서부터 순서대로 평가하고 처음 맞는 규칙이 적용돼요. 파일에 적힌 값이 그대로 저장돼요.
        </p>
        <div className="flex flex-col gap-3">
          {preview.channels.map((channel) => (
            <RulesFold key={channel.index} channel={channel} flow={flow} />
          ))}
        </div>
      </section>
    </div>
  );
}
