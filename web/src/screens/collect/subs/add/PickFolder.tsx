import { useId } from "react";

import { SaveFolderFields } from "../../folder/SaveFolderFields";
import type { AnimeOfFolder } from "../../folder/saveFolder";
import { ArchivedWorkNotice } from "../../rules/ArchivedWorkNotice";
import { useArchivedWork } from "../../rules/useArchivedWork";

/**
 * Step 5: the save folder as a work and a season, suggested from the release
 * title and editable. A subscription waiting for its title has no suggestion;
 * picking a library work fills the season, and the folder can be replaced when
 * a title is picked.
 */
export function PickFolder({
  directory,
  release,
  anime,
  waiting = false,
  onChange,
}: {
  directory: string;
  /** The picked release title's work; `null` while waiting for one. */
  release: string | null;
  anime: AnimeOfFolder;
  /** Subscribing before the first episode: there is no release title to suggest a folder from. */
  waiting?: boolean;
  onChange: (directory: string) => void;
}) {
  const uid = useId();
  const archivedWork = useArchivedWork(directory, directory !== "");

  return (
    <section aria-labelledby={`${uid}-h`} className="flex min-w-0 flex-col gap-3">
      <h3 id={`${uid}-h`} className="text-[15px] font-bold">
        저장 폴더를 정해요
      </h3>
      <p className="min-w-0 text-[13px] leading-normal text-text-secondary">
        {waiting
          ? "릴리스 제목이 아직 없어서 제안이 없어요. 라이브러리 작품을 고르면 시즌을 채워요. 제목 후보를 고를 때 그 제목으로 만든 폴더로 바꿀 수 있어요."
          : "고른 릴리스 제목으로 만든 제안이에요. 수집 폴더 아래 ‘작품/Season NN’에 받고, 작품과 시즌을 고쳐도 돼요."}
      </p>
      <SaveFolderFields
        directory={directory}
        release={release}
        anime={anime}
        onChange={(change) => onChange(change.directory)}
      />
      {archivedWork && <ArchivedWorkNotice archived={archivedWork} />}
    </section>
  );
}
