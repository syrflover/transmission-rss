import type { Command } from "../../../lib/api-types/Command";
import type { WorkArchive } from "../../../lib/api-types/WorkArchive";

/*
 * The notice of a work's `보관 폴더로 옮기기`. It imports only types (by a relative path, erased at run time), so it
 * runs and is type-checked under `node --test`.
 */

/**
 * Where one `보관 폴더로 옮기기` is (`useWorkArchive.ts` moves it on):
 *
 * - `waiting`: the server deleted the rules and has the command, the worker has
 *   not ended it. The folder may be moving.
 * - `unconfirmed`: the answer to the request was lost, or the server cannot be
 *   reached, so it is not known whether the server has the command (and
 *   deleted the rules).
 * - `ended`: the worker ended the command on this page; the outcome stays shown
 *   after the work is read again, even once the work left the collect folder.
 */
export type WorkArchivePhase =
  | { kind: "idle"; error: string | null }
  | { kind: "sending" }
  | { kind: "waiting" }
  /** `lost`: the server answered that it has no such command, so sending it again with the same ID is safe. */
  | { kind: "unconfirmed"; lost: boolean }
  | { kind: "ended"; command: Command };

const MOVING = "보관 폴더로 옮기는 중이에요.";

/** One sentence of the notice, with what explains it. */
export interface ArchiveLine {
  title: string;
  detail: string | null;
  failed: boolean;
  /** When the move ended, `null` while it has not. */
  at: number | null;
}

/** What the work's `보관 폴더로 옮기기` notice shows. */
export interface ArchiveNotice {
  line: ArchiveLine | null;
  /** Why the last request was refused. */
  error: string | null;
  /** `다시 옮기기` is offered. */
  moveAgain: boolean;
}

function endedLine(command: Command): ArchiveLine {
  const reason = command.outcome?.reason ?? null;
  const at = command.finished_at;
  if (command.state === "failed") {
    return {
      title: "보관 폴더로 옮기지 못했어요.",
      // The move's reasons say what moved and what to do before `다시 옮기기`.
      detail: reason ?? "까닭은 알 수 없어요.",
      failed: true,
      at,
    };
  }
  if (command.outcome?.result === "kept") {
    return { title: "작품 폴더를 옮기지 않았어요.", detail: reason, failed: false, at };
  }
  return { title: "보관 폴더로 옮겼어요.", detail: reason, failed: false, at };
}

function phaseLine(phase: WorkArchivePhase): ArchiveLine | null {
  switch (phase.kind) {
    case "sending":
      return { title: MOVING, detail: "접수하는 중이에요.", failed: false, at: null };
    case "waiting":
      return { title: MOVING, detail: "접수됐어요. 끝나면 여기에 결과가 나와요.", failed: false, at: null };
    case "unconfirmed":
      return {
        title: MOVING,
        detail: phase.lost
          ? "접수됐는지 확인했더니 서버에 요청이 없어요. 같은 요청을 다시 보낼 수 있어요."
          : "접수됐는지 확인하지 못했어요. 결과를 알 수 없어 다시 확인하고 있어요.",
        failed: false,
        at: null,
      };
    case "ended":
      return endedLine(phase.command);
    case "idle":
      return null;
  }
}

/**
 * The notice of the work's move: this page's own move while it goes and once it ended; otherwise the move the server
 * shows (`work.archive.command`: one that did not go through while the work is in the collect folder with no rule
 * saving into it), whose open state the page follows as its own. `null` when there is nothing to say.
 */
export function archiveNotice(work: { archive: WorkArchive }, phase: WorkArchivePhase): ArchiveNotice | null {
  let line = phaseLine(phase);
  let ended: Command | null = phase.kind === "ended" ? phase.command : null;
  const stored = work.archive.command;
  const storedOpen = stored !== null && (stored.state === "pending" || stored.state === "running");
  if (line === null && phase.kind === "idle" && stored && !storedOpen) {
    ended = stored;
    line = endedLine(stored);
  }
  const error = phase.kind === "idle" ? phase.error : null;
  if (line === null && error === null) return null;
  return { line, error, moveAgain: ended?.state === "failed" && work.archive.available };
}
