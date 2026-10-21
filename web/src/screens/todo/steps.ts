/**
 * What each step of a job is called on the screens (`docs/specs/jobs.md`, 작업 상세). The keys are the step kinds,
 * but this file does not import `api.ts`, so the modules the tests reach stay free of it; `JobSteps.tsx` checks that
 * every `StepKind` has a label.
 */
export const STEP_LABEL = {
  found: "후보 발견",
  open: "게시물 열기",
  auth: "인증",
  receive: "받기",
  placement: "배치 확인",
  store: "보관",
  approval: "교체 승인",
  apply: "적용",
} as const;

/** The label of a step kind this build may not know (the server is newer), else the kind itself. */
export function stepLabel(kind: string): string {
  return (STEP_LABEL as Readonly<Record<string, string>>)[kind] ?? kind;
}
