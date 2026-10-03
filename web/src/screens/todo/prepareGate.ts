/**
 * When a job's page asks for its remote screen to be prepared
 * (`POST …/screen`): once per opening of the page, on the first read of the
 * job that comes from the server. The job shown first may be cached from an
 * earlier visit, so it is not the reason to ask. Only a job that has a screen
 * asks. Every later read (polling the job, and the reads a reconnecting
 * socket makes) never asks; the person's own `다시 열기` is the only other
 * request.
 */
export interface PrepareGate {
  /** The job the page showed first, possibly cached. */
  readonly cached: unknown;
  /** The page's one read that decides was made. */
  decided: boolean;
}

/** The gate of a page opened showing `cached` (`undefined`: nothing yet). */
export function openGate(cached: unknown): PrepareGate {
  return { cached, decided: false };
}

/** Whether this read of the job asks for the screen; decides the gate on the first read from the server. */
export function asksOnRead(gate: PrepareGate, job: { screen: { state: string } | null } | undefined): boolean {
  if (gate.decided || job === undefined || job === gate.cached) return false;
  gate.decided = true;
  return job.screen !== null && job.screen.state !== "unavailable";
}
