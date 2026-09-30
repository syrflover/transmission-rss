import { EmptyState, ScreenFrame } from "./ScreenFrame";

export function CollectScreen() {
  return (
    <ScreenFrame title="수집">
      <EmptyState>아직 등록한 채널이 없어요. 채널을 등록하면 수집한 항목이 여기에 쌓여요.</EmptyState>
    </ScreenFrame>
  );
}
