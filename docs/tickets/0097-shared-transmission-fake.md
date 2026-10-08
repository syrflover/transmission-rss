# 0097 Transmission 가짜 서버를 trss-collect의 테스트도 쓰게 옮겨요

- 상태: 완료 (2026-10-08)
- 출처: [수집 받기 줄기의 재구성](refactoring.md#수집-받기-줄기의-재구성), [ADR 0015](../adr/0015-test-a-rule-once-in-its-crate.md)
- 막는 티켓: [0092](0092-refactor-baseline.md)(기준값)

## 작업

Transmission 가짜 서버는 trss-worker의 테스트 폴더(`tests/it/common`)에만 있어요. 2026-10-07에 worker 테스트 파일 18개 중 9개, 테스트 399개 중 330개가 썼어요.
그래서 Transmission이 끼는 수집 규칙은 지금 worker 테스트로만 확인할 수 있고, [0101](0101-receive-line-tests.md)에서 그 규칙 테스트를 trss-collect로 내릴 수 없어요.
가짜 서버를 trss-collect의 테스트도 쓸 수 있는 곳으로 먼저 옮겨요. 가짜 AniList와 가짜 Anissia가 각 클라이언트 크레이트에 `fake`로 있는 것이 앞선 예예요.
같은 폴더의 가짜 피드 서버와 가짜 nyaa도 수집 규칙 테스트가 쓰면 함께 옮겨요.

## 완료 기준

- worker 테스트가 옮긴 가짜 서버를 쓰고, 통과·건너뛴 테스트 수가 앞뒤로 같아요.
- trss-collect의 테스트 하나 이상이 옮긴 가짜 서버로 돌아요.
- 가짜 서버가 trss-web과 trss-worker의 제품 빌드에 들어가지 않아요.

## 결과

### 옮긴 곳

`8239ab7`(`refactor(test): move the fake Transmission, feed and nyaa servers into the crates that own their clients`)에서 옮겼어요. 가짜 AniList·Anissia처럼, 가짜가 흉내 내는 클라이언트를 가진 크레이트의 `fake` 모듈에 두고 `test-support` 기능 뒤에 숨겼어요.

| 가짜 | 지금 |
| --- | --- |
| Transmission(`FakeTransmission`, `FakeTorrent`, `Call`), 요청을 붙잡는 `Gate` | trss-transmission `fake`. `Gate`를 쓰는 가짜들이 모두 의존하는 가장 아래 크레이트예요. worker 쪽에 따로 있던 봇 라벨 상수는 trss-transmission의 `BOT_LABEL`을 써요. |
| 피드 서버(`FeedServer`), nyaa 검색(`FakeNyaa`) | trss-collect `fake`. 피드와 지난 회차 검색을 받아 오는 코드가 trss-collect에 있어요. |

- 옮긴 코드는 보이는 범위, `use`, 모듈 경로만 바뀌었어요.
- worker 테스트의 `common`에는 `Harness`, `WebApi`, 규칙 도우미, fixture가 남고, 옮긴 가짜를 다시 내보내요. 그래서 worker 테스트 파일은 하나도 고치지 않았어요. `common/mod.rs`는 약 1,400줄 줄었어요.
- trss-worker는 dev-dependency에서만 두 크레이트의 `test-support`를 켜요.

### 제품 빌드에 들어가지 않는 것

- `cargo tree -p trss-worker -e normal,build -e features`에 `feature "test-support"`가 0번 나와요. trss-web도 0번이에요. 같은 명령에 `dev`를 더하면 두 크레이트의 `test-support`가 보여서, 이 명령이 기능이 켜졌을 때 그것을 보여 준다는 것도 확인했어요.
- dev 프로필의 `trss-worker`와 `trss-web` 바이너리에 가짜의 글자(`fake-session-id` 등)가 없고, worker 테스트 바이너리에는 있어요.

### trss-collect의 테스트

worker의 `archive_move.rs`에 있던 `a_transmission_that_refuses_the_move_leaves_everything_in_place`를 trss-collect의 작업 폴더 이동 테스트(`rule_archive/work_folder/tests.rs`)로 내렸어요(`test(collect): check a refused torrent-set-location of a work folder move against the fake Transmission`, 이 결과를 함께 담은 커밋). 같은 경우를 확인하는 trss-collect 테스트는 없었어요.

- 이 테스트는 Transmission이 `torrent-set-location`을 거절하면 이동이 그 까닭으로 끝나고 두 폴더가 그대로인지 확인해요. 잠금, 주기 순서, 동시 실행, 자식 프로세스, 이어 하기, 웹에서 worker까지의 명령이 없는 규칙 테스트라서 골랐어요.
- worker 쪽은 웹 API와 worker 실행을 거쳤고, trss-collect 쪽은 `move_work_folder`를 옮긴 가짜 Transmission으로 바로 불러요. 명령의 `failed` 상태 대신 `MoveError::Failed`와 그 까닭(`permission denied`)을 확인하고, 폴더 단언은 같아요.
- 거절을 흉내 내는 줄을 지우자 테스트가 `Ok(Moved)`로 실패하는 것을 확인했어요.
- `MoveError::Failed`가 명령의 `failed`와 까닭으로 이어지는 길은 다른 보관 폴더 이동 worker 테스트가 여럿 확인해요. [0101](0101-receive-line-tests.md)에서 보관 폴더 이동 테스트를 나눌 때 다시 봐요.

### 검증한 것

2026-10-08에 `cargo test --locked --workspace -j 4`로 확인했어요.

| | 시작 | `8239ab7` 뒤 | 테스트를 내린 뒤 |
| --- | --- | --- | --- |
| workspace(통과·실패·무시) | 2,848·0·14 | 2,848·0·14 | 2,848·0·14 |
| trss-worker | 428·0·0 | 428·0·0 | 427·0·0 |
| trss-collect | 496·0·1 | 496·0·1 | 497·0·1 |
| trss-worker `tests/it`의 `finished in` | 11.07 s | 11.10 s | 11.24 s |
| trss-collect 단위 테스트의 `finished in` | 0.99 s | 0.95 s | 0.95 s |
