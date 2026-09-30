import { EmptyState } from "../../ScreenFrame";

/** The subscriptions tab. Filled when subscriptions arrive (outcome 2). */
export function SubsTab() {
  return (
    <EmptyState>
      아직 구독한 작품이 없어요. 구독하면 작품마다 받을 채널과 저장 폴더가 여기에 모여요.
    </EmptyState>
  );
}
