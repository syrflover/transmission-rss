import { EmptyState, ScreenFrame } from "./ScreenFrame";

export function ScheduleScreen() {
  return (
    <ScreenFrame title="이번 주 편성">
      <EmptyState>아직 보여줄 편성이 없어요. 방영작을 구독하면 이번 주 방영 일정이 여기에 나타나요.</EmptyState>
    </ScreenFrame>
  );
}
