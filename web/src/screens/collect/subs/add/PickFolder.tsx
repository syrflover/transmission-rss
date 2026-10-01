import { useId } from "react";

import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

import { hintClass, inputClass, labelClass } from "../../channels/styles";
import { folderProblem } from "./draft";

/** Step 5: the save folder, suggested from the release title and editable. */
export function PickFolder({
  directory,
  onChange,
}: {
  directory: string;
  onChange: (directory: string) => void;
}) {
  const uid = useId();
  const problem = folderProblem(directory);

  return (
    <section aria-labelledby={`${uid}-h`} className="flex min-w-0 flex-col gap-3">
      <h3 id={`${uid}-h`} className="text-[15px] font-bold">
        저장 폴더를 정해요
      </h3>
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
          {problem ?? "고른 릴리스 제목으로 만든 제안이에요. 수집 폴더 아래의 상대 경로이고, 고쳐도 돼요."}
        </p>
      </div>
    </section>
  );
}
