import { useId, useMemo, useRef, useState } from "react";
import { Link, useSearchParams } from "react-router-dom";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { ApiError } from "@/lib/api";
import { useAfterDelay, useCached } from "@/lib/cached";
import { cn } from "@/lib/utils";

import { candidatesChanged, KEYS } from "../../cache";
import { btnNeutral, btnPrimary, hintClass, inputClass, labelClass } from "../../channels/styles";
import { channelName, listRules, type Rule, type RuleFields, type RuleList } from "../../rules/api";
import { usePreview } from "../../rules/usePreview";
import { nameTitle } from "../api";
import { airing } from "../format";
import { listed, PastChecklist, ReceiveProgress, receivable, receiveOrder } from "../add/PastItems";
import { folderProblem, optionClass } from "../add/draft";
import { useReceive } from "../add/useReceive";

/**
 * Where a title candidate (or a `no_match` row of the record tab) is given to
 * a waiting subscription: `/collect/subs/title?channel=<id>&work=<phrase>` and,
 * when the work has one, `&folder=<suggested folder>`.
 *
 * The subscription is picked here, never assumed: the link between an anime
 * and a work is only a suggestion. Naming the title makes the work the rule's
 * match phrase; the items already recorded for it are the rule's past items,
 * and only the ticked ones are received.
 */
export function NameTitle() {
  const [params] = useSearchParams();
  const channelId = params.get("channel") ?? "";
  const work = params.get("work") ?? "";
  const folder = params.get("folder");
  const rules = useCached<RuleList>(KEYS.rules, listRules, "구독을 불러오지 못했어요.");

  const list = rules.data;
  const channel = list?.channels.find((c) => c.id === channelId) ?? null;
  const waiting = useMemo(
    () =>
      list
        ? list.rules
            .filter((r) => r.channel_id === channelId && r.match === null && r.state === "active" && r.subscription)
            .sort((a, b) => a.order - b.order)
        : [],
    [list, channelId],
  );
  const siblings = useMemo(
    () => (list ? list.rules.filter((r) => r.channel_id === channelId).sort((a, b) => a.order - b.order) : []),
    [list, channelId],
  );

  // Naming the title changes the rule list the choices come from, and the
  // subscription just named is no longer waiting. Once the title is named the
  // page keeps showing the result of that, from the choices it had.
  const [locked, setLocked] = useState(false);
  const frozen = useRef<{ label: string; waiting: Rule[]; siblings: Rule[] } | null>(null);
  if (!locked && channel !== null && waiting.length > 0) {
    frozen.current = { label: channelName(channel), waiting, siblings };
  }
  const naming = locked ? frozen.current : channel !== null && waiting.length > 0 ? frozen.current : null;

  return (
    <div className="flex min-w-0 flex-col gap-4">
      <div className="flex flex-wrap items-center justify-between gap-x-4 gap-y-2.5">
        <h2 className="min-w-0 text-[17px] font-bold break-words">제목 정하기</h2>
        <Button asChild type="button" variant="ghost" className={btnNeutral}>
          <Link to="/collect/subs">구독 목록</Link>
        </Button>
      </div>

      {(channelId === "" || work === "") && (
        <p role="alert" className="text-[13px] font-semibold text-urgent">
          어떤 제목인지 알 수 없는 주소예요. 구독 탭의 제목 후보에서 다시 열어 주세요.
        </p>
      )}

      {list === undefined && rules.error !== null && (
        <div className="flex flex-col items-start gap-2.5">
          <p role="alert" className="text-[13px] font-semibold text-urgent">
            {rules.error}
          </p>
          <Button type="button" variant="ghost" className={btnNeutral} onClick={rules.reload}>
            재시도
          </Button>
        </div>
      )}
      {list === undefined && rules.error === null && rules.slow && (
        <p className="text-[13px] text-text-muted">구독을 불러오는 중이에요.</p>
      )}

      {naming === null && list !== undefined && channelId !== "" && work !== "" && channel === null && (
        <p role="status" className="text-[13px] leading-normal font-semibold text-text-secondary">
          이 채널은 이제 없어요.
        </p>
      )}

      {naming === null && list !== undefined && channel !== null && work !== "" && waiting.length === 0 && (
        <p role="status" className="text-[13px] leading-normal font-semibold text-text-secondary">
          {channelName(channel)} 채널에는 제목을 기다리는 구독이 없어요. 이미 제목을 정했거나 구독을 멈췄는지 구독 목록에서 확인해 주세요.
        </p>
      )}

      {naming !== null && work !== "" && (
        <Naming
          key={`${channelId}\n${work}`}
          channelId={channelId}
          channelLabel={naming.label}
          work={work}
          suggested={folder}
          waiting={naming.waiting}
          siblings={naming.siblings}
          onNamed={() => setLocked(true)}
        />
      )}
    </div>
  );
}

function Naming({
  channelId,
  channelLabel,
  work,
  suggested,
  waiting,
  siblings,
  onNamed,
}: {
  channelId: string;
  channelLabel: string;
  work: string;
  suggested: string | null;
  waiting: Rule[];
  siblings: Rule[];
  /** Called when the title was named, before the lists it changes are read again. */
  onNamed: () => void;
}) {
  const uid = useId();
  const [pickedId, setPickedId] = useState<string | null>(waiting.length === 1 ? waiting[0].id : null);
  const picked = waiting.find((r) => r.id === pickedId) ?? null;
  // Keep the folder the subscription has, or replace it with the one made from the title.
  const [replace, setReplace] = useState(suggested !== null);
  const [folderText, setFolderText] = useState(suggested ?? "");
  const folderBad = replace ? folderProblem(folderText) : null;
  const directory = picked === null ? "" : replace ? folderText.trim() : picked.directory;

  const fields = useMemo<RuleFields>(
    () => ({
      match: work,
      regex: picked?.regex ?? false,
      case_insensitive: picked?.case_insensitive ?? false,
      directory,
      episode: picked?.episode ?? 1,
      state: "active",
    }),
    [work, picked, directory],
  );
  const position = picked ? Math.max(0, siblings.findIndex((r) => r.id === picked.id)) : 0;
  // Nothing is asked until a subscription is picked: the preview is that rule's.
  const preview = usePreview(channelId, picked?.id ?? null, fields, position, 0, picked !== null && folderBad === null);
  const shown =
    picked === null ? null : preview.state === "ready" ? preview.preview : preview.state === "loading" ? preview.previous : null;
  const slow = useAfterDelay(picked !== null && shown === null && preview.state === "loading");

  const [ticked, setTicked] = useState<ReadonlySet<number>>(new Set());
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [done, setDone] = useState<Rule | null>(null);
  const [titles, setTitles] = useState<ReadonlyMap<number, string>>(new Map());
  const receive = useReceive(done?.id ?? null);

  const items = shown ? listed(shown.items) : [];
  const tickedNow = items.filter((i) => receivable(i) && ticked.has(i.id));

  const submit = async () => {
    if (picked === null) return;
    setBusy(true);
    setError(null);
    try {
      const rule = await nameTitle(
        { rule_id: picked.id, rule_version: picked.version },
        work,
        replace ? folderText.trim() : null,
      );
      onNamed();
      candidatesChanged();
      setTitles(new Map(tickedNow.map((i) => [i.id, i.title])));
      setDone(rule);
      if (tickedNow.length > 0) receive.start(rule.id, receiveOrder(tickedNow).map((i) => i.id));
    } catch (e) {
      setError(e instanceof ApiError ? e.message : "제목을 정하지 못했어요. 다시 시도해 주세요.");
    } finally {
      setBusy(false);
    }
  };

  if (done !== null) {
    const subject = done.subscription?.anime?.subject ?? done.directory;
    return (
      <section aria-labelledby={`${uid}-done`} className="flex min-w-0 flex-col gap-3.5">
        <h3 id={`${uid}-done`} className="text-[15px] font-bold">
          제목을 정했어요
        </h3>
        <p className="min-w-0 text-[13px] leading-normal break-words text-text-secondary">
          {subject}의 일치 문구는 ‘{done.match}’이에요. 이 제목의 새 회차는 다음 RSS 확인부터 {channelLabel} 채널에서 받아요.
          {receive.entries.length === 0 && " 지난 항목은 받지 않았어요. 규칙 화면에서 골라 받을 수 있어요."}
        </p>
        <ReceiveProgress entries={receive.entries} titleOf={(id) => titles.get(id) ?? `항목 ${id}`} onRetry={receive.retry} />
        <div className="flex flex-wrap gap-2.5">
          <Button asChild type="button" variant="ghost" className={btnPrimary}>
            <Link to={`/collect/rules?rule=${encodeURIComponent(done.id)}`}>규칙 열기</Link>
          </Button>
          <Button asChild type="button" variant="ghost" className={btnNeutral}>
            <Link to="/collect/subs">구독 목록</Link>
          </Button>
        </div>
      </section>
    );
  }

  return (
    <div className="flex min-w-0 flex-col gap-5">
      <div className="min-w-0 rounded-card border border-hairline-soft bg-surface-1 p-[18px] shadow-(--card-shadow) max-[480px]:p-4">
        <dl className="m-0 grid grid-cols-[84px_minmax(0,1fr)] gap-x-3 gap-y-1.5 text-[13px]">
          <dt className="font-semibold text-text-muted">제목</dt>
          <dd className="m-0 min-w-0 font-bold break-all">{work}</dd>
          <dt className="font-semibold text-text-muted">채널</dt>
          <dd className="m-0 min-w-0 break-all">{channelLabel}</dd>
        </dl>
      </div>

      <section aria-labelledby={`${uid}-pick`} className="flex min-w-0 flex-col gap-2.5">
        <h3 id={`${uid}-pick`} className="text-[15px] font-bold">
          어느 구독의 제목인가요
        </h3>
        <p className={hintClass}>
          작품과 제목이 이어지는지는 알 수 없어서 직접 골라요. 고른 구독의 일치 문구가 이 제목이 돼요.
        </p>
        <ul className="m-0 flex list-none flex-col gap-2 p-0">
          {waiting.map((rule) => {
            const on = rule.id === pickedId;
            const anime = rule.subscription?.anime ?? null;
            return (
              <li key={rule.id}>
                <button
                  type="button"
                  aria-pressed={on}
                  onClick={() => {
                    setPickedId(rule.id);
                    setTicked(new Set());
                  }}
                  className={cn(optionClass, on ? "border-focus" : "border-hairline-soft")}
                >
                  <span className="min-w-0 text-[14.5px] leading-snug font-semibold break-words">
                    {anime?.subject ?? "이름 없는 작품"}
                  </span>
                  {anime && <span className="text-xs text-text-secondary">{airing(anime)}</span>}
                  <span className="min-w-0 font-mono text-xs break-all text-text-muted">{rule.directory}</span>
                </button>
              </li>
            );
          })}
        </ul>
      </section>

      {picked !== null && (
        <section aria-labelledby={`${uid}-folder`} className="flex min-w-0 flex-col gap-2.5">
          <h3 id={`${uid}-folder`} className="text-[15px] font-bold">
            저장 폴더
          </h3>
          <div role="radiogroup" aria-labelledby={`${uid}-folder`} className="flex flex-col gap-2">
            <label
              className={cn(
                optionClass,
                "cursor-pointer",
                !replace ? "border-focus" : "border-hairline-soft",
                suggested === null && "cursor-default",
              )}
            >
              <span className="flex items-start gap-2.5">
                <input
                  type="radio"
                  name={`${uid}-folder`}
                  checked={!replace}
                  onChange={() => setReplace(false)}
                  className="mt-0.5 size-[16px] flex-none accent-focus"
                />
                <span className="flex min-w-0 flex-col gap-0.5">
                  <span className="text-[13.5px] font-semibold">지금 폴더를 그대로 써요</span>
                  <span className="min-w-0 font-mono text-xs break-all text-text-secondary">{picked.directory}</span>
                </span>
              </span>
            </label>
            {suggested !== null && (
              <label className={cn(optionClass, "cursor-pointer", replace ? "border-focus" : "border-hairline-soft")}>
                <span className="flex items-start gap-2.5">
                  <input
                    type="radio"
                    name={`${uid}-folder`}
                    checked={replace}
                    onChange={() => setReplace(true)}
                    className="mt-0.5 size-[16px] flex-none accent-focus"
                  />
                  <span className="flex min-w-0 flex-col gap-0.5">
                    <span className="text-[13.5px] font-semibold">이 제목으로 만든 폴더로 바꿔요</span>
                    <span className="text-xs text-text-secondary">제목에서 만든 제안이에요. 고쳐도 돼요.</span>
                  </span>
                </span>
              </label>
            )}
          </div>
          {replace && (
            <div className="flex min-w-0 flex-col gap-1.5">
              <Label htmlFor={`${uid}-dir`} className={labelClass}>
                새 저장 폴더
              </Label>
              <Input
                id={`${uid}-dir`}
                value={folderText}
                onChange={(e) => setFolderText(e.target.value)}
                spellCheck={false}
                autoComplete="off"
                aria-invalid={folderBad !== null}
                className={`${inputClass} font-mono`}
              />
              {folderBad !== null && <p className={`${hintClass} font-semibold text-urgent`}>{folderBad}</p>}
            </div>
          )}
        </section>
      )}

      {picked !== null && (
        <section aria-labelledby={`${uid}-past`} className="flex min-w-0 flex-col gap-2.5">
          <h3 id={`${uid}-past`} className="text-[15px] font-bold">
            이미 기록된 항목
          </h3>
          <p className={hintClass}>
            제목을 정하는 것만으로는 아무것도 받지 않아요. 지난 항목은 체크한 것만 받아요. ‘[Batch]’처럼 원하지 않는 항목은 체크하지 않으면
            돼요.
          </p>
          {preview.state === "failed" && (
            <p role="alert" className="text-[13px] font-semibold text-urgent">
              {preview.message}
            </p>
          )}
          {shown?.error && (
            <p role="alert" className="text-[13px] font-semibold text-urgent">
              {shown.error.message}
            </p>
          )}
          {slow && <p className="text-[13px] text-text-muted">기록을 확인하는 중이에요.</p>}
          {shown && !shown.error && items.length === 0 && (
            <p className="text-[13px] leading-normal text-text-secondary">
              이 제목의 지난 항목은 받을 게 없어요. 제목을 정하면 다음 RSS 확인부터 새 회차를 받아요.
            </p>
          )}
          <PastChecklist items={items} ticked={ticked} onTicked={setTicked} label="이미 기록된 항목" />
          {shown?.truncated && <p className={hintClass}>항목이 많아서 최근 {shown.items.length}개만 보여줘요.</p>}
        </section>
      )}

      {error && (
        <p role="alert" className="text-[13px] leading-normal font-semibold text-urgent">
          {error}
        </p>
      )}

      <div className="flex">
        <Button
          type="button"
          variant="ghost"
          className={btnPrimary}
          disabled={busy || picked === null || folderBad !== null || shown?.error != null}
          onClick={() => void submit()}
        >
          {busy ? "제목을 정하는 중" : tickedNow.length > 0 ? `제목 정하고 ${tickedNow.length}개 받기` : "제목 정하기"}
        </Button>
      </div>
    </div>
  );
}
