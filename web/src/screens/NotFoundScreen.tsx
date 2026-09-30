import { Link } from "react-router-dom";

import { EmptyState, ScreenFrame } from "./ScreenFrame";

export function NotFoundScreen() {
  return (
    <ScreenFrame title="화면을 찾을 수 없어요">
      <EmptyState>
        주소가 바뀌었거나 없는 화면이에요.{" "}
        <Link to="/" className="font-semibold text-text-primary underline underline-offset-4">
          이번 주 편성
        </Link>
        으로 이동해 주세요.
      </EmptyState>
    </ScreenFrame>
  );
}
