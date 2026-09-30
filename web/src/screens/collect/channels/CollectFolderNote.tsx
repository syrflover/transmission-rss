import { Link } from "react-router-dom";

import { hintClass } from "./styles";

/**
 * The one line that says where a channel's torrents go: a channel has no
 * folder of its own, every rule's folder is under the app's collect folder.
 */
export function CollectFolderNote({ className }: { className?: string }) {
  return (
    <p className={`${hintClass} ${className ?? ""}`} data-testid="collect-folder-note">
      받는 폴더는 앱의 수집 폴더 아래 규칙의 저장 폴더예요.{" "}
      <Link to="/settings/collection" className="font-semibold text-focus underline underline-offset-2">
        설정의 수집 폴더
      </Link>
      에서 바꿔요.
    </p>
  );
}
