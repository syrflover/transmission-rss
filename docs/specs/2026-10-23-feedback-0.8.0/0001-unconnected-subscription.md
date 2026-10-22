# 0001 시즌에 붙지 않은 구독의 원인을 찾아 고쳐요

- 상태: 완료 (2026-10-23, 테스트로 확인. 서버 확인은 [0011](0011-deploy-and-check-on-server.md)에서 해요. 아래 "결과")
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

- [x] 원인과 그 근거(이력의 행, Transmission의 응답, 라이브러리의 경로 같은 관찰)를 결과에 적어요.
- [x] 원인이 결함이면, 그 원인을 재현하는 테스트가 고치기 전에는 실패하고 고친 뒤에는 통과해요.
- [ ] 원인이 지금 요구가 허용한 경우(예: Transmission이 토렌트를 지움)이면 고치지 않고, 어떤 대안이 있는지와 함께 사용자에게 알려 정해요. 정한 것을 결과에 적어요.
- [x] 고친 동작에 맞춰 [라이브러리 명세](../../requirements/library.md)나 [RSS 수집 명세](../../requirements/collection.md)의 구독 시즌 연결 문장과 검증 표의 행을 고쳐요.
- [x] `cargo test --workspace`가 통과해요.

세 번째 기준은 원인이 결함이라 해당하지 않아서 비워 뒀어요. 고치는 방법은 대안과 함께 사용자에게 알려 정했어요(아래 "정한 것").

## 결과

### 원인

이 구독이 받은 영상은 지난 회차 `- 24` 하나였고, 사용자는 그 항목이 피드에서 빠진 뒤에 받았어요. trss는 피드에서 빠진 trss 라벨 토렌트를 다 받은 뒤 첫 정리에서 Transmission에서 지웠고, 수집 주기 하나의 순서는 정리 → 감시 폴더 읽기 → 시즌 연결이에요. 그래서 시즌 연결은 Transmission이 그 토렌트를 가진 동안 라이브러리가 영상을 아는 때를 한 번도 만나지 못했어요.

방영이 끝난 뒤 구독해 피드에서 빠진 지난 회차만 받은 구독은 언제나 이렇게 돼요. 방영 중에 구독한 구독은 새 회차가 피드에 있는 동안 시즌에 붙어요. 작업에 적은 후보 가운데 0123의 이동과 `Memory`는 원인이 아니었어요.

### 근거

- 서버 DB 사본(2026-10-22에 `dev/compose.sh pull`로 받은 것)의 수집 이력에서, 이 규칙이 받은 항목은 `[SubsPlease] Seihantai na Kimi to Boku - 24 (1080p) [78085A98].mkv` 하나예요(해시 `09ca69c1a0394fcf0b53480a9af86c40a855d284`). 이 항목은 SubsPlease 피드에서 서울 시각 2026-10-04 04:01:37에 마지막으로 보였고, 2026-10-08 12:58:57에 받았어요.
- 같은 사본의 라이브러리에는 그 작품의 `Season 02/Seihantai na Kimi to Boku S02E12.mkv`가 2026-10-08 13:02:43에 기록돼 있어요. 시즌 2를 잡은 Anissia 연결이나 다른 구독은 없어서, 다른 작품이 시즌을 잡아 막힌 경우가 아니에요.
- [5-deployed-verification/0123](../../archive/tickets/5-deployed-verification/0123-collect-moves-archived-work-folder.md)의 서버 기록대로 작품 폴더는 구독할 때 `Shows (current)`로 옮겨졌고, 영상도 옮긴 뒤에 들어왔어요. 라이브러리가 경로로 영상을 찾는 코드(`find_videos`)는 감시 폴더 경로를 `/` 경계에서만 잘라, `/downloads/Shows`와 `/downloads/Shows (current)`를 헷갈리지 않아요.
- 시즌 연결의 코드는 2026-10-02 뒤로 테스트를 옮긴 것 말고는 바뀌지 않았어요.
- 그때의 서버 로그는 그 뒤 배포에서 컨테이너가 다시 만들어져 남지 않았고, 지금 서버의 Transmission에 그 토렌트가 없는지도 보지 않았어요. 원인은 위의 날짜와 아래 재현으로 정했어요.
- 재현 테스트는 실제 worker의 주기를 돌려요. 피드에 없는 항목의 다 받은 trss 라벨 토렌트와 라이브러리의 영상을 두면, 고치기 전에는 그 주기의 로그에 `Removed name | 0000000000000000000000000000000000000001`이 감시 폴더 읽기보다 먼저 찍히고 규칙이 시즌에 붙지 않아 실패했어요. 라벨이 없는 토렌트로 같은 일을 하는 기존 테스트는 통과했어요.

### 정한 것

사용자가 정리를 한 번 늦추는 방법을 골랐어요(2026-10-23). 다 받은 trss 라벨 토렌트도 정리 대상인 것을 처음 본 정리에서는 남기고, 다음 정리에서도 정리 대상이면 지워요. 기본 주기가 5분이라 피드에서 빠진 토렌트가 그만큼 더 남아요.

다른 두 안은 고르지 않았어요. 시즌 연결을 정리보다 먼저 하면 정리 시점은 그대로지만, 두 단계 사이 몇 초 동안 다운로드가 끝나면 또 놓쳐요. 시즌을 기다리는 구독의 토렌트를 남기면, 시즌에 붙을 수 없는 구독(영상이 두 시즌에 있거나 다른 작품이 시즌을 잡음)의 토렌트가 계속 쌓여요.

리드가 정한 것도 있어요. 처음 본 토렌트는 worker 프로세스에만 기억해서, worker를 다시 시작하면 한 번 더 기다려요. 채널을 못 읽었거나 명령이 토렌트를 다루는 중이라 정리를 건너뛴 주기는 세지 않아요. [RSS 수집 명세의 채널과 다운로드 규칙](../../requirements/collection.md#채널과-다운로드-규칙)에 이 동작과 까닭을, [방영작 구독](../../requirements/collection.md#방영작-구독)에 지난 회차만 받은 구독도 이어진다는 문장을 더했어요.

### 테스트

2026-10-23에 334377fb 위의 작업 트리에서 `cargo test --workspace`가 모두 통과했어요(3,203개 통과, 12개 무시).

- 새 테스트 `trss` `tests/it/season_link.rs` `a_past_episode_received_after_its_item_left_the_feed_connects_the_subscription`은 위의 재현이에요. 고치기 전에는 실패했고 고친 뒤에는 통과해요.
- 새 테스트 `trss-transmission` `item_tests.rs` `a_torrent_accounted_for_between_two_looks_stays_and_starts_over`는 두 정리 사이에 다시 정리 대상이 아니게 된 토렌트가 남고 처음부터 다시 세는 것을 겨눠요. 같은 파일의 `a_cleanup_removes_only_finished_bot_torrents_whose_item_is_gone_and_keeps_their_data`는 두 번째 정리에서 지우는 것을 겨누도록 고쳤어요.
- 정리를 한 주기 안에서 기대하던 기존 테스트 7개는 주기를 하나 더 돌린 뒤 정리를 확인하도록 고쳤어요. `trss-collect`의 `cycle/tests.rs` 3개와 `revisions/several_tests.rs` 1개, `trss`의 `tests/it/worker_cycle.rs` 3개예요.
- `trss`의 `tests/it/receive_once.rs` `the_cycles_removal_leaves_the_torrents_alone_while_a_command_adds`도 같은 까닭으로 마지막 정리를 한 주기 뒤에 확인해요.

### 한계

- 서버에서 확인하지 않았어요. [0011](0011-deploy-and-check-on-server.md)에서 배포한 뒤 봐요.
- 이 수정은 이미 지워진 Seihantai의 토렌트를 되돌리지 않아요. 그 구독은 [0003](0003-archive-deletes-the-rule.md)의 마이그레이션이 지우고, 작품은 사용자가 작품 상세에서 시즌을 이을 때까지 폴더 이름으로 보여요(사용자 결정, [보관 ADR](../../adr/0022-archiving-deletes-rules-and-follows-the-folder.md)).
- 다 받기 전에 도는 시즌 연결은 Transmission에는 파일이 있고 라이브러리에는 없는 이 상황을 "다운로드 폴더가 다른 경로에 마운트됐을 수 있다"는 로그로 worker 시작마다 규칙당 한 번 남겨요. 이번 변경 전부터 있던 동작이고 고치지 않았어요.
