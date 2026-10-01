import { useId } from "react";

import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

import { hintClass, inputClass, labelClass } from "../../channels/styles";
import { folderProblem } from "./draft";

/**
 * Step 5: the save folder, suggested from the release title and editable. A
 * subscription waiting for its title has no suggestion; the folder is typed
 * here and can be replaced when a title is picked.
 */
export function PickFolder({
  directory,
  waiting = false,
  onChange,
}: {
  directory: string;
  /** Subscribing before the first episode: there is no release title to suggest a folder from. */
  waiting?: boolean;
  onChange: (directory: string) => void;
}) {
  const uid = useId();
  const problem = folderProblem(directory);

  return (
    <section aria-labelledby={`${uid}-h`} className="flex min-w-0 flex-col gap-3">
      <h3 id={`${uid}-h`} className="text-[15px] font-bold">
        저장 폴더를 정해요
      </h3>
      {waiting && (
        <p className="min-w-0 text-[13px] leading-normal text-text-secondary">
          수집 폴더 아래의 상대 경로예요. 릴리스 제목이 아직 없어서 제안이 없어요. 제목 후보를 고를 때 그 제목으로 만든 폴더로 바꿀 수 있어요.
        </p>
      )}
      <div className="flex min-w-0 flex-col gap-1.5">
        <Label htmlFor={`${uid}-dir`} className={labelClass}>
          저장 폴더
        </Label>
        <Input
          id={`${uid}-dir`}
          value={directory}
          onChange={(e) => onChange(e.target.value)}
          spellCheck={false}
          autoComplete="off"
          aria-invalid={problem !== null}
          aria-describedby={`${uid}-dir-hint`}
          className={`${inputClass} font-mono`}
        />
        <p id={`${uid}-dir-hint`} className={problem ? `${hintClass} font-semibold text-urgent` : hintClass}>
          {problem ??
            (waiting
              ? "수집 폴더 아래의 상대 경로로 적어요."
              : "고른 릴리스 제목으로 만든 제안이에요. 수집 폴더 아래의 상대 경로이고, 고쳐도 돼요.")}
        </p>
      </div>
    </section>
  );
}
