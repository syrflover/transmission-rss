import { useEffect, useId, useRef, useState, type FormEvent } from "react";
import { Link } from "react-router-dom";

import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { ApiError } from "@/lib/api";
import { useCached } from "@/lib/cached";
import { dateTime } from "@/lib/time";
import { hintClass, inputClass, labelClass } from "@/screens/collect/channels/styles";

import { BTN } from "../parts";
import {
  FORMAT_LABEL,
  formatOrderText,
  idleMinuteRange,
  idleMinutesText,
  idleText,
  inRange,
  isPolicy,
  loadPolicy,
  POLICY_KEY,
  savePolicy,
  type Policy,
  type SubtitleFormat,
} from "./api";

const LOAD_FAILED = "공통 정책을 불러오지 못했어요.";

/** `자막 형식·브라우저`: the common policy, saved as one version. */
export function PolicyPanel() {
  const policy = useCached<Policy>(POLICY_KEY, loadPolicy, LOAD_FAILED);
  const data = policy.data;

  return (
    <div className="flex flex-col gap-5 p-5 max-[720px]:p-4">
      <header className="flex flex-col gap-1.5">
        <h2 id="panel-title" tabIndex={-1} className="m-0 text-xl font-bold outline-none">
          자막 형식·브라우저
        </h2>
        <p className="max-w-[62ch] text-[13.5px] leading-relaxed text-text-secondary">
          자막을 고르는 형식의 우선순위와 서버 브라우저의 유휴 시간, 동시 작업 수를 정해요. 세 값은 한 번에 저장해요.
        </p>
      </header>

      {data === undefined && policy.error !== null && (
        <div className="flex flex-col items-start gap-2.5">
          <p role="alert" className="text-[13px] font-semibold text-urgent">
            {policy.error}
          </p>
          <button type="button" className={BTN.plain} onClick={policy.reload}>
            재시도
          </button>
        </div>
      )}
      {data === undefined && policy.error === null && policy.slow && (
        <p className="text-[13px] text-text-muted">불러오는 중이에요.</p>
      )}
      {data !== undefined && <PolicyForm stored={data} onSaved={policy.update} />}
    </div>
  );
}

/** The input values: the order, and the two numbers as typed. */
interface Draft {
  order: SubtitleFormat[];
  idle: string;
  jobs: string;
}

function draftOf(policy: Policy): Draft {
  return {
    order: policy.format_order,
    idle: idleMinutesText(policy.idle_timeout_seconds),
    jobs: String(policy.max_concurrent_jobs),
  };
}

function PolicyForm({ stored, onSaved }: { stored: Policy; onSaved: (next: Policy) => void }) {
  const uid = useId();
  const [draft, setDraft] = useState(() => draftOf(stored));
  // What the server last told us; a save is sent against its version.
  const [known, setKnown] = useState(stored);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [conflict, setConflict] = useState<Policy | null>(null);
  const [saved, setSaved] = useState(false);
  const orderRef = useRef<HTMLOListElement>(null);
  // The move button to keep focus on after the list reorders.
  const [focus, setFocus] = useState<{ format: SubtitleFormat; down: boolean } | null>(null);

  const base = draftOf(known);
  const idleRange = idleMinuteRange(known.limits);
  const jobsRange = known.limits.max_concurrent_jobs;
  const idleMinutes = inRange(draft.idle, idleRange);
  const jobs = inRange(draft.jobs, jobsRange);
  const idleMessage = `브라우저 유휴 시간은 ${idleRange.min}분부터 ${idleRange.max}분까지 정할 수 있어요.`;
  const jobsMessage = `동시 작업 수는 ${jobsRange.min}개부터 ${jobsRange.max}개까지 정할 수 있어요.`;
  const idleChanged = draft.idle !== base.idle;
  const unchanged =
    !idleChanged &&
    draft.jobs === base.jobs &&
    draft.order.join() === base.order.join();
  // A stored time that is not a whole minute is shown rounded, and kept until the minutes are typed.
  const idleSeconds = idleChanged ? (idleMinutes ?? 0) * 60 : known.idle_timeout_seconds;
  const valid = idleMinutes !== null && jobs !== null;

  // A newer answer for a form nobody has touched replaces what it shows.
  const pristine = useRef(false);
  pristine.current = unchanged && !busy && conflict === null;
  useEffect(() => {
    if (!pristine.current || stored.version === known.version) return;
    setKnown(stored);
    setDraft(draftOf(stored));
  }, [stored, known.version]);

  useEffect(() => {
    if (focus === null) return;
    const buttons = orderRef.current?.querySelectorAll<HTMLButtonElement>(`[data-format="${focus.format}"]`);
    const [up, down] = [buttons?.[0], buttons?.[1]];
    const wanted = focus.down ? down : up;
    (wanted && !wanted.disabled ? wanted : focus.down ? up : down)?.focus();
    setFocus(null);
  }, [focus]);

  const move = (index: number, by: -1 | 1) => {
    const order = [...draft.order];
    [order[index], order[index + by]] = [order[index + by], order[index]];
    setDraft({ ...draft, order });
    setFocus({ format: draft.order[index], down: by === 1 });
    setSaved(false);
  };

  const edit = (next: Partial<Draft>) => {
    setDraft({ ...draft, ...next });
    setSaved(false);
  };

  const save = async () => {
    if (busy || !valid) return;
    setBusy(true);
    setError(null);
    setSaved(false);
    try {
      const next = await savePolicy(known.version, {
        format_order: draft.order,
        idle_timeout_seconds: idleSeconds,
        max_concurrent_jobs: jobs,
      });
      setKnown(next);
      setConflict(null);
      setDraft(draftOf(next));
      onSaved(next);
      setSaved(true);
    } catch (e) {
      if (e instanceof ApiError && e.code === "conflict") {
        // The answer carries the stored policy: keep the input, show both, and wait for a choice.
        try {
          const current = isPolicy(e.current) ? e.current : await loadPolicy();
          setKnown(current);
          setConflict(current);
          onSaved(current);
        } catch {
          setError(e.message);
        }
      } else {
        setConflict(null);
        setError(e instanceof ApiError ? e.message : "저장하지 못했어요. 잠시 뒤 다시 시도해 주세요.");
      }
    } finally {
      setBusy(false);
    }
  };

  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (conflict === null && !unchanged) void save();
  };

  const useServer = () => {
    setDraft(draftOf(known));
    setError(null);
    setConflict(null);
  };

  return (
    <form onSubmit={submit} className="flex flex-col gap-5" noValidate>
      <fieldset className="m-0 flex min-w-0 flex-col gap-2 border-0 p-0">
        <legend className={`${labelClass} mb-1.5 p-0`}>자막 형식 우선순위</legend>
        <ol ref={orderRef} className="m-0 flex list-none flex-col gap-2 p-0">
          {draft.order.map((format, index) => (
            <li
              key={format}
              className="flex items-center gap-3 rounded-xl border border-hairline bg-surface-2 px-3.5 py-2"
            >
              <span className="grid size-6 flex-none place-items-center rounded-full bg-surface-3 text-xs font-bold text-text-secondary">
                {index + 1}
              </span>
              <span className="min-w-0 flex-1 text-sm font-bold">{FORMAT_LABEL[format]}</span>
              <button
                type="button"
                data-format={format}
                className={BTN.plain}
                aria-label={`${FORMAT_LABEL[format]} 위로`}
                disabled={index === 0}
                onClick={() => move(index, -1)}
              >
                위로
              </button>
              <button
                type="button"
                data-format={format}
                className={BTN.plain}
                aria-label={`${FORMAT_LABEL[format]} 아래로`}
                disabled={index === draft.order.length - 1}
                onClick={() => move(index, 1)}
              >
                아래로
              </button>
            </li>
          ))}
        </ol>
        <p className={hintClass}>위에 있는 형식부터 골라요. 같은 형식을 두 번 둘 수는 없어요.</p>
      </fieldset>

      <div className="flex min-w-0 flex-col gap-1.5">
        <Label htmlFor={`${uid}-idle`} className={labelClass}>
          브라우저 유휴 시간
        </Label>
        <div className="flex items-center gap-2">
          <Input
            id={`${uid}-idle`}
            type="number"
            inputMode="numeric"
            min={idleRange.min}
            max={idleRange.max}
            step={1}
            value={draft.idle}
            onChange={(e) => edit({ idle: e.target.value })}
            className={`${inputClass} w-28`}
            aria-invalid={idleMinutes === null}
            aria-describedby={`${uid}-idle-hint`}
          />
          <span className="text-sm text-text-secondary">분</span>
        </div>
        {idleMinutes === null && (
          <p role="alert" className="text-[13px] font-semibold text-urgent">
            {idleMessage}
          </p>
        )}
        <p id={`${uid}-idle-hint`} className={hintClass}>
          유휴 시간이 지나면 원격 브라우저는 닫혀요. 인증이 필요한 작업은 그대로 남고, 작업을 열면 브라우저가 다시
          준비돼요.
          {known.idle_timeout_seconds % 60 !== 0 &&
            !idleChanged &&
            ` 저장된 값은 ${idleText(known.idle_timeout_seconds)}이고, 바꾸지 않으면 그대로 둬요.`}
        </p>
      </div>

      <div className="flex min-w-0 flex-col gap-1.5">
        <Label htmlFor={`${uid}-jobs`} className={labelClass}>
          동시 작업 수
        </Label>
        <div className="flex items-center gap-2">
          <Input
            id={`${uid}-jobs`}
            type="number"
            inputMode="numeric"
            min={jobsRange.min}
            max={jobsRange.max}
            step={1}
            value={draft.jobs}
            onChange={(e) => edit({ jobs: e.target.value })}
            className={`${inputClass} w-28`}
            aria-invalid={jobs === null}
            aria-describedby={`${uid}-jobs-hint`}
          />
          <span className="text-sm text-text-secondary">개</span>
        </div>
        {jobs === null && (
          <p role="alert" className="text-[13px] font-semibold text-urgent">
            {jobsMessage}
          </p>
        )}
        <p id={`${uid}-jobs-hint`} className={hintClass}>
          서버 브라우저로 한꺼번에 처리하는 작업의 수예요.
        </p>
      </div>

      {conflict && (
        <section role="alert" className="flex flex-col gap-3 rounded-xl border border-hairline bg-surface-2 p-3.5">
          <p className="text-sm font-bold">다른 곳에서 먼저 저장했어요.</p>
          <p className="text-[13px] leading-normal text-text-secondary">
            입력한 값은 그대로 두었어요. 서버 값을 쓰거나, 내 값을 새 버전으로 다시 저장할 수 있어요.
          </p>
          <div className="grid grid-cols-2 gap-x-3.5">
            <Compare
              title="내 값"
              order={formatOrderText(draft.order)}
              idle={idleMinutes === null ? "올바르지 않음" : idleText(idleSeconds)}
              jobs={jobs === null ? "올바르지 않음" : `${jobs}개`}
            />
            <Compare
              title="서버 값"
              order={formatOrderText(conflict.format_order)}
              idle={idleText(conflict.idle_timeout_seconds)}
              jobs={`${conflict.max_concurrent_jobs}개`}
            />
          </div>
          <div className="flex flex-wrap gap-2">
            <button type="button" className={BTN.plain} disabled={busy} onClick={useServer}>
              서버 값 사용
            </button>
            <button type="button" className={BTN.action} disabled={busy || !valid} onClick={() => void save()}>
              내 값으로 다시 저장
            </button>
          </div>
        </section>
      )}
      {error && (
        <p role="alert" className="text-[13px] font-semibold text-urgent">
          {error}
        </p>
      )}
      <p role="status" className="text-[13px] font-semibold text-text-secondary empty:hidden">
        {saved && !error ? "저장했어요." : null}
      </p>

      <div className="flex flex-wrap items-center gap-x-4 gap-y-3 border-t border-hairline-soft pt-4">
        {conflict === null && (
          <div className="flex flex-wrap gap-2">
            <button type="submit" className={BTN.main} disabled={busy || unchanged || !valid}>
              {busy ? "저장 중" : "저장"}
            </button>
            <button
              type="button"
              className={BTN.plain}
              disabled={busy || unchanged}
              onClick={() => {
                setDraft(base);
                setError(null);
              }}
            >
              되돌리기
            </button>
          </div>
        )}
        <p className="min-w-0 flex-1 basis-48 text-[13px] text-text-muted">
          {known.saved_at === null ? "아직 저장한 적이 없어서 기본값을 쓰고 있어요." : `마지막 저장 ${dateTime(known.saved_at)}`}
        </p>
      </div>

      <Overrides overrides={stored.overrides} />
    </form>
  );
}

/** One side of the conflict banner: the three values under a title. */
function Compare({ title, order, idle, jobs }: { title: string; order: string; idle: string; jobs: string }) {
  return (
    <div className="min-w-0">
      <h3 className="m-0 mb-1.5 text-[13px] font-bold">{title}</h3>
      <dl className="m-0 flex flex-col gap-1.5 text-[13px]">
        {[
          ["자막 형식", order],
          ["유휴 시간", idle],
          ["동시 작업", jobs],
        ].map(([label, value]) => (
          <div key={label}>
            <dt className="text-xs font-semibold text-text-muted">{label}</dt>
            <dd className="m-0 break-words">{value}</dd>
          </div>
        ))}
      </dl>
    </div>
  );
}

/** `작품별 재정의`: the works that order the formats their own way, each with a link to the work. */
function Overrides({ overrides }: { overrides: Policy["overrides"] }) {
  return (
    <section aria-labelledby="overrides-title" className="flex flex-col gap-2.5 border-t border-hairline-soft pt-5">
      <h3 id="overrides-title" className="m-0 text-sm font-bold">
        작품별 재정의
      </h3>
      <p className={hintClass}>작품마다 다른 순서는 그 작품의 상세 화면에서 바꿔요.</p>
      {overrides.length === 0 ? (
        <p className="text-[13px] text-text-secondary">순서를 따로 정한 작품은 없어요.</p>
      ) : (
        <ul className="m-0 flex list-none flex-col gap-2 p-0">
          {overrides.map((work) => (
            <li key={work.work_id}>
              <Link
                to={`/library/${work.work_id}`}
                className="flex flex-wrap items-center justify-between gap-x-3 gap-y-1 rounded-xl border border-hairline bg-surface-2 px-3.5 py-2.5 hover:bg-surface-3"
              >
                <span className="min-w-0 text-sm font-semibold break-words">{work.name}</span>
                <span className="text-[13px] text-text-secondary">{formatOrderText(work.format_order)}</span>
              </Link>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
