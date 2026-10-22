# 0001 시즌에 붙지 않은 구독의 원인을 찾아 고쳐요

- 상태: 대기
- 출처: [명세의 작품명](spec.md#작품명), 사용자 피드백 4·5번(2026-10-23)
- 막는 티켓: 없음

## 작업

서버(0.8.0)의 `Seihantai na Kimi to Boku`는 작품명이 폴더 이름이에요. 그 작품의 `정반대의 너와 나 2기` 구독(규칙 `d45f0159-b9c2-40e7-b510-73b92c2ef8e7`, 저장 폴더 `Seihantai na Kimi to Boku/Season 02`)이 시즌에 붙지 않았기 때문이에요(`season_id`와 `season_blocked` 모두 없음, 2026-10-23 API로 확인). 서울 시각으로 2026-10-08 03:23에 구독해 같은 날 12:58에 마지막으로 받았고, 2026-10-14 02:58에 보관했어요.

구독을 시즌에 잇는 단계(`trss-collect`의 `season_link.rs`)는 2026-10-02부터 있었어요. 그래서 이 규칙이 수집 중이던 6일 동안 붙었어야 해요. 그 단계는 규칙이 받은 수집 이력 → Transmission이 알려 주는 토렌트 파일의 지금 경로 → 라이브러리가 그 경로에서 찾은 작품·시즌 순서로 시즌을 정해요. 어느 고리에서 끊겼는지 `stora:diagnosing-bugs`의 방식으로 찾아요. 후보는 이래요.

- Transmission이 그 토렌트를 더는 갖고 있지 않아요. 지금 요구에서는 이 경우 규칙을 그대로 둬요.
- 구독할 때 한 이동([5-deployed-verification/0123](../../archive/tickets/5-deployed-verification/0123-collect-moves-archived-work-folder.md), 보관 폴더에서 수집 폴더로 옮김) 때문에 라이브러리가 아는 경로와 Transmission의 경로가 달라졌어요.
- 시도를 기억하는 메모리(`Memory`)가 다시 시도하지 않았어요.

진단에는 지금의 서버나 서버 DB의 새 복사본을 써요(`dev/compose.sh pull`은 서버의 컨테이너를 잠시 멈추므로 사용자에게 먼저 물어요). [0003](0003-archive-deletes-the-rule.md)의 마이그레이션이 이 규칙과 구독을 지우므로, 그 릴리스가 서버에 올라가기 전에 진단을 마쳐요. 업데이트 뒤 이 작품이 폴더 이름으로 남는 것은 사용자 결정이라 고치지 않아요([보관 ADR](../../adr/0022-archiving-deletes-rules-and-follows-the-folder.md)).

## 완료 기준

- [ ] 원인과 그 근거(이력의 행, Transmission의 응답, 라이브러리의 경로 같은 관찰)를 결과에 적어요.
- [ ] 원인이 결함이면, 그 원인을 재현하는 테스트가 고치기 전에는 실패하고 고친 뒤에는 통과해요.
- [ ] 원인이 지금 요구가 허용한 경우(예: Transmission이 토렌트를 지움)이면 고치지 않고, 어떤 대안이 있는지와 함께 사용자에게 알려 정해요. 정한 것을 결과에 적어요.
- [ ] 고친 동작에 맞춰 [라이브러리 명세](../../requirements/library.md)나 [RSS 수집 명세](../../requirements/collection.md)의 구독 시즌 연결 문장과 검증 표의 행을 고쳐요.
- [ ] `cargo test --workspace`가 통과해요.
