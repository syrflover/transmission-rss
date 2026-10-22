import { stepLabel } from "./steps.ts";

/**
 * A job the process stopped in the middle of a step (`중단됨`) is the only one with `다시 시작`: a person starts it
 * again and it goes on from the step it was cut in (`docs/requirements/jobs.md`, 체크포인트와 중단 복구).
 */
export function canRestart(job: { state: string }): boolean {
  return job.state === "interrupted";
}

/**
 * How the step track draws a step's state: an `interrupted` job keeps the step it was cut in as `current`, which the
 * track draws as stopped (`중단`), not as going on.
 */
export function drawnStepState<S extends string>(jobState: string, state: S): S | "stopped" {
  return jobState === "interrupted" && state === "current" ? "stopped" : state;
}

/** What an `interrupted` job says about itself: the step it was cut in, and what the restart does. */
export function interruptedText(stage: string | null): string {
  const where = stage === null ? "실행하던 중에" : `${stepLabel(stage)} 단계를 실행하던 중에`;
  return `${where} 멈췄어요. 다시 시작하면 끊긴 단계부터 이어서 해요.`;
}
