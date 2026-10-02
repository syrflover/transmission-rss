import type { ComponentType, ComponentProps } from "react";

import {
  ArchiveIcon,
  CaptionIcon,
  DownloadIcon,
  FolderIcon,
  LockIcon,
  UploadIcon,
} from "./icons";

export type SettingsItemId =
  | "policy"
  | "collection"
  | "folders"
  | "storage"
  | "export"
  | "import"
  | "deploy";

export interface SettingsItem {
  id: SettingsItemId;
  group: string;
  title: string;
  icon: ComponentType<ComponentProps<"svg">>;
  /**
   * Shown in the panel while the item has no content yet. Each item is filled
   * by the result goal that owns it (see docs/specs/settings.md); only the
   * common policy, the collect folder, the watch folders and the import are built so far.
   */
  empty?: string;
}

/** The settings items in list order, grouped as in docs/specs/settings.md#설정-화면. */
export const SETTINGS_ITEMS: SettingsItem[] = [
  { id: "policy", group: "공통 정책", title: "자막 형식·브라우저", icon: CaptionIcon },
  { id: "collection", group: "수집", title: "수집 폴더", icon: FolderIcon },
  { id: "folders", group: "라이브러리", title: "감시 폴더", icon: FolderIcon },
  {
    id: "storage",
    group: "라이브러리",
    title: "파일 용량·정리",
    icon: ArchiveIcon,
    empty: "보관한 파일이 아직 없어요. 자막이나 표지를 보관하면 용량과 정리할 작품이 여기에 나와요.",
  },
  {
    id: "export",
    group: "데이터",
    title: "내보내기",
    icon: DownloadIcon,
    empty: "아직 내보낼 수 없어요. 내보내기가 준비되면 여기에서 앱 설정을 파일로 내보내요.",
  },
  { id: "import", group: "데이터", title: "가져오기", icon: UploadIcon },
  {
    id: "deploy",
    group: "배포",
    title: "배포 설정",
    icon: LockIcon,
    empty: "배포 설정은 아직 보여줄 수 없어요. Transmission 주소와 서버 포트 같은 값은 배포 설정에서만 바뀌고 여기에는 읽기 전용으로 나와요.",
  },
];

export function findItem(id: string | undefined): SettingsItem | undefined {
  return SETTINGS_ITEMS.find((item) => item.id === id);
}
