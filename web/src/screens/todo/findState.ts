/**
 * What a find job's (직접 찾기) badge names, apart from any job's: `finding` while its server browser is open for
 * the user, `finishing` once the user finished it until it ends, and `nothing` when it ended with no file kept
 * (`받은 파일 없음`, not `받음`). `null` when the job is named as any job is. `kept` is how many files it kept.
 */
export function findShown(job: {
  state: string;
  wait: string | null;
  finishing: boolean;
  kept: number;
}): "finding" | "finishing" | "nothing" | null {
  if (job.finishing) return "finishing";
  if (job.state === "waiting" && job.wait === "auth") return "finding";
  if (job.state === "done" && job.kept === 0) return "nothing";
  return null;
}
