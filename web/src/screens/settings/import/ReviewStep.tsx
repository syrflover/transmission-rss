import { cn } from "@/lib/utils";

import { AlertIcon, BackIcon, CheckIcon, ChevronIcon } from "../icons";
import { ActionBar, Banner, BTN, CARD, Facts, Labeled, Tag } from "../parts";
import { caseCounts, standing, UNLISTED_NOTE, weekLabel } from "./suggestions";
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
  const waiting = channel.existing?.title_waiting_kept ?? 0;
  const keptNote = waiting > 0 && (
    <p className="text-[13px] text-text-secondary">제목 대기 구독 {waiting}개는 그대로 남겨요.</p>
  );
  if (removed.length === 0) {
    return (
      <>
        <p className="text-[13px] text-text-muted">교체해도 없어지는 규칙은 없어요.</p>
        {keptNote}
      </>
    );
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
      {keptNote && <div className="mt-2">{keptNote}</div>}
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
          <Labeled label="파일의 채널 폴더">{channel.directory}</Labeled>
          <Labeled label="파일의 규칙">{channel.rules.length}개</Labeled>
          {channel.not_imported !== null ? (
            <Tag icon={AlertIcon} tone="warn">
              가져오지 않음
            </Tag>
          ) : existing ? (
            <Labeled label="지금 규칙">{existing.rule_count}개</Labeled>
          ) : (
            <Tag>새 채널</Tag>
          )}
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
        {channel.not_imported !== null ? (
          <p className="text-[13px] leading-relaxed text-text-secondary">{channel.not_imported}</p>
        ) : existing ? (
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

/** What the comment above a rule offers, and the box that keeps or drops it. */
function SuggestionBlock({
  channel,
  rule,
  index,
  flow,
}: {
  channel: ChannelView;
  rule: RuleView;
  index: number;
  flow: ImportFlow;
}) {
  const suggestion = rule.suggestion;
  if (suggestion.kind === "none") {
    return (
      <p className="flex flex-wrap items-center gap-x-2 gap-y-1 text-[13px] text-text-muted">
        <Tag>주석 없음</Tag>
        규칙만 가져와요.
      </p>
    );
  }
  if (suggestion.kind === "unreadable") {
    return (
      <p className="flex flex-wrap items-center gap-x-2 gap-y-1 text-[13px] text-text-muted">
        <Tag icon={AlertIcon} tone="warn">
          주석을 읽을 수 없음
        </Tag>
        {suggestion.reason} 규칙만 가져와요.
      </p>
    );
  }

  const animeNo = suggestion.anime_no as number;
  const decision = flow.choices[channel.index];
  const state = standing(channel, rule, decision, flow.lookup.unlisted);
  const checked = flow.isPicked(channel.index, index);
  const found = flow.lookup.schedules[animeNo];
  const looking = flow.lookup.status === "loading";
  const unknown = looking ? "읽는 중" : "미정";
  const creators = flow.lookup.creators[animeNo];
  // An anime Anissia does not list has no caption list to compare the name with.
  const unlisted =
    state !== "unlisted" &&
    suggestion.creator !== null &&
    Array.isArray(creators) &&
    !creators.includes(suggestion.creator);
  const note =
    state === "skipped"
      ? "이 채널을 건너뛰어서 구독 제안도 함께 빠져요."
      : state === "not_imported"
        ? "이 채널은 가져오지 않아서 구독 제안도 빠져요."
        : state === "blocked"
          ? suggestion.blocked
          : state === "kept"
            ? "교체해도 이 규칙은 지금 구독 그대로 두고, 이 제안은 쓰지 않아요."
            : state === "unlisted"
              ? UNLISTED_NOTE
              : null;

  return (
    <div className="flex flex-col gap-1.5 rounded-lg border border-hairline bg-surface-2 p-3">
      <label
        className={cn(
          "flex items-start gap-2.5 text-sm has-[:focus-visible]:outline-2 has-[:focus-visible]:outline-offset-2 has-[:focus-visible]:outline-focus",
          state === "pickable" ? "cursor-pointer" : "cursor-not-allowed opacity-70",
        )}
      >
        <input
          type="checkbox"
          checked={checked}
          disabled={state !== "pickable"}
          onChange={(event) => flow.pick(channel.index, index, event.target.checked)}
          className="mt-0.5 size-4 flex-none accent-(--focus-ring)"
        />
        <span className="font-bold">구독으로 가져오기</span>
      </label>
      <Facts className="pl-[26px]">
        <Labeled label="Anissia 작품">
          {found ? found.subject : unknown} <span className="text-xs text-text-muted">#{animeNo}</span>
        </Labeled>
        <Labeled label="방영">
          {found ? (
            weekLabel(found.week, found.air_time)
          ) : suggestion.comment_airs !== null && !looking ? (
            <>
              {weekLabel(suggestion.comment_airs.week, suggestion.comment_airs.time)}{" "}
              <span className="text-xs text-text-muted">(주석에 적힌 값)</span>
            </>
          ) : (
            unknown
          )}
        </Labeled>
        {suggestion.creator !== null ? (
          <Labeled label="자막 제작자">{suggestion.creator}</Labeled>
        ) : (
          <Tag>제작자 미정</Tag>
        )}
        {unlisted && (
          <Tag icon={AlertIcon} tone="warn">
            자막 목록에 없는 제작자
          </Tag>
        )}
      </Facts>
      {unlisted && state === "pickable" && (
        <p className="pl-[26px] text-[13px] leading-relaxed text-text-muted">
          이 작품의 Anissia 자막 목록에 없는 이름이에요. 체크하면 적힌 이름 그대로 따라가는 구독으로 가져와요.
        </p>
      )}
      {suggestion.creator === null && state === "pickable" && (
        <p className="pl-[26px] text-[13px] leading-relaxed text-text-muted">
          제작자를 나중에 정해야 해서 처음에는 체크하지 않았어요. 체크하면 제작자 미정 구독으로 가져와요.
        </p>
      )}
      {note && <p className="pl-[26px] text-[13px] leading-relaxed text-text-muted">{note}</p>}
    </div>
  );
}

function RuleRow({
  rule,
  no,
  replacing,
  channel,
  flow,
}: {
  rule: RuleView;
  no: number;
  replacing: boolean;
  channel: ChannelView;
  flow: ImportFlow;
}) {
  return (
    <li className="flex gap-3 border-t border-hairline-soft px-4 py-3 first:border-t-0">
      <span className="w-6 flex-none pt-0.5 text-right text-[13px] text-text-muted tabular-nums">{no}</span>
      <div className="flex min-w-0 flex-1 flex-col gap-1.5">
        <div className="text-sm">
          <RuleName phrase={rule.match} />
        </div>
        <Facts>
          <Labeled label="저장 폴더">{rule.directory || "수집 폴더"}</Labeled>
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
        <SuggestionBlock channel={channel} rule={rule} index={no - 1} flow={flow} />
      </div>
    </li>
  );
}

/** The four cases of a channel's comments as count chips, and the keep-all and drop-all buttons. */
function SuggestionToolbar({ channel, flow }: { channel: ChannelView; flow: ImportFlow }) {
  const counts = caseCounts(channel);
  const decision = flow.choices[channel.index];
  const pickable = channel.rules.filter(
    (rule) => standing(channel, rule, decision, flow.lookup.unlisted) === "pickable",
  ).length;
  const picked = channel.rules.filter((_, index) => flow.isPicked(channel.index, index)).length;
  return (
    <div className="flex flex-wrap items-center gap-x-4 gap-y-2.5 border-t border-hairline-soft px-4 py-3">
      <Facts className="min-w-0 flex-1 basis-72">
        <span className="text-[13px] text-text-muted">규칙 위 주석</span>
        <Tag>주소와 제작자 {counts.with_creator}개</Tag>
        <Tag>제작자 미정 {counts.address_only}개</Tag>
        <Tag>읽을 수 없음 {counts.unreadable}개</Tag>
        <Tag>주석 없음 {counts.none}개</Tag>
      </Facts>
      <div className="flex min-w-0 flex-wrap items-center gap-2">
        <span className="text-[13px] text-text-secondary" aria-live="polite">
          구독으로 가져올 규칙 {picked}개
        </span>
        <button
          type="button"
          className={BTN.plain}
          disabled={pickable === 0 || picked === pickable}
          aria-label={`구독 제안 모두 선택 (${channel.url})`}
          onClick={() => flow.pickAll(channel.index, true)}
        >
          모두 선택
        </button>
        <button
          type="button"
          className={BTN.plain}
          disabled={picked === 0}
          aria-label={`구독 제안 모두 해제 (${channel.url})`}
          onClick={() => flow.pickAll(channel.index, false)}
        >
          모두 해제
        </button>
      </div>
    </div>
  );
}

function RulesFold({ channel, flow }: { channel: ChannelView; flow: ImportFlow }) {
  const replacing = flow.choices[channel.index] === "replace";
  const notImported = channel.not_imported !== null;
  const skipped = flow.choices[channel.index] === "skip" || notImported;
  return (
    <details open className={cn(CARD, "group overflow-hidden", skipped && "opacity-70")}>
      <summary className="flex cursor-pointer list-none flex-wrap items-center gap-x-3 gap-y-1.5 px-4 py-3 [&::-webkit-details-marker]:hidden">
        <ChevronIcon className="size-4 flex-none text-text-muted transition-transform group-open:rotate-90" />
        <span className="min-w-0 flex-1 basis-56 text-sm font-bold break-all">{channel.url}</span>
        <Facts>
          <Labeled label="평가 순서">
            {channel.rules.length === 0 ? "규칙 없음" : `1번에서 ${channel.rules.length}번`}
          </Labeled>
          {notImported ? <Tag tone="warn">가져오지 않음</Tag> : skipped && <Tag tone="pending">건너뜀</Tag>}
        </Facts>
      </summary>
      {channel.rules.length > 0 && <SuggestionToolbar channel={channel} flow={flow} />}
      {channel.rules.length > 0 && (
        <ol className="m-0 list-none border-t border-hairline-soft p-0">
          {channel.rules.map((rule, i) => (
            <RuleRow key={i} rule={rule} no={i + 1} replacing={replacing} channel={channel} flow={flow} />
          ))}
        </ol>
      )}
    </details>
  );
}

/** Where the background reads of Anissia stand, and what a failure leaves undecided. */
function LookupNote({ flow }: { flow: ImportFlow }) {
  const { status, problem } = flow.lookup;
  if (problem !== null) {
    return (
      <Banner tone="fail" role="status" title="Anissia에서 읽지 못한 값이 있어요">
        {problem} 읽지 못한 방영 요일·시간은 주석에 적힌 값(없으면 미정)으로 두고, 구독은 그대로 가져올 수 있어요. 체크한 구독의 요일과 시간은
        앱이 나중에 Anissia에서 다시 읽어 채워요.
      </Banner>
    );
  }
  if (status === "loading") {
    return (
      <p role="status" className="text-[13px] text-text-muted">
        Anissia에서 방영 요일·시간과 자막 제작자를 읽는 중이에요. 기다리지 않고 가져와도 돼요.
      </p>
    );
  }
  return null;
}

export function ReviewStep({ flow }: { flow: ImportFlow }) {
  const preview = flow.preview;
  if (!preview) return null;

  const ruleCount = preview.channels.reduce((sum, channel) => sum + channel.rules.length, 0);
  const conflicts = preview.conflict_count;
  const outside = preview.channels.filter((channel) => channel.not_imported !== null).length;
  const { current: currentFolder, will_set: folderToSet } = preview.collect_folder;
  const counts = { replace: 0, add: 0, skip: 0 };
  for (const decision of Object.values(flow.choices)) counts[decision] += 1;
  const fresh = preview.channels.length - conflicts - outside;

  const summary =
    flow.undecided > 0 ? (
      <>이미 있는 채널마다 가져오는 방법을 선택해 주세요. 아직 {flow.undecided}개 채널이 남았어요.</>
    ) : (
      <Facts>
        {fresh > 0 && <Labeled label="새 채널">{fresh}개 추가</Labeled>}
        {counts.replace > 0 && <Labeled label="교체">{counts.replace}개 채널</Labeled>}
        {counts.add > 0 && <Labeled label="추가">{counts.add}개 채널</Labeled>}
        {counts.skip > 0 && <Labeled label="건너뛰기">{counts.skip}개 채널</Labeled>}
        <Labeled label="구독으로 가져올 규칙">{flow.pickedCount}개</Labeled>
      </Facts>
    );

  return (
    <div className="flex flex-col gap-6">
      <Banner tone="calm" title="기존 YAML 파일이에요">
        채널 {preview.channels.length}개와 규칙 {ruleCount}개가 들어 있고, 채널 순서와 규칙 평가 순서는 파일 그대로
        가져와요.{" "}
        {conflicts > 0
          ? `지금 설정에 파일과 같은 주소의 채널이 ${conflicts}개 있어서, 그 채널마다 어떻게 가져올지 선택해 주세요. 나머지 채널은 묻지 않고 새로 추가해요.`
          : outside > 0
            ? "지금 같은 주소의 채널이 없어서 가져올 채널은 모두 새로 추가해요."
            : "지금 같은 주소의 채널이 없어서 파일의 채널을 모두 새로 추가해요."}{" "}
        채널 주소의 쿼리 값은 모두 비밀로 저장하고 화면에는 가려서 보여줘요.
      </Banner>

      {folderToSet !== null && (
        <Banner tone="calm" title="수집 폴더를 정해요">
          아직 수집 폴더가 없어서 가져오면서 <b className="font-mono break-all text-text-primary">{folderToSet}</b>로
          정해요. 가져온 규칙의 저장 폴더는 이 폴더 아래 경로로 바뀌어 저장되고, 받는 위치는 파일에서와 같아요.
        </Banner>
      )}
      {currentFolder !== null && outside === 0 && preview.channels.length > 0 && (
        <p className="text-[13px] leading-relaxed text-text-secondary">
          수집 폴더는 <b className="font-mono break-all text-text-primary">{currentFolder}</b>예요. 그 안에 있는 채널
          폴더는 규칙의 저장 폴더 앞에 남은 부분을 붙여 가져와요.
        </p>
      )}
      {outside > 0 && (
        <Banner tone="fail" role="status" title={`수집 폴더 밖의 채널 ${outside}개는 가져오지 않아요`}>
          {currentFolder !== null ? (
            <>
              수집 폴더는 <b className="font-mono break-all text-text-primary">{currentFolder}</b>예요.{" "}
            </>
          ) : null}
          채널의 까닭은 아래 채널 카드에 있어요. 나머지 채널은 그대로 가져와요.
        </Banner>
      )}

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
        <p className="text-[13.5px] leading-relaxed text-text-secondary">
          규칙 위 주석에서 Anissia 작품 주소와 자막 제작자를 읽어 구독 제안을 만들었어요. 주석은 사람이 쓴 형식이라
          틀릴 수 있어서, 체크한 제안만 구독으로 가져오고 나머지는 규칙만 가져와요. 방영 요일과 시간은 주석이 아니라
          Anissia에서 읽어 보여주고, Anissia에 닿지 못할 때만 주석에 적힌 값을 그렇다고 밝혀서 보여줘요.
        </p>
        <LookupNote flow={flow} />
        <div className="flex flex-col gap-3">
          {preview.channels.map((channel) => (
            <RulesFold key={channel.index} channel={channel} flow={flow} />
          ))}
        </div>
      </section>
    </div>
  );
}
