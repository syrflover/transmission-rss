/**
 * How long to wait before the n-th attempt (1-based) to reconnect after a
 * socket that closed on its own; `null` when to stop and let the person ask
 * again. The counter starts over when a frame arrives.
 */
const DELAYS_MS = [500, 1000, 2000, 4000, 8000];

export function retryDelay(failures: number): number | null {
  return failures >= 1 && failures <= DELAYS_MS.length ? DELAYS_MS[failures - 1] : null;
}

/**
 * A run bound to the job: the run, and when it was bound (`bound`). Another
 * check of the job in the same run is another binding.
 */
export interface Binding {
  run: string;
  bound: number | null;
}

export function sameBinding(a: Binding, b: Binding): boolean {
  return a.run === b.run && a.bound === b.bound;
}

/** What a socket's end leaves to do. */
export type Outcome =
  /** Connect (again) to `binding`. */
  | { kind: "connect"; binding: Binding }
  /** The job no longer has a screen to connect to; the page shows why. */
  | { kind: "stop" };

/**
 * After the job was read again: the binding to connect to, if the job still
 * waits for its check with a run bound (`ready`). A reconnect never asks for
 * a run: a screen that is `preparing` or `closed` ends here, and only the
 * person's opening of the page (`POST …/screen`) prepares it again.
 *
 * `ended` is the binding whose socket the server ended: it is over, so it is
 * not connected to again (the server would end it again), while a binding
 * that took its place is, even of the same run.
 */
export function afterEnd(
  job: { waiting: boolean; screen: { state: string; run: string | null; bound: number | null } | null },
  ended: Binding | null,
): Outcome {
  const screen = job.screen;
  if (!job.waiting || screen === null || screen.state !== "ready" || screen.run === null) return { kind: "stop" };
  const binding = { run: screen.run, bound: screen.bound };
  if (ended !== null && sameBinding(binding, ended)) return { kind: "stop" };
  return { kind: "connect", binding };
}
