/**
 * The work in the archive folder that a rule saving to `directory` would bring
 * into the collect folder, by the library's records. `merges` is whether the
 * collect folder has the work too, so the two become one. It is declared here,
 * not in `api.ts`, so this module imports nothing and runs and is type-checked
 * under `node --test`.
 */
export interface ArchivedWork {
  work: string;
  archive_folder: string;
  collect_folder: string;
  merges: boolean;
}

/**
 * The request that asks which work of the archive folder a rule saving to
 * `directory` would bring over, or `null` while nothing is typed. The folder is
 * sent as typed: the server finds its work folder (`./A/S3` and `A/S3` are one),
 * so the notice and the rule it announces cannot disagree on spelling. A form
 * that edits a stored rule sends the rule's folder as `from`, and the server
 * answers nothing for a folder in the same work folder.
 */
export function archivedWorkPath(directory: string, from: string | null = null): string | null {
  const typed = directory.trim();
  if (typed === "") return null;
  const stored = from === null ? "" : `&from=${encodeURIComponent(from.trim())}`;
  return `/rules/archived-work?directory=${encodeURIComponent(typed)}${stored}`;
}

/**
 * Whether a rule just made can receive past items now: only one that came back
 * collecting. The server holds a rule for its work folder by the disk, so the
 * answer to the made rule, not the notice shown before, tells.
 */
export function receivesPastItemsNow(ruleState: string): boolean {
  return ruleState === "active";
}

/** The last component of a folder path, to name a folder as the user knows it. */
function folderName(path: string): string {
  const parts = path.split("/").filter((part) => part !== "");
  return parts[parts.length - 1] ?? path;
}

/**
 * What the rule-add and subscription forms say before the rule is made, when
 * its work folder is in the archive folder: the folder comes over first, the
 * rule collects after, and a work in both folders is merged.
 */
export function archivedWorkNotice(archived: ArchivedWork): { title: string; detail: string } {
  const from = `${folderName(archived.archive_folder)}/${archived.work}`;
  const to = folderName(archived.collect_folder);
  return {
    title: `보관 폴더의 ‘${from}’를 수집 폴더(${to})로 옮겨요.`,
    detail:
      (archived.merges
        ? `수집 폴더에도 ‘${archived.work}’가 있어서 두 폴더를 하나로 합쳐요. 같은 이름의 파일이 겹치면 아무것도 옮기지 않고 규칙은 멈춘 채로 둬요. `
        : "겹치는 파일이 있으면 아무것도 옮기지 않고 규칙은 멈춘 채로 둬요. ") +
      "옮기기가 끝나기 전에는 규칙이 아무것도 받지 않아요. 규칙 화면에서 결과를 볼 수 있어요.",
  };
}
