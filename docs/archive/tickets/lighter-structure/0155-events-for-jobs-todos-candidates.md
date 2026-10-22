# 0155 자막 작업·할 일·자막 후보·보관 파일 정리를 이벤트로 다시 읽어요

- 상태: 완료
- 출처: [구조 가볍게 하기 명세의 화면 알림](lighter-structure.md#화면-알림)
- 막는 티켓: [0154](0154-change-events-and-command-reads.md)(SSE 경로), [0164](0164-db-change-events.md)(DB 연결이 바뀐 표를 알림)

## 작업

[0154](0154-change-events-and-command-reads.md)가 만든 SSE 경로와 바뀜을 내보내는 방법으로, 명세 표의 다음 6곳을 주기 조회에서 이벤트로 옮겨요. 바뀜은 대부분 `trss-jobs`와 `trss-library` 안에서 생겨요.

[0152](0152-interrupted-jobs-stop.md)의 `중단됨`과 다시 시작도 작업의 상태가 바뀌는 것이므로 같은 이벤트가 가요. [0164](0164-db-change-events.md) 뒤에는 DB 연결이 알리므로 따로 보낼 코드가 없어요.

[0164 결과의 이벤트 수](0164-db-change-events.md#개발-환경에서-측정한-이벤트-수)처럼 이벤트는 짧게 몰려 와요. 자막 작업 하나가 같은 작업의 이벤트를 74 ms 안에 12개 보냈어요. 서버는 이벤트를 모아 보내지 않으므로, 화면은 읽는 중에 온 이벤트를 모아 그 읽기가 끝난 뒤 한 번만 더 읽어요. [0154](0154-change-events-and-command-reads.md)의 `CommandWatch`가 명령마다 그렇게 해요.

## 완료 기준

- [x] 다음 6곳은 주기 조회를 하지 않고, 이벤트를 받을 때 지금의 조회 경로로 다시 읽어요.
  - 할 일 화면의 자막 작업 목록: 자막 작업의 단계와 상태
  - 작업 상세: 그 작업의 단계, 상태, 원격 화면
  - 할 일 목록: 할 일이 생기거나 끝남
  - 메뉴의 할 일 배지: 할 일이 생기거나 끝남
  - 작품 상세의 자막 후보: 그 시즌의 후보, 후보의 작업, Anissia 새로고침
  - 보관 파일 정리: 그 작품의 정리 상태
- [x] 위의 바뀜마다 서버가 이벤트를 보내는 것을 테스트가 확인해요. 바뀜이 기능 크레이트 안에서 생기면 그 크레이트의 테스트가, 프로세스에서 생기면 `trss` 크레이트의 프로세스 테스트가 확인해요.
- [x] 한 화면에 맞는 이벤트가 몰려 와도, 읽는 중이면 그 읽기가 끝난 뒤 한 번만 더 읽는 것을 화면 테스트가 확인해요.
- [x] trss가 스스로 Anissia를 읽어 후보가 바뀐 경우에도 이벤트가 가서, 지금의 60초 조회 없이 작품 상세의 후보가 바뀌어요.
- [x] 화면 테스트와 `npm run typecheck`가 통과해요.
- [x] 같은 커밋에서 이 화면들이 상태를 주기적으로 읽는다고 적은 명세의 문장을 고쳐요.
- [x] [명세의 동작 보존 조건](lighter-structure.md#동작-보존)을 지켜요.

## 결과

2026-10-22에 여섯 곳을 주기 조회에서 이벤트로 옮겼어요. 같은 경로나 같은 캐시 키를 읽는 작품 상세의 할 일과 적용 상태, 할 일 화면의 제안도 함께 옮겼어요. 표지, 시즌 정보, 이번 주 편성, 수집 상태판은 [0156](0156-events-for-covers-schedule-status.md)이 옮겨요. 구현은 `stora:task` 에이전트가 했어요.

### 다시 읽는 방식

- `web/src/lib/rereads.ts`의 `createRereader`가 읽기를 모아요. 읽기는 한 번에 하나만 하고, 읽는 동안 온 요청은 표시만 남겨서 그 읽기가 끝난 뒤 한 번만 더 읽어요. React와 브라우저 전역 없이 써서 `node --test`로 확인해요.
- `web/src/lib/useRead.ts`가 `usePolled`를 대신해요. 첫 효과에서 이벤트를 듣기 시작한 뒤 처음 읽고, 맞는 `change`와 `ready`·`lagged`에 다시 읽어요. `web/src/screens/todo/poll.ts`는 지웠고, 캐시 키는 `keys.ts`로 옮겼어요.
- 읽기가 실패하면 보이던 값을 두고, 그 화면이 타이머로 읽던 간격 뒤에 다시 읽어요. 없는 대상(`not_found`)은 다시 읽지 않고, 탭이 숨겨진 동안에는 읽지 않아요.
- 어떤 바뀜에 다시 읽는지는 `web/src/screens/todo/listens.ts`와 `web/src/screens/library/detail/listens.ts`가 정해요. 화면의 경로가 읽는 표를 `TABLES`의 종류로 묶었고, 놓치기보다 한 번 더 읽는 쪽을 골랐어요.

### 곳마다 다시 읽게 하는 바뀜

| 곳 | 파일 | 다시 읽게 하는 바뀜 | 실패 뒤 다시 읽기 |
| --- | --- | --- | --- |
| 할 일 화면의 자막 작업 목록 | `TodoScreen.tsx` | `subtitle_job`, `library`, `artwork`, `anissia` | 2초 |
| 작업 상세 | `JobDetailScreen.tsx` | 그 작업의 `subtitle_job`·`subtitle_screen`, 그 작품의 `library`·`artwork`·`subtitle_store`·`subtitle_mapping`·`season_info`, `anissia` | 2초 |
| 할 일 목록 | `TodoScreen.tsx`, `WorkTodos.tsx` | `subtitle_job`, `subtitle_store`, `subtitle_mapping`, `library`, `artwork`, `history`, `rules`, `anissia`, `settings` | 10초 |
| 메뉴의 할 일 배지 | `app/todo-count.ts` | 할 일 목록과 같아요. 경로가 바뀔 때도 읽어요. | 15초 |
| 작품 상세의 자막 후보 | `candidates.ts` | 그 Anissia 작품의 `anissia`, 그 작품의 `season_info`·`subtitle_mapping`·`library`·`subtitle_store`, 모든 `subtitle_job`과 `command` | 새로고침 중 3초, 그 밖 60초 |
| 보관 파일 정리 | `StoredCleanup.tsx` | 정리를 기다리는 동안, 그 작품의 `subtitle_store`와 모든 `subtitle_job` | 3초 |
| 적용 상태(여섯 곳 밖) | `ApplyStatus.tsx` | 그 작업의 `subtitle_job`, 그 작품의 `library`·`subtitle_store` | 2초 |
| 할 일 화면의 제안(여섯 곳 밖) | `TodoScreen.tsx` | `rules`, `history`, `library`, `artwork`, `anissia`, `subtitle_mapping`, `collect_status`, `settings` | 10초 |

`id`가 `null`인 바뀜은 그 종류를 읽는 화면이면 늘 맞아요. 자막 후보가 모든 `command`에 읽는 것은, 새로고침 명령의 ID를 브라우저가 만들고 목록이 그 ID를 다음 읽기에서야 알려 주기 때문이에요.

### 표 하나의 분류

자막 후보 경로는 Anissia를 마지막으로 읽은 시각(`read_at`)을 `anissia_caption_poll`에서 읽어 보여 줘요. [0164](0164-db-change-events.md)는 이 표를 화면이 읽지 않는 표로 적었는데, 그 분류가 틀렸어요. 그래서 `anissia`(대상 없음)로 알리게 고쳤어요. 이 때문에 `trss`가 스스로 Anissia를 읽고 새 줄을 찾지 못해도 자막 후보가 읽은 시각을 새로 보여요.

### 테스트

서버 테스트 10개를 더했어요. 모두 `Db::open_announcing`으로 연 DB에서 실제로 바꾸는 코드를 돌리고, 화면이 듣는 종류와 ID가 알려지는지 확인해요.

| 바뀜 | 테스트 |
| --- | --- |
| 작업이 생기고 실행 중 단계와 상태가 바뀜 | `trss-jobs` `tests/it/runner.rs` `a_job_is_announced_when_it_is_made_and_at_each_change_of_its_run` |
| 작업이 `중단됨`이 되고 다시 시작함 | `trss-jobs` `tests/it/interrupted.rs` `an_interrupted_job_and_its_restart_are_announced_for_the_job` |
| 작업의 원격 화면 | `trss-jobs` `tests/it/screen.rs` `the_remote_screen_of_a_job_is_announced_when_it_is_bound_ended_and_asked_again` |
| 할 일이 생기거나 끝남 | `trss-jobs` `tests/it/todo.rs`의 `a_job_that_waits_for_a_check_raises_a_to_do_announced_for_the_job`, `a_replacement_to_decide_and_the_decision_that_ends_it_are_announced_for_the_job`, `a_receive_failure_a_rule_recorded_is_announced_as_a_history_change`, `a_video_check_the_person_answers_is_announced_for_its_work` |
| 후보의 회차 확인과 매핑 | `trss-jobs` `tests/it/follow.rs` `an_episode_check_that_arises_and_the_mapping_that_ends_it_are_announced_for_the_work` |
| 보관 파일 정리 | `trss-jobs` `tests/it/cleanup.rs` `a_clean_that_is_asked_for_and_the_removal_that_ends_it_are_announced_for_the_work` |
| `trss`가 스스로 Anissia를 읽음 | `trss-collect` `anissia/captions_tests.rs` `a_reading_is_announced_as_an_anissia_change_even_when_it_finds_no_new_line` |

여섯 곳의 바뀜은 모두 기능 크레이트 안에서 생겨서 `trss`의 프로세스 테스트는 더하지 않았어요. 웹의 다시 시작 경로와 프로세스를 시작할 때의 `중단됨`도 `trss-jobs`의 함수를 부르고, worker의 쓰기가 `GET /api/events`에 닿는 것은 0164의 프로세스 테스트가 확인해요. Anissia 테스트는 `anissia_caption_poll`을 알리지 않는 표로 되돌리면 실패하는 것을 에이전트가 봤어요.

웹에는 `web/src/lib/rereads.test.ts`의 테스트 14개를 더했어요. 읽는 동안 몰려 온 바뀜이 읽기 하나만 더 하는 것, 실패 뒤의 재시도, 없는 대상, 숨겨진 탭, 멈춘 뒤를 확인하고, 화면마다 맞는 바뀜과 맞지 않는 바뀜도 확인해요. 모으는 분기를 빼면 몰린 바뀜 263개에 263번 읽어 테스트 4개가 실패하는 것을 에이전트가 봤어요.

2026-10-22에 `cargo test --locked --workspace`가 통과 3,176개, 실패 0개, 건너뛴 것 12개였고, 테스트 바이너리의 시간 합은 44.3초였어요. 0164 뒤의 3,166개보다 10개 많아요. `cargo clippy --workspace --all-targets --all-features`에 경고가 없고, 웹의 `npm test`(300개)와 `npm run typecheck`가 통과했어요. 원본 로그는 이 PC의 `dev/local/measure/0155/`에 있어요.

### 개발 환경에서 확인

2026-10-22에 이 PC의 개발 환경(서버 DB 사본, 이 변경으로 빌드한 이미지)을 built-in browser로 열었어요.

- 교체 승인을 기다리는 작업 상세를 10초 동안 열어 두는 동안, 작업은 처음 한 번과 연결된 뒤 `ready`로 한 번, 모두 두 번 읽었어요. 예전에는 2초마다 읽었어요.
- `새 자막으로 교체`를 누르자 작업은 17 ms 만에 끝났고, 화면은 POST를 보내고 52 ms 뒤의 읽기로 새로고침 없이 `작업을 마쳤어요`를 보였어요.
- 할 일 화면을 연 탭을 그대로 두고, 다른 탭에서 `회차 확인 필요` 하나를 `확인함`으로 처리했어요. 지켜보던 탭은 그 전 16초 동안 할 일 경로들을 읽지 않다가, 할 일·개수·작업 목록·제안 경로를 한 번씩 읽었어요. 배지는 4에서 3으로, 목록의 `회차 확인 필요`는 3개에서 2개로 바뀌었어요.
- 콘솔 오류는 그 전에 연 수집 화면의 `GET /api/archive-batches/current` 404 하나였어요. 일괄 보관이 없을 때의 대답이에요.

### 동작 보존

화면에 보이는 내용과 경로, JSON은 그대로예요. 지운 테스트는 없고, 검증 표가 이름으로 가리키는 테스트도 옮기거나 지우지 않았어요. 다만 다시 읽는 때가 둘 바뀌었어요.

- 끝난 작업의 상세는 예전에는 끝난 뒤 더 읽지 않았어요. 이제는 그 작업이나 작품이 바뀌었다는 이벤트가 오면 다시 읽어요.
- 할 일 화면의 보관 제안은 피드가 4주 동안 조용했는지처럼, 시간이 지나야 달라지는 조건이 있어요. 예전에는 10초마다 읽어서 그 조건이 곧 보였어요. 이제는 수집 주기가 끝날 때(`collect_status`) 다시 읽으므로, 길게는 한 주기 뒤에 보여요. 그런 기한에 서버가 이벤트를 보내는 일은 [0156](0156-events-for-covers-schedule-status.md)의 완료 기준에 있어요.

### 확인하지 않은 것

- `useRead`와 훅의 React 연결: 자동 테스트가 없고, 개발 환경에서는 작업 상세와 할 일 화면만 봤어요.
- 자막 후보, 보관 파일 정리, 적용 상태, 작품 상세의 할 일은 개발 환경에서 보지 않았어요.
- 서버(j4105)와 reverse proxy를 거친 전달
