import { useCallback, useEffect, useRef, useState } from "react";

import { ApiError } from "@/lib/api";
import { patch, store } from "@/lib/cached";
import { newCommandId } from "@/lib/commands";

import { createSubtitleJob } from "../../todo/api";
import { usePolled } from "../../todo/poll";
import {
  candidatesKey,
  loadCandidates,
  type Candidate,
  type CandidateJob,
  type CandidateList,
  type CandidateMapping,
  type WorkEpisode,
} from "../api";
import { segmentsText } from "./episodeKey.ts";
import { episodeLabel, type EpisodeOrder } from "./model";

/**
 * What the `자막 후보` section and the episode rows derive from the candidates
 * answer: the groups by creator, the newest observation of each episode, the
 * kinds, the counts and which library episode a candidate is about. A
 * candidate's episode is Anissia's text, so the screen reads none of it as a
 * number: the server sends the key two texts of one episode share
 * (`episode_key`), the episode's place in the order (`episode_rank`) and what
 * kind of text it is (`episode_form`).
 */

// --- episodes ---------------------------------------------------------------------------

/**
 * Whether a candidate's episode is the library episode. This compares Anissia's
 * label as written with the library's episode number: creators number one
 * anime differently (season-relative `12` or cumulative `24`), and the per-source
 * episode mapping (자막의 회차 대응) that tells them apart does not exist yet.
 */
export function matchesEpisode(c: Pick<Candidate, "episode_key">, episode: Pick<WorkEpisode, "episode_key">): boolean {
  return c.episode_key === episode.episode_key;
}

/** A candidate's episode as shown, always with its own label: `24화`; text that is no episode number (`0`, `SP`) as written. */
export function candidateLabel(c: Pick<Candidate, "episode" | "episode_shown" | "episode_form">): string {
  return c.episode_form === "number" ? episodeLabel(c.episode_shown) : c.episode;
}

type Ranked = Pick<Candidate, "episode_rank" | "episode_form">;

/** The episodes in the order asked for: numbers ascending (or descending), then the other texts as written. */
function compareEpisodes(a: Ranked, b: Ranked, order: EpisodeOrder): number {
  const x = a.episode_form !== "text";
  const y = b.episode_form !== "text";
  if (x && y) return order === "latest" ? b.episode_rank - a.episode_rank : a.episode_rank - b.episode_rank;
  if (x) return -1;
  if (y) return 1;
  return a.episode_rank - b.episode_rank;
}

// --- categories and job states -------------------------------------------------------------

/**
 * What a candidate is to the user, the first that applies:
 * `received` a job received it; `revision` the same creator's subtitle of the
 * episode was received before; `has` the episode has a subtitle already;
 * `missing` the episode has none.
 */
export type Kind = "received" | "revision" | "has" | "missing";

export const KIND_TEXT: Record<Kind, string> = {
  received: "받음",
  revision: "수정",
  has: "자막 있음",
  missing: "누락",
};

/** The episodes (`episode_key`) of the season that have a subtitle: a library episode with a subtitle file, or a candidate a job received. */
export type Holdings = ReadonlySet<string>;

export function holdingsOf(list: CandidateList, episodes: readonly WorkEpisode[]): Holdings {
  const has = new Set<string>();
  for (const episode of episodes) if (episode.subtitle.length > 0) has.add(episode.episode_key);
  for (const c of list.candidates) if (c.job?.state === "done") has.add(c.episode_key);
  return has;
}

export function kindOf(c: Candidate, holdings: Holdings): Kind {
  if (c.job?.state === "done") return "received";
  if (c.revision !== null) return "revision";
  return holdings.has(c.episode_key) ? "has" : "missing";
}

/** A job holds the candidate (running, waiting or done), so it cannot be taken again; `failed` and `held` can. */
export function isHeld(c: Candidate): boolean {
  const state = c.job?.state;
  return state === "pending" || state === "running" || state === "waiting" || state === "done";
}

/** What a job's item says about its candidate, in a word; `null` for none. */
export function jobStatus(job: CandidateJob | null): string | null {
  switch (job?.state) {
    case undefined:
      return null;
    case "pending":
    case "running":
      return "받는 중";
    case "waiting":
      switch (job.wait) {
        case "auth":
          return "인증 필요";
        case "subtitle":
          return "자막 대기";
        case "placement":
          return "회차 확인 필요";
        case "approval":
          return "교체 승인";
        case "video":
          return "영상 대기";
        default:
          return "대기";
      }
    case "held":
      return "보류";
    case "failed":
      return "실패";
    case "done":
      return "받음";
  }
}

/** Whether a job of the list is going on, so the list should be read often. */
function active(list: CandidateList): boolean {
  const open = (state: string) => state === "pending" || state === "running";
  return list.candidates.some((c) => c.job !== null && open(c.job.state)) || (list.refresh !== null && open(list.refresh.state));
}

// --- groups ---------------------------------------------------------------------------------

/** One episode of a creator: its newest observation, and the older ones the user still has picked. */
export interface CandidateRow {
  key: string;
  candidate: Candidate;
  kind: Kind;
  /** Older observations of the episode that stay on screen because they are picked, newest first. */
  picked: { candidate: Candidate; kind: Kind }[];
}

export interface CandidateGroup {
  sourceId: string;
  creator: string;
  /** The host of the newest observation's post. */
  host: string | null;
  /** The subscription's creator. */
  subscribed: boolean;
  rows: CandidateRow[];
  missing: number;
  revision: number;
  /** The episodes the rows are about, as `1–4화` (the server names the runs). */
  range: string;
  /** The newest observation's `sort_at`. */
  latest: number;
}

/** The host of a post address, or `null` when it cannot be read. */
export function hostOf(url: string): string | null {
  try {
    return new URL(url).host || null;
  } catch {
    return null;
  }
}

/**
 * The creators of the candidates, the subscription's creator first and then
 * the one observed latest. A creator has a row for each episode, ordered as
 * the episode list is: the newest observation of the episode, and below it any
 * older one of the episode that is in `picked`.
 */
export function groupsOf(
  list: CandidateList,
  episodes: readonly WorkEpisode[],
  order: EpisodeOrder,
  subscribed: string | null,
  picked: ReadonlySet<number>,
): CandidateGroup[] {
  const holdings = holdingsOf(list, episodes);
  const bySource = new Map<string, Candidate[]>();
  for (const c of list.candidates) {
    const group = bySource.get(c.source_id);
    if (group) group.push(c);
    else bySource.set(c.source_id, [c]);
  }

  const groups: CandidateGroup[] = [];
  for (const [sourceId, observed] of bySource) {
    const byEpisode = new Map<string, Candidate[]>();
    for (const c of observed) {
      const key = c.episode_key;
      const same = byEpisode.get(key);
      if (same) same.push(c);
      else byEpisode.set(key, [c]);
    }
    const rows: CandidateRow[] = [];
    for (const [key, same] of byEpisode) {
      // The IDs grow with the time of observing: the highest is the newest.
      const newest = same.reduce((a, b) => (b.id > a.id ? b : a));
      rows.push({
        key,
        candidate: newest,
        kind: kindOf(newest, holdings),
        picked: same
          .filter((c) => c.id !== newest.id && picked.has(c.id) && !isHeld(c))
          .sort((a, b) => b.id - a.id)
          .map((candidate) => ({ candidate, kind: kindOf(candidate, holdings) })),
      });
    }
    rows.sort((a, b) => compareEpisodes(a.candidate, b.candidate, order));
    const newest = observed.reduce((a, b) => (b.id > a.id ? b : a));
    groups.push({
      sourceId,
      creator: newest.creator,
      host: hostOf(newest.post_url),
      subscribed: subscribed !== null && newest.creator === subscribed,
      rows,
      missing: rows.filter((r) => r.kind === "missing").length,
      revision: rows.filter((r) => r.kind === "revision").length,
      range: segmentsText(list.creator_episodes.find((c) => c.source_id === sourceId)?.episode_segments ?? []),
      latest: Math.max(...observed.map((c) => c.sort_at)),
    });
  }
  groups.sort((a, b) => Number(b.subscribed) - Number(a.subscribed) || b.latest - a.latest);
  return groups;
}

/** The candidates `모두 받기` takes: the group's missing episodes that no job holds, in the order shown. */
export function missingOf(group: CandidateGroup): number[] {
  return group.rows.filter((r) => r.kind === "missing" && !isHeld(r.candidate)).map((r) => r.candidate.id);
}

/** The group's picked candidates in the order shown (an older observation follows the episode's newest). */
export function pickedOf(group: CandidateGroup, picked: ReadonlySet<number>): number[] {
  const ids: number[] = [];
  for (const row of group.rows) {
    if (picked.has(row.candidate.id) && !isHeld(row.candidate)) ids.push(row.candidate.id);
    for (const older of row.picked) if (!isHeld(older.candidate)) ids.push(older.candidate.id);
  }
  return ids;
}

/**
 * The candidates an episode row without a subtitle names: each creator's
 * newest observation of the episode, none when the subscription's creator has
 * one (it is received on its own) and none that a job received.
 */
export function candidatesOf(
  list: CandidateList,
  episode: WorkEpisode,
  subscribed: string | null,
): Candidate[] {
  if (episode.subtitle.length > 0) return [];
  const newest = new Map<string, Candidate>();
  for (const c of list.candidates) {
    if (!matchesEpisode(c, episode)) continue;
    const seen = newest.get(c.source_id);
    if (!seen || c.id > seen.id) newest.set(c.source_id, c);
  }
  const all = [...newest.values()];
  if (subscribed !== null && all.some((c) => c.creator === subscribed)) return [];
  return all.filter((c) => c.job?.state !== "done").sort((a, b) => b.sort_at - a.sort_at);
}

// --- dates ---------------------------------------------------------------------------------

/** `9월 30일 23:06`, with the year when it is not this year's. It is the time Anissia's line was updated, nothing else. */
export function updatedText(c: Candidate, now: Date = new Date()): string {
  if (c.updated_at === null) {
    return c.updated_parse_failed ? `${c.updated} (날짜를 읽지 못했어요)` : "알 수 없음";
  }
  const d = new Date(c.updated_at);
  const pad = (n: number) => String(n).padStart(2, "0");
  const year = d.getFullYear() === now.getFullYear() ? "" : `${d.getFullYear()}년 `;
  return `${year}${d.getMonth() + 1}월 ${d.getDate()}일 ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

// --- reading --------------------------------------------------------------------------------

/** How often the list is read while a job or a refresh is going on, and otherwise. */
const FAST_MS = 3000;
const SLOW_MS = 60_000;

/**
 * The season's candidates, read while the page is shown: often while a job of
 * a candidate or a refresh goes on, otherwise now and then (the worker reads
 * Anissia on its own every 30 minutes). A season with no Anissia link is not
 * read. `reload` reads at once, after an action of this page.
 */
export function useCandidates(workId: string, season: number, animeNo: number | null) {
  const key = candidatesKey(workId, season, animeNo);
  const polled = usePolled<CandidateList | null>(
    key,
    (signal) => (animeNo === null ? Promise.resolve(null) : loadCandidates(workId, season, signal)),
    (data) => (animeNo === null ? null : data && active(data) ? FAST_MS : SLOW_MS),
    "자막 후보를 불러오지 못했어요.",
  );
  const reload = useCallback(async () => {
    if (animeNo === null) return;
    try {
      store(key, await loadCandidates(workId, season));
    } catch {
      // The polling reads it again.
    }
  }, [key, animeNo, workId, season]);
  /** Shows the candidates as taken by a job at once, before the next reading says so. */
  const taken = useCallback(
    (ids: readonly number[], jobId: string) => {
      const job: CandidateJob = { id: jobId, state: "pending", wait: null };
      patch<CandidateList | null>(key, (list) =>
        list ? { ...list, candidates: list.candidates.map((c) => (ids.includes(c.id) ? { ...c, job } : c)) } : list,
      );
      void reload();
    },
    [key, reload],
  );
  /** Shows a creator's mapping as the server now holds it (`null`: it has none) and reads the list again. */
  const setMapping = useCallback(
    (sourceId: string, mapping: CandidateMapping | null) => {
      patch<CandidateList | null>(key, (list) =>
        list
          ? {
              ...list,
              mappings: [...(list.mappings ?? []).filter((m) => m.source_id !== sourceId), ...(mapping ? [mapping] : [])],
            }
          : list,
      );
      void reload();
    },
    [key, reload],
  );
  return { ...polled, reload, taken, setMapping };
}

// --- creating a job ---------------------------------------------------------------------------

/**
 * Where one `받기` is:
 *
 * - `sending`: the request is on its way.
 * - `made`: the server made the job (`202`) or had made it (`200`); it is only accepted, not received.
 * - `unconfirmed`: the answer was lost or the server failed, so it is not known whether the job exists; the same request can be sent again.
 * - `refused`: the server refused it, with a sentence.
 */
export type CreatePhase =
  | { kind: "idle" }
  | { kind: "sending" }
  | { kind: "made"; jobId: string }
  | { kind: "unconfirmed"; message: string }
  | { kind: "refused"; message: string };

interface Request {
  id: string;
  candidates: number[];
}

const sameList = (a: readonly number[], b: readonly number[]) => a.length === b.length && a.every((x, i) => x === b[i]);

/**
 * The `받기` flow of one place (a creator's group, an episode's candidate):
 * a job for the candidates, made by one request.
 *
 * - One ID per press, kept until the server answers `200`/`202` or refuses, so
 *   a double press or a resend after a lost answer makes one job. A press while
 *   one is on its way does nothing.
 * - The same candidates pressed again after a lost answer reuse the ID, in the
 *   same order, so the server sees the same content.
 * - `onMade` runs once the job exists, with the candidates it took.
 * - More than `maxCandidates` (what the server takes for one job) are not sent.
 */
export function useCreateJob(
  workId: string,
  season: number,
  onMade: (candidates: readonly number[], jobId: string) => void,
  maxCandidates: number,
) {
  const [phase, setPhase] = useState<CreatePhase>({ kind: "idle" });
  const request = useRef<Request | null>(null);
  const busy = useRef(false);
  const alive = useRef(true);
  const onMadeRef = useRef(onMade);
  onMadeRef.current = onMade;

  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);

  const send = useCallback(
    async (req: Request) => {
      busy.current = true;
      setPhase({ kind: "sending" });
      try {
        const made = await createSubtitleJob({ id: req.id, work_id: workId, season, candidates: req.candidates });
        request.current = null;
        if (alive.current) setPhase({ kind: "made", jobId: made.id });
        onMadeRef.current(req.candidates, made.id);
      } catch (e) {
        const refused = e instanceof ApiError && (e.code === "invalid" || e.code === "conflict" || e.code === "not_found");
        if (refused) request.current = null;
        if (alive.current) {
          const message = e instanceof ApiError ? e.message : "작업을 만들지 못했어요.";
          setPhase(refused ? { kind: "refused", message } : { kind: "unconfirmed", message });
        }
      } finally {
        busy.current = false;
      }
    },
    [workId, season],
  );

  /** Creates the job for `candidates` (a new request, or the unanswered one when it is the same content). */
  const create = useCallback(
    (candidates: readonly number[]) => {
      if (busy.current || candidates.length === 0 || candidates.length > maxCandidates) return;
      const kept = request.current;
      void send(kept && sameList(kept.candidates, candidates) ? kept : (request.current = { id: newCommandId(), candidates: [...candidates] }));
    },
    [send, maxCandidates],
  );

  /** Sends the unanswered request again, with its ID and its candidates. */
  const resend = useCallback(() => {
    if (!busy.current && request.current) void send(request.current);
  }, [send]);

  return { phase, create, resend };
}
