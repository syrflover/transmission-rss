import { EmptyState, ScreenFrame } from "./ScreenFrame";

export function LibraryScreen() {
  return (
    <ScreenFrame title="라이브러리">
      <EmptyState>아직 발견한 작품이 없어요. 감시 폴더를 연결하면 작품이 여기에 나타나요.</EmptyState>
    </ScreenFrame>
  );
}
