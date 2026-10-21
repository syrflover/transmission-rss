# 0156 표지·시즌 정보·편성·상태판을 이벤트로 다시 읽고 남은 주기 조회를 없애요

- 상태: 완료
- 출처: [구조 가볍게 하기 명세의 화면 알림](lighter-structure.md#화면-알림)
- 막는 티켓: [0154](0154-change-events-and-command-reads.md)(SSE 경로), [0148](0148-one-instance-lock-and-cycle-stall.md)(수집 주기만 보는 멈춤 판정), [0164](0164-db-change-events.md)(DB 연결이 바뀐 표를 알림)

## 작업

[0154](0154-change-events-and-command-reads.md)가 만든 SSE 경로와 바뀜을 내보내는 방법으로, 명세 표의 남은 5곳을 주기 조회에서 이벤트로 옮겨요. 그리고 2026-10-21에 찾은 18곳 말고도 서버 상태를 주기적으로 읽는 곳이 남았는지 찾아 같은 방식으로 바꿔요.

상태판의 `stalled`는 서버가 읽는 순간의 시각으로 정해요. 멈춤은 아무 일도 일어나지 않아서 생기므로, 바뀜을 알리는 이벤트만으로는 화면에 닿지 않아요. 그래서 멈춤 판정의 기한이 지나면 서버가 이벤트를 보내요. 화면이 기한을 계산해 다시 읽게 하면 판정 규칙이 화면과 서버 두 곳에 생겨서예요. 기한은 [0148](0148-one-instance-lock-and-cycle-stall.md)이 바꾼 판정 규칙의 기한이에요.

[0164 결과의 이벤트 수](0164-db-change-events.md#개발-환경에서-측정한-이벤트-수)처럼 이벤트는 짧게 몰려 와요. 수집 주기 하나가 새 항목이 없어도 본 수집 기록 항목마다 `history` 이벤트를 보내서, 5 ms 안에 250개가 왔어요. 서버는 이벤트를 모아 보내지 않으므로, 화면은 읽는 중에 온 이벤트를 모아 그 읽기가 끝난 뒤 한 번만 더 읽어요. [0155](0155-events-for-jobs-todos-candidates.md)가 만든 `web/src/lib/useRead.ts`와 `web/src/lib/rereads.ts`가 그렇게 해요.

서버가 읽는 순간의 시각으로 정하는 상태에는 할 일 화면의 보관 제안도 있어요. 피드가 4주 동안 조용했는지처럼 시간이 지나야 달라지는 조건이 있어서, [0155](0155-events-for-jobs-todos-candidates.md)는 우선 수집 주기가 끝날 때(`collect_status`) 그 제안을 다시 읽게 했어요.

## 완료 기준

- [x] 다음 5곳은 주기 조회를 하지 않고, 이벤트를 받을 때 지금의 조회 경로로 다시 읽어요.
  - 작품 상세의 표지: 그 작품의 표지를 받음
  - 표지 고르기 대화 상자: 그 작품의 표지를 받음
  - 시즌 정보: 그 작품의 시즌 정보를 받음
  - 이번 주 편성: 수집 주기가 끝남, 회차의 영상·자막이 바뀜, 멈춤 판정의 기한이 지남
  - 수집 상태판: 수집 주기가 끝남, Transmission 상태를 읽음, 멈춤 판정의 기한이 지남
- [x] 위의 바뀜마다 서버가 이벤트를 보내는 것을 테스트가 확인해요.
- [x] 한 화면에 맞는 이벤트가 몰려 와도, 읽는 중이면 그 읽기가 끝난 뒤 한 번만 더 읽는 것을 화면 테스트가 확인해요.
- [x] 수집 주기가 멈춘 채 기한이 지나면, 화면을 다시 불러오지 않아도 편성 옆 열의 `다음` 자리와 수집 상태판의 RSS 칸에 멈춤 문구가 보여요. 가짜 시계로 기한을 넘기는 테스트가 서버의 이벤트를 확인해요.
- [x] 서버가 읽는 순간의 시각으로 정하는 상태를 찾아, 멈춤 말고도 있으면 그 기한에 이벤트를 보내요. 결과에 찾은 방법과 찾은 상태를 적어요.
- [x] 화면 코드에서 서버 상태를 주기적으로 읽는 곳을 찾아 모두 이벤트로 바꿔요. 입력이 멈추기를 기다리는 미리보기와 `몇 분 전` 같은 화면 안의 시계는 그대로예요. 결과에 찾은 방법과 찾은 곳을 적어요.
- [x] 화면 테스트와 `npm run typecheck`가 통과해요.
- [x] 같은 커밋에서 이 화면들이 상태를 주기적으로 읽는다고 적은 명세의 문장을 고쳐요.
- [x] [명세의 동작 보존 조건](lighter-structure.md#동작-보존)을 지켜요.

## 결과

2026-10-22에 다섯 곳을 주기 조회에서 이벤트로 옮겼어요. 서버 시계만으로 달라지는 상태는 쓰는 코드가 없어서 DB 연결이 알리지 못하므로, 그때를 알리는 마감 서비스(`crates/trss/src/deadlines.rs`)를 `trss` 프로세스에 더했어요. 구현은 `stora:task` 에이전트가 했어요.

### 곳마다 다시 읽게 하는 바뀜

표지와 시즌 정보는 예전에도 받는 중일 때만 주기적으로 읽었고, 이제도 그때만 이벤트를 들어요. 나머지 셋은 늘 들어요. 다시 읽는 방식은 [0155](0155-events-for-jobs-todos-candidates.md#다시-읽는-방식)가 만든 `useRead`와 `createRereader`를 써요.

| 곳 | 파일 | 다시 읽게 하는 바뀜 | 실패 뒤 다시 읽기 |
| --- | --- | --- | --- |
| 작품 상세의 표지 | `WorkDetailScreen.tsx` | 표지를 받는 중일 때, 그 작품의 `artwork` | 3초 |
| 표지 고르기 대화 상자 | `CoverDialog.tsx` | 표지를 찾거나 받는 중일 때, 그 작품의 `artwork` | 3초 |
| 시즌 정보 | `SeasonInfoSection.tsx` | 시즌 검색이 남았을 때, 그 작품의 `season_info` | 4초 |
| 이번 주 편성 | `ScheduleScreen.tsx`, `schedule/listens.ts` | `collect_status`, `library`, `rules`, `anissia`, `season_info`, `artwork`, `history`, `settings` | 30초 |
| 수집 상태판 | `StatusBoard.tsx`, `collect/status/listens.ts` | `collect_status`, `rules`, `history`, `settings` | 30초 |
| 할 일 목록과 메뉴의 할 일 개수(다섯 곳 밖) | `todo/listens.ts` | 0155의 종류에 `collect_status`를 더했어요 | 그대로 |

`collect_status`는 수집 주기와 Transmission 상태의 쓰기도 알리고, 마감 서비스의 때도 알려요. 할 일 목록에 `collect_status`를 더한 것은 `다시 받기` 제안이 토렌트 목록이 30분보다 오래되면 사라지기 때문이에요.

### 서버가 읽는 순간의 시각으로 정하는 상태

시각을 쓰는 조회 경로를 모두 읽어서 찾았어요. `status_api`, `schedule_api`, `archive_api`, `subscriptions_api`, `save_folder_api`, `in_place`, `watch_folders_api`예요. 화면이 보여 주는 상태가 아무것도 쓰지 않고 시간만 지나서 달라지는 것은 여섯이고, 마감 서비스가 그때 `collect_status`(`id`는 `null`)를 알려요. 각 때를 구하는 함수는 판정 규칙 옆에 있고, 규칙과 같은 밀리초를 가리키는지 그 크레이트의 테스트가 확인해요.

| 상태 | 달라지는 때 | 함수 |
| --- | --- | --- |
| 수집이 멈춘 것으로 침([0148](0148-one-instance-lock-and-cycle-stall.md)의 판정) | 끝나지 않은 주기가 한도를 넘기거나, 끝난 주기의 다음 확인이 간격 하나보다 늦어질 때 | `CycleState::next_change` |
| Transmission 목록을 믿지 않음 | 목록이 주기 간격 셋보다 오래될 때 | `TransmissionCounts::next_stale` |
| 토렌트 목록을 믿지 않아 `다시 받기` 제안이 사라짐 | 목록을 가져온 지 30분이 지날 때 | `in_place::listing_stale_at` |
| 편성 카드의 `방영 전`이 `영상 대기`가 됨 | 방영 시각 | `schedule_api::next_video_change` |
| 서울 날짜·주·분기와 `오늘`·`내일` | 다음 서울 자정 | `trss_core::calendar::next_day_start` |
| 규칙이 4주 조용했다고 쳐서 보관 제안이 나옴 | 규칙이 세기 시작한 시각의 4주 뒤 | `archive_api::next_quiet_deadline` |

알리지 않는 때와 그 까닭이에요.

- 4주 구간에서 항목이 빠져서 규칙이 조용해지는 때: 채널의 제목을 모두 읽어야 구할 수 있어서, 다음 수집 주기가 끝날 때의 알림에 맡겼어요.
- 앞서 간 시계가 적은 채널의 첫 읽기 시각이 지나서 규칙이 조용하지 않게 되는 때: 시계가 틀렸던 경우에만 생겨요. 리뷰에서 나왔어요.
- 보는 사람이 서울이 아닌 시간대에 있을 때 상태판 7일 막대의 날짜가 바뀌는 때: 서버는 서울 자정만 알려요.
- 감시 폴더 목록이 새로 찾은 작품으로 세는 7일이 지나는 때, 구독 목록과 저장 폴더 화면의 서울 날짜: 이 화면들은 주기 조회를 한 적이 없고 열 때 읽어요. 감시 폴더는 리뷰에서 나왔어요.
- 수집 기록 화면의 `다시 받기`: 열 때 읽고 이벤트로 다시 읽지 않아서, 토렌트 목록의 때에도 다시 읽지 않아요.

### 마감 서비스

서비스는 가장 이른 때까지 자고, DB를 짧은 주기로 읽지 않아요. 처음에 때를 구하고, 때를 옮길 수 있는 바뀜(`collect_status`, `rules`, `anissia`, `season_info`, `history`)을 들을 때와 때가 왔을 때 다시 구해요. 한 시간 넘게 자지 않고 시계를 다시 확인해서, 시계를 바꾸거나 기기가 잠든 뒤에도 한 시간 안에 알아차려요. DB를 읽지 못하면 1분 뒤 다시 시도해요.

토렌트 목록(`transmission_listing`)은 알리지 않는 표예요. 수집 주기가 목록을 쓴 뒤 주기의 끝을 쓰고 그것을 알리므로, 새 목록이 생기면 서비스는 늘 때를 다시 구해요. `TABLES`의 이 표 설명이 "지난 회차 검색만 읽는다"고 틀리게 적고 있어서 함께 고쳤어요.

### 화면에서 주기 조회를 찾은 방법

`web/src`, `index.html`, `vite.config.ts`에서 `setInterval`, `setTimeout`, `refetchInterval`, `usePolled`, `requestAnimationFrame`, `useInterval`, `visibilitychange`, `poll`, `while(`를 찾았어요. 서버 상태를 주기적으로 읽는 곳은 위의 다섯 곳뿐이었어요. 남은 타이머는 서버 상태를 주기적으로 읽지 않아요.

- 입력이 멈추기를 기다리는 미리보기
- 실패 뒤의 재시도
- 잃은 POST를 다시 보내는 것
- 원격 화면 WebSocket의 다시 연결
- `useAfterDelay`
- `ApplyStatus`의 화면 시계
- `events.ts`의 watchdog
- 애니메이션 프레임
- 지난 회차 검색의 진행: 이미 이벤트로 읽어요. `pollSearch`는 한 번 읽는 함수예요.

### 테스트

서버 테스트는 세 갈래예요.

- 때를 구하는 함수마다 판정 규칙의 경계를 그 규칙의 크레이트에서 확인해요. 테스트 이름은 [명세의 검증 표](../specs/web-app.md)의 마감 서비스 행에 있어요.
- `trss`의 `tests/it/deadlines.rs`는 멈춘 tokio 시간과 테스트가 정한 시계 위에서 서비스를 돌려요. 멈춤, 서울 자정, 토렌트 목록의 때를 알리는 것, 때를 옮기는 바뀜에 다시 구하는 것, 가장 이른 때를 고르는 것, 리뷰에서 나온 두 경우를 확인하는 테스트 7개예요. 멈춤 테스트는 알림 뒤 상태판 경로가 `cycle.stalled`를 참으로 읽는 것까지 봐요.
- 다섯 곳이 듣는 쓰기 바뀜은 0155처럼 `Db::open_announcing`으로 연 DB에서 실제로 쓰는 코드를 돌리고, 화면이 듣는 종류와 ID가 알려지는지 확인해요.

| 바뀜 | 테스트 |
| --- | --- |
| 표지를 고르고 이미지를 받음 | `trss-library` `artwork/tests.rs` `a_cover_that_is_chosen_and_the_image_that_then_arrives_are_announced_for_the_work` |
| 시즌 정보를 찾아 저장함 | `trss-library` `seasons/tests.rs` `a_season_whose_entry_is_found_and_stored_is_announced_for_the_work` |
| 수집 주기가 시작하고 끝남 | `trss-collect` `store/history/tests.rs` `a_cycle_that_begins_and_ends_is_announced_as_a_collect_status_change_without_an_id` |
| Transmission 상태를 읽음 | `trss-collect` `store/status/tests.rs` `transmissions_counts_that_are_recorded_are_announced_as_a_collect_status_change_without_an_id` |
| 회차의 자막이나 영상을 찾음 | `trss-library` `store/library/tests.rs` `a_scan_that_finds_a_video_or_a_subtitle_is_announced_for_the_work` |

에이전트가 표마다 `TABLES`에서 알리지 않는 표로 바꿔 돌렸을 때 다섯 테스트가 모두 실패하는 것을 봤어요. 마감 서비스의 알림 한 줄을 지우면 멈춤과 서울 자정 테스트가, 바뀜을 들어도 다시 구하지 않게 하면 다시 구하는 테스트가, 토렌트 목록의 때를 서비스에서 빼거나 한 밀리초 줄이면 그 테스트가 실패하는 것도 봤어요.

웹에는 `web/src/lib/rereads.test.ts`의 테스트 6개를 더하고 할 일 테스트 하나를 고쳤어요. 화면마다 맞는 바뀜과 맞지 않는 바뀜, 수집 주기의 몰린 바뀜에 편성과 상태판이 읽기 하나만 더 하는 것, 마감 서비스의 이벤트에 읽는 것, 읽는 중에 온 표지와 시즌 정보에 한 번 더 읽는 것을 확인해요.

2026-10-22에 `cargo test --locked --workspace`가 통과 3,199개, 실패 0개, 건너뛴 것 12개였고, 빌드 뒤 다시 실행한 테스트 바이너리의 시간 합은 44.35초였어요. 0155 뒤의 3,176개보다 23개 많아요. `cargo clippy --locked --workspace --all-targets --all-features`에 경고가 없고, `cargo fmt --all --check`, 웹의 `npm test`(306개)와 `npm run typecheck`가 통과했어요. 원본 로그는 이 PC의 `dev/local/measure/0156/`에 있어요.

### 개발 환경에서 확인

2026-10-22에 이 PC의 개발 환경(서버 DB 사본, 이 변경으로 빌드한 이미지)을 built-in browser로 열고, 이벤트 스트림을 `dev/measure/record-events.py`로 기록했어요. 이때의 이미지는 리뷰에서 고친 것을 담기 전의 것이에요. 리뷰에서 고친 것은 마감 서비스의 경합과 `useRead`의 늦은 답이어서, 아래 관찰과 관계가 없어요.

- 편성 화면을 보이는 채로 둔 동안 수집 주기 하나가 바뀜 254개(`history` 250개, `collect_status` 4개)를 보냈어요. 화면은 편성·상태판·할 일 개수 경로를, 주기가 시작할 때의 `collect_status`에 한 번, 끝에 몰린 바뀜에 두 번 읽었어요. 그 밖의 때에는 읽지 않았어요. 예전에는 편성과 상태판을 30초마다 읽었어요.
- 편성, 수집, 작품 상세 탭은 열 때와 `ready`에 한 번씩 읽은 뒤 다른 경로를 읽지 않았어요. 작품 상세의 작품은 표지를 받는 중이 아니어서 표지를 다시 읽지 않았어요.
- browser pane이 숨겨져 있으면 탭도 약 30초 뒤 숨겨져서, 그동안 온 주기에는 읽지 않았어요. 다시 보이자 `ready`로 편성·상태판·할 일 개수를 한 번씩 읽었어요.

원본은 이 PC의 `dev/local/measure/0156/`에 있어요.

### 검토

`stora:complex-reviewer`가 마감 서비스의 실행 순서, 때를 구하는 함수와 판정 규칙, 다시 구하게 하는 바뀜, 조회 경로의 바뀐 코드, 화면의 듣는 종류를 검토했어요. 지적 다섯 가지를 모두 고쳤어요.

- 서비스는 `now`를 읽고 때를 구한 뒤에야 잠을 시작해서, 잠이 늘 구하는 시간만큼 늦게 끝났어요. 그 사이에 때를 옮길 수 있는 바뀜이 오면, 서비스는 때가 지난 시각에서 다시 구해서 그 때를 알리지 않았어요. 멈춤의 때는 아무것도 쓰이지 않을 때 오므로 그 뒤로 멈춤 문구가 보이지 않을 수 있었어요. 때를 구하기 전부터 잠을 세고, 무엇에 깨든 다시 구하기 전에 온 때를 알리고, 때를 구하지 못해도 앞서 구한 때를 알리게 고쳤어요. 리뷰어의 재현 테스트와 오류 경로 테스트가 고치기 전 코드에서 실패하는 것을 봤어요.
- 서비스가 자기가 보낸 `collect_status`를 듣고 때를 한 번 더 구했어요. 알린 뒤 들은 것을 비워요.
- `useRead`가 `useCached`에 있던 보호를 잃었어요. 읽는 동안 화면이 그 값을 직접 고치면 늦게 온 답이 덮어썼어요. 명세의 [늦게 도착한 이전 조회 응답](../specs/web-app.md#웹-명령과-상태-갱신)에 어긋나서 되살렸고, 0155의 결함이라 0155 커밋에 고쳐 넣었어요.
- `TABLES`의 토렌트 목록 두 표 설명, 명세의 알리지 않는 때 목록을 고쳤어요.
- 이 결과를 적었어요.

### 동작 보존

경로와 JSON, 화면에 보이는 내용은 그대로예요. 지운 테스트는 없어요. 상태판 경로와 `다시 받기`의 근거를 읽는 경로가 시스템 시계 대신 `state.anissia.now()`를 쓰고, 보관 제안 경로의 읽기를 `read_inputs`로 나눴어요. 운영에서는 같은 시계이고 같은 읽기예요. 다시 읽는 때는 바뀌었어요.

- 다섯 곳은 정해진 간격 대신 바뀜과 때에 다시 읽어요. 바뀜이 없으면 읽지 않아요.
- 할 일 목록과 배지는 토렌트 목록이 30분보다 오래될 때와 수집 주기가 끝날 때도 다시 읽어요.

### 확인하지 않은 것

- 마감 서비스가 `trss` 프로세스에서 때를 알리는 것: 개발 환경을 지켜본 동안 온 때가 없었어요.
- 표지와 시즌 정보를 받는 중인 화면, 표지 고르기 대화 상자, 할 일 화면의 `다시 받기`: 개발 환경에서 그 상태를 만들지 않았어요.
- 실제 시계와 잠의 어긋남: 테스트는 멈춘 tokio 시간 위에서 자요.
- `useRead`의 늦은 답 보호: React 연결이라 자동 테스트가 없어요.
- 서버(j4105)와 reverse proxy를 거친 전달
