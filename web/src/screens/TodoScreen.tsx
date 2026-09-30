import { EmptyState, ScreenFrame } from "./ScreenFrame";

export function TodoScreen() {
  return (
    <ScreenFrame title="할 일">
      <EmptyState>처리할 일이 없어요. 인증이나 승인이 필요한 일이 생기면 여기에 모여요.</EmptyState>
    </ScreenFrame>
  );
}
