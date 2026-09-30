import { EmptyState, ScreenFrame } from "./ScreenFrame";

export function SettingsScreen() {
  return (
    <ScreenFrame title="설정">
      <EmptyState>아직 바꿀 수 있는 설정이 없어요. 설정 항목이 준비되면 여기에서 바꿀 수 있어요.</EmptyState>
    </ScreenFrame>
  );
}
