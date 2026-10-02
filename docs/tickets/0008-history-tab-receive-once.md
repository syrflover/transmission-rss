# 0008 수집 이력을 보고 한 번 받기를 해요

- 상태: 완료
- 출처: [수집 이력](../specs/collection.md#수집-이력), [수집 화면](../specs/collection.md#수집-화면), [웹 명령과 상태 갱신](../specs/web-app.md#웹-명령과-상태-갱신)
- 막는 티켓: [0003](0003-web-shell.md), [0004](0004-worker-collection-history.md), [0006](0006-rules-tab-preview.md)

## 작업

수집 화면의 `기록` 탭에서 수집 이력을 시간순으로 훑고, 항목이 왜 받아지지 않았는지 보고, 규칙 없이 `한 번 받기`로 받거나 그 항목에서 새 규칙을 만들어요.
`한 번 받기`는 웹이 접수하고 worker가 실행하는 첫 명령이라, [웹 명령의 계약](../specs/web-app.md#웹-명령과-상태-갱신)(명령 ID, 중복 전달, 접수와 완료의 구분)을 여기서 처음 세워요.

- 목록은 최근 순서, 결과·채널 필터, `9월 28일 (월)` 같은 날짜 머리, 무한 스크롤이에요.
  기록 줄은 일어난 일을 먼저 쓰고 덧붙이는 정보는 흐리게 옆에 둬요.
- 줄을 누르면 그 항목으로 할 수 있는 일(`한 번 받기`·`규칙 생성`)과 받지 않은 까닭을 펼쳐요.
- `규칙 생성`은 항목 제목으로 채운 새 규칙 상세(0006)를 열고, 저장하기 전에는 아무것도 바꾸지 않아요.
- `한 번 받기`는 펼친 자리에 채널 기본 폴더로 채운 저장 폴더 칸을 두고, 고른 폴더에 받아 회차 변환 없이 trname 이름을 붙여요([수집 이력](../specs/collection.md#수집-이력)).
  고를 수 있는 폴더는 그 채널의 기본 폴더 아래로 한정하고(감시 폴더가 생기는 결과 목표 2부터는 감시 폴더 아래도), 절대 경로·`..`·링크로 벗어나는 입력을 거부해요.
  명령 ID와 함께 접수하고, 접수 응답을 받음으로 표시하지 않아요.
  worker가 Transmission에 넣은 결과를 그 기록의 결과로 남기고 화면이 주기 조회로 보여줘요.
- 상태 판의 `실패·중복 N개`는 이 탭의 해당 필터로 이어져요.
- 원래 링크는 [수집 이력](../specs/collection.md#수집-이력)대로 가린 자리를 채널의 비밀 값으로 되살리고, 그래도 가린 값이 남으면 채널의 지금 RSS에서 같은 동일성 키의 항목을 찾아 얻어요(사용자 결정).
- `규칙 생성`은 [0006](0006-rules-tab-preview.md)의 규칙 상세를 열어요. 나머지는 0006과 함께 진행할 수 있고, 이 연결만 0006을 합친 뒤에 붙여요.

## 완료 기준

| 입력·상태 | 기대 결과 |
| --- | --- |
| 규칙 불일치로 남은 `LIAR GAME - 26` 항목에서 저장 폴더를 `LIAR GAME/Season 01`로 고르고 `한 번 받기` | 접수 뒤 줄에 받는 중이 보이고, worker 처리 뒤 결과가 받음으로 바뀌어요. 그 폴더에 trname 이름으로 받아지며 규칙은 생기지 않아요. |
| 저장 폴더를 바꾸지 않고 `한 번 받기` | 채널 기본 폴더에 받아져요. |
| 저장 폴더에 `../../etc` 입력 | 접수하지 않고 까닭을 문장으로 알려요. |
| 같은 명령 ID로 `한 번 받기`를 두 번 전달 | Transmission에 한 번만 넣고 두 번째 전달에 기존 결과를 돌려줘요. |
| 같은 명령 ID에 다른 항목을 붙여 전달 | 거부해요. |
| 접수 응답 전에 연결이 끊김 | 화면이 같은 명령 ID로 결과를 조회하고, 새 ID로 다시 보내지 않아요. |
| 시험용 Transmission을 멈춘 채 `한 번 받기` | 결과가 `추가 실패`와 까닭으로 남아요. |
| 채널 URL과 같은 이름(`token`)의 비밀 값이 든 링크의 항목을 `한 번 받기` | 가린 자리를 채널의 비밀 값으로 되살린 링크가 Transmission에 가고, 이력·응답·로그에는 비밀 값이 없어요. |
| 비밀 값이 다른 이름·경로에 든 링크이고 항목이 지금 RSS에 있음 | 지금 RSS에서 찾은 원래 링크로 받아요. |
| 위와 같은데 항목이 지금 RSS에서 빠짐 | `추가 실패`와 원래 링크를 되살리지 못했다는 까닭이 남아요. |
| 기록 1,000건 이상에서 결과 필터 `추가 실패` | 해당 기록만 날짜 머리와 함께 보이고, 스크롤로 이어 불러와도 위치가 튀지 않아요. |
| 항목에서 `규칙 생성` 후 저장하지 않고 닫음 | 규칙이 생기지 않아요. |

## 결과

### 구현한 것

- 웹 명령의 계약을 `crates/trss-core/src/commands/`(저장소, 마이그레이션 5), `crates/trss-web/src/commands_api.rs`, `crates/trss-worker/src/commands.rs`로 세웠어요. 다음 명령은 종류와 payload만 더하면 돼요.

  | 호출 | 응답 |
  | --- | --- |
  | `POST /api/commands` `{ id, kind, payload }` | 새로 접수하면 `202`, 같은 ID·같은 내용이면 `200`과 지금 상태. 같은 ID에 다른 내용이거나 같은 항목에 열린 명령이 있으면 `409`(`current`에 저장된 명령). 폴더·항목이 잘못이면 `400`·`404`이고 아무것도 저장하지 않아요. |
  | `GET /api/commands/{id}` | 명령과 상태, 끝났으면 `outcome{result, reason}`. 모르는 ID는 `404`(서버가 접수한 적 없음). |

  상태는 `pending → running → done | failed`예요. 접수 응답은 `pending`일 뿐이고 결과는 worker가 남긴 `outcome`과 기록 항목에서만 나와요.
  명령 ID는 브라우저가 사용자 동작마다 `crypto.getRandomValues`로 만들어요(8~64자 `[A-Za-z0-9_-]`). 저장소는 `commands(seq, id UNIQUE, kind, payload(정규 JSON), subject, state, attempts, created_at, updated_at, finished_at, outcome, add_unconfirmed)`이고(`add_unconfirmed`는 리뷰 뒤 수정의 마이그레이션 6), 같은 ID의 접수는 한 쓰기 트랜잭션이라 동시 전달도 한 건이에요.
- worker는 3초마다(`TRSS_` 환경 변수 없이 `Worker::with_command_poll`로 조정) 열린 명령을 확인하고, 수집 주기와 같은 flock을 잡았을 때만 실행해요. 잠겨 있으면 다음 확인으로 미뤄요. 루프 훅은 `crates/trss-worker/src/lib.rs`의 두 번째 ticker 하나예요.
  실행 중이던 worker가 죽으면 다음 worker가 `running` 명령을 다시 집어요(최대 5번). 이때 Transmission이 이미 받았으면 두 번 넣지 않아요. 명령의 추가가 붙인 명령 라벨로 이 명령이 넣은 토렌트임을 알아 `받음`으로 기록하고 이름을 바꿔요(아래 "명령 라벨"). `running` 명령이 남은 채 시작한 수집 주기는 빠진 토렌트를 정리하지 않아요.
- `한 번 받기`(`receive_once`) 실행: 채널 기본 폴더 아래로 폴더를 다시 확인(절대 경로·`..`·링크로 벗어남 거부)하고, 원래 링크를 되살려 채널 라벨로 넣은 뒤 결과를 기록 항목(`record_outcome`, 규칙 없음)에 남겨요. 명령의 결과와 까닭은 넣은 뒤 기록 항목에 남은 결과를 따라요(그 사이 규칙이 받았으면 `받음`).
  이 명령의 추가로 토렌트가 새로 들어갔을 때만 고른 폴더 기준으로 회차 변환 없이(`starts_episode_at = 0`) trname 이름을 붙여요. Transmission에 이미 있던 토렌트(`중복`)는 이름을 바꾸지 않고 메모도 남기지 않아요.
  trname이 이름을 만들지 못하면(사용자 결정 "원래 이름으로 둠") 규칙 경로와 달리 토렌트와 데이터를 지우지 않고 원래 이름으로 둔 채 `받음`으로 기록하고, 항목에 메모를 남겨요(`HistoryStore::note_received`, 결과 변경 기록은 남기지 않아요). 채널 기본 폴더처럼 작품·시즌 부분이 없는 폴더에 받으면 이 경우라서, 원래 이름과 "작품과 회차를 알아내지 못했다"는 메모가 남아요. 파일이 여러 개인 토렌트는 바로 그대로 두고 "파일이 여러 개인 토렌트라 이름을 바꾸지 않았어요."를 남겨요.
  이름 바꾸기와 메모가 끝난 뒤에 명령을 끝내요. 화면이 끝난 명령을 보고 항목을 다시 읽으면 메모가 함께 보여요.
- 링크 복원(`crates/trss-collect/src/commands/link.rs`): (1) 링크의 호스트가 채널 URL의 호스트와 같을 때만(대소문자 무시, 사용자 결정) 가린 자리를 채널 URL에서 같은 이름의 비밀 값으로 채워요. 채운 링크가 항목의 동일성 키와 맞는지 확인해요. 호스트가 없는 자석 링크나 다른 호스트의 링크는 채우지 않아요. (2) 가림이 남거나 채우지 않았으면 채널의 지금 RSS를 읽어 같은 동일성 키의 항목 링크를 써요. (3) 못 찾으면 `추가 실패`와 "원래 링크를 되살리지 못했어요."로 시작하는 까닭을 남기고 Transmission에는 아무것도 보내지 않아요. 복원한 링크는 이력·응답·로그에 남지 않고, 로그 가림 목록에만 더해요.
- `cycle.rs`: 청소 단계가 지우지 않을 해시에, 이번 주기에 읽은 RSS에 아직 있는 항목 중 이력이 받음·중복으로 기록한 해시를 더했어요. 규칙 없이 받은 토렌트가 다음 주기에 지워지지 않아요. 규칙이 나중에 그 항목을 고르면 `중복`이 되고, 규칙 경로는 직접 받은 토렌트의 이름을 바꾸거나 지우지 않아요(리뷰 뒤 수정).
- 기록 API `GET /api/history?result=a,b&channel=&after=&limit=`(기본 50, 최대 200)와 `GET /api/history/{id}`(`crates/trss-web/src/history_api.rs`)는 최신순 커서 페이지와 `counts`를 주고, 링크는 보내지 않아요. 열린 명령은 항목의 `command`로 실려 새로고침해도 받는 중이 이어져요.
- 화면(`web/src/screens/collect/history/`): 결과 칩(개수 포함)·채널 선택, `9월 28일 (월)` 날짜 머리, IntersectionObserver 무한 스크롤, 펼침 줄(받지 않은 까닭, 저장 폴더 칸, `한 번 받기`, `규칙 생성`)이에요. 필터는 `/collect/history?result=add_failed,duplicate&channel=<id>`로 주소에 남아요.
  저장 폴더 칸은 채널 기본 폴더를 앞에 고정해 보여주고 그 아래 경로만 적어요(비우면 기본 폴더). `규칙 생성`은 `/collect/rules/new?channel=<id>&match=<제목>`으로 이동해요.
  받는 중에는 2초마다 명령을 조회하고, 끝나면 항목을 다시 읽어 그 자리에서 바꿔요. 응답을 잃으면 같은 ID로 조회하고, 서버에 없다고 하면(404) 같은 ID·같은 내용으로만 다시 보낼 수 있어요. "확인하지 못했어요"와 "추가 실패"는 다른 문구와 색이에요.

### 검증한 것

백엔드 행은 `crates/trss-worker/tests/receive_once.rs`(리뷰 뒤 수정을 더해 38개, 시험용 Transmission·RSS)로 확인했어요.

| 완료 기준 | 근거 |
| --- | --- |
| `LIAR GAME - 26`을 `LIAR GAME/Season 01`로 | `a_no_match_item_is_received_into_the_chosen_folder_by_the_worker`: 접수 뒤 worker 실행 전에는 Transmission이 비어 있고 항목도 `규칙 불일치`, 실행 뒤 그 폴더·`LIAR GAME S01E26.mkv`·봇 라벨·규칙 수 그대로. 화면은 아래 브라우저 확인. |
| 폴더를 바꾸지 않음 | `without_a_chosen_folder_it_goes_to_the_channels_base_folder` |
| `../../etc` | `a_folder_that_leaves_the_base_folder_is_refused_and_nothing_is_accepted`(접수·추가 없음), 접수 뒤 링크가 생긴 경우 `a_link_made_after_the_request_was_accepted_is_caught_when_the_worker_runs_it`, 폴더 규칙 단위 테스트(심볼릭 링크 포함) |
| 같은 ID 두 번 | `the_same_command_id_delivered_twice_adds_one_torrent_and_returns_the_result`: `torrent-add` 한 번, 두 번째와 늦은 세 번째 전달은 기존 결과 |
| 같은 ID·다른 항목 | `the_same_command_id_with_another_item_is_refused`(`409`, 첫 요청만 실행) |
| 접수 응답 전 끊김 | 서버: `after_a_lost_answer_the_command_is_looked_up_by_the_same_id`(GET으로 조회, 모르는 ID는 404). 브라우저: 응답을 버린 경우와 요청이 서버에 닿지 않은 경우 모두 POST의 ID가 같고 새 ID가 없음 |
| Transmission 멈춤 | `a_stopped_transmission_leaves_add_failed_with_a_reason`, `a_transmission_that_refuses_the_torrent_gives_its_reason`, 실패 뒤 새 명령으로 다시 받기 `a_failed_item_can_be_received_again_with_a_new_command` |
| 같은 이름 `token` 비밀 값 | `a_secret_in_a_query_of_the_channels_name_is_filled_back_from_the_channel`(채널과 같은 호스트의 링크. 복원한 링크가 Transmission에 가고 RSS를 다시 읽지 않음), `assert_secret_nowhere`가 API 본문·이력·변경 이력·명령에서 원문을 찾지 않음. 실제 `trss-worker` 프로세스 출력에서도 `a_started_worker_process_runs_an_accepted_command_and_prints_no_secret` |
| 다른 이름·경로, 지금 RSS에 있음 | `a_secret_under_another_name_is_found_in_the_current_feed`, 동일성이 링크인 경우 `a_link_whose_own_value_differs_from_the_channels_is_taken_from_the_feed_instead`, 다른 호스트의 링크 `a_link_on_another_host_is_not_filled_with_the_channels_secret`(채널의 비밀 값이 Transmission 요청에 없음) |
| 지금 RSS에서 빠짐 | `when_the_item_has_left_the_feed_the_link_cannot_be_recovered`, RSS가 500인 경우 `when_the_feed_cannot_be_read_the_link_cannot_be_recovered_either`, 다른 호스트의 링크 `a_link_on_another_host_that_left_the_feed_is_not_received`(모두 Transmission 호출 없음) |
| 1,000건 이상에서 `추가 실패` | 서버: `over_a_thousand_records_the_add_failed_filter_pages_without_jumping`(1,300건을 도착분과 함께 페이지). 브라우저: 1,306건 DB에서 `add_failed` 260건이 50건씩 6번에 중복·누락 없이 오고, 따라가던 줄의 문서 위치가 95프레임 내내 같아요. 날짜 머리 28개. |
| `규칙 생성` 후 저장 안 함 | 화면은 링크로 이동만 하고 저장 API를 부르지 않아요. 브라우저 흐름 끝에서 채널 `rule_count`가 0이에요. 0006과 합친 뒤 브라우저에서 `&`·`?`·`#`·`%`가 든 제목의 `규칙 생성`을 누르면 새 규칙 상세가 그 채널과 제목 그대로 채워져 열리고, 미리보기가 그 항목을 `이 규칙이 받아요`로 보여줘요. |

그 밖에 확인한 것:

- 이름을 만들 수 없는 항목: `a_name_trname_cannot_derive_stays_in_transmission_with_its_data_and_is_noted`(토렌트·데이터가 남고 `torrent-remove`·이름 바꾸기 호출이 없으며 `받음`과 메모가 기록되고 다음 주기에도 남아요), 기본 폴더로 받는 테스트, 메모 저장소 테스트 `a_note_goes_only_on_a_received_item_that_has_none`. 브라우저 확인은 메모를 넣기 전에 했고, 그 뒤에는 줄의 문구 한 곳만 바꿨어요(빌드만 확인).
- 정리 단계: `a_torrent_received_by_hand_survives_the_next_cycle_while_its_item_is_in_the_feed`(두 주기를 지나도 남고, 항목이 RSS에서 빠지면 그때 지워져요), `a_torrent_transmission_already_had_is_kept_too`.
- 재시작·동시 실행: 재시작한 worker가 접수된 명령을 한 번 실행, 실행 중 죽은 명령의 재실행, 넣은 뒤 죽은 경우 토렌트 한 개(명령 라벨 뒤로 `받음`, 이름 변경), worker 둘이 동시에 잡으면 한 쪽만 실행하고 다른 쪽은 `Busy`, 수집 주기가 잠금을 잡은 동안 대기, 돌고 있는 worker가 다음 주기를 기다리지 않고 명령을 실행.
- 저장소·API 단위 테스트(명령 마이그레이션이 기존 DB의 데이터를 지키는지 포함. 0006과 합치며 상태 스냅숏이 4, 명령이 5가 됐어요). `cargo test --offline` 전체 통과, `cargo clippy --offline --all-targets` 경고 없음, `cargo fmt --check` 통과, `bun run build` 통과.
- 브라우저(헤드리스 Chromium, playwright-core, 로컬 `trss-web`·`trss-worker`, 시험용 RSS·Transmission 대역, 임시 DB): 44개 확인 통과. 1440·768·390·320px의 라이트·다크 모두에서 목록·펼침·긴 폴더 입력 상태에 가로 넘침 없음. `../../etc`는 문장으로 거부하고 접수하지 않음. 접수 뒤 줄이 `받는 중`이고 Transmission에 들어가기 전에는 `받음`으로 바뀌지 않음(4초 지연을 걸어 확인). 다시 불러온 화면도 진행 중 명령을 이어 보여줘요. 멈춘 Transmission은 빨간 까닭과 함께 `추가 실패`. 응답·페이지·웹/worker 로그에 비밀 값 없음.
- 0006과 합친 뒤(임시 DB, 로컬 `trss-web`): 상태 판의 `실패·중복 2개`를 누르면 `/collect/history?result=add_failed,duplicate`가 열리고 `추가 실패`·`중복` 두 줄만 보여요. `규칙 생성` 연결은 위 표에 적었어요.

### 검증하지 못한 것과 남은 점

- 브라우저 확인은 Rust 대역이 아니라 같은 프로토콜의 Node 대역으로 했어요. 실제 Transmission·실제 RSS는 쓰지 않았어요.
- 저장 폴더의 링크 검사는 web/worker가 미디어 볼륨을 볼 수 있을 때만 링크를 따라가요. 볼 수 없으면 문자열 규칙(절대 경로·`..`)만 적용돼요. worker가 실행 직전에 다시 검사해요.
- Transmission에 넣은 직후 결과를 쓰기 전에 worker가 죽으면 그 명령은 다시 실행돼 명령 라벨로 자기 토렌트를 알아보고 `받음`으로 끝나요(토렌트는 한 개). 처음에는 `중복`으로 끝나고 이름을 바꾸지 않는 알려진 틈이었어요(아래 "명령 라벨"). 그 사이에 도는 수집 주기는 `running` 명령이 있어 정리를 건너뛰어요.
  보낸 추가 요청이 답을 받지 못했는데 Transmission이 실제로는 받은 경우는 아래 "리뷰 뒤 수정"의 다시 보내기로 해시를 알아내요. 다섯 번째 시도까지 답이 없거나 명령 태스크가 패닉하면 해시를 모르는 채 끝나고, 다음 주기 한 번만 정리를 건너뛰어요. 그 뒤에는 항목 라벨(아래 "항목 라벨")이 항목이 피드에 있는 동안 그 토렌트를 지켜요.
- 이력 전이 규칙상 `추가 실패`로 남은 `한 번 받기` 결과를 이후 주기가 `규칙 불일치`로 덮어쓸 수 있어요(받음·중복만 고정). 명령 기록에는 실패와 까닭이 남아요.
- 제목 검색 상자와 상태 판에서 넘어오는 추천 필터는 넣지 않았어요(완료 기준 밖).

### 리뷰 뒤 수정

독립 리뷰의 지적을 고쳤고, 같은 리뷰어가 고친 부분을 다시 확인했어요(P1·링크 복원·재시작 정리는 원래 재현 시험을 다시 돌려 확인). 고친 것마다 회귀 시험을 먼저 써서 수정 전에는 실패하고 수정 뒤에는 통과하는 것을 확인했어요. 시험은 따로 적지 않으면 `crates/trss-worker/tests/receive_once.rs`에 있어요.

- **규칙 경로가 직접 받은 토렌트를 지움 (P1)**: 주기는 Transmission에 이미 있던 토렌트(`중복`)에도 이름 변경을 걸었고, 기존 `rename_torrent`는 trname이 이름을 만들지 못하면 토렌트를 데이터와 함께 지워요. 직접 받은 항목에서 만든 규칙이 그 항목을 고르면 데이터가 지워졌어요.
  이제 `rename_torrent`는 `RenameMode`를 받아요. 이번 주기가 새로 넣은 토렌트(`Added`)만 기존 동작(이름 변경, 이름을 못 만들면 데이터와 함께 제거)을 따르고, 기존 바이너리도 이 방식을 써요. 이미 있던 토렌트(`Existing`)는 지우지 않고, 이름이 아직 trname 형식이 아닐 때만 바꿔요(앞선 실행이 끝내지 못한 이름 변경). 기록에 직접 받음(규칙 없는 `받음`)으로 남은 토렌트는 이름도 바꾸지 않아요.
  기존 바이너리는 이미 이름을 바꾼 `중복` 토렌트에 회차 보정을 한 번 더 적용했어요(`Slime S04E38` → `S04E14`). 이 경우도 이제 그대로 둬요. 출처를 모르는 토렌트(전환 전 cron이 넣은 것)의 첫 이름 변경은 기존 바이너리와 같아서 `worker_legacy_comparison`이 그대로 통과해요.
  시험: `a_rule_that_later_selects_a_hand_received_item_neither_removes_nor_renames_it`, `a_rule_with_another_folder_does_not_rename_a_hand_received_file`, `worker_cycle::a_torrent_transmission_already_had_is_never_removed_by_the_renaming`, `worker_cycle::a_torrent_a_rule_received_and_named_is_not_renamed_again`, 저장소 `a_torrent_is_received_by_hand_when_a_received_item_without_a_rule_holds_it`.
- **`중복`에도 이름을 바꾸고 결과가 어긋남 (P2)**: 명령은 자기 추가가 새로 넣었을 때만 이름을 바꿔요. 명령의 결과와 까닭은 기록 항목에 남은 결과에서 나와서, 규칙이 먼저 받은 항목이 "받음"과 "이미 같은 토렌트가 있어요"를 함께 보이지 않아요. 시험: `a_command_that_meets_the_torrent_a_rule_received_after_intake_leaves_it_as_it_is`, `a_command_into_the_base_folder_puts_no_note_on_an_item_a_rule_received`, `a_command_whose_torrent_went_in_before_the_worker_died_adds_no_second_torrent`(이름 변경 없음을 더함).
- **재시작 뒤 첫 주기의 정리 (P2)**: 넣은 뒤 결과를 쓰기 전에 죽은 worker를 다시 띄우면 주기가 명령보다 먼저 돌아, 해시를 모르는 그 토렌트를 정리했어요. 이제 `Worker::tick`이 잠금을 잡은 채 `running` 명령 수를 세어 넘기고(`CommandsAtStart`), 하나라도 있으면 그 주기는 정리하지 않아요(`CycleReport::commands_running`). 시험: `a_cycle_run_while_a_command_is_left_running_removes_nothing`, 저장소 `only_started_and_unended_commands_count_as_running`.
- **시간 초과한 명령의 추가 (P2)**: 명령의 추가가 답을 받지 못하면(`AddError::Rpc`) 또는 명령 태스크가 패닉하면, 명령을 끝낼 때 `add_unconfirmed`를 남겨요(마이그레이션 6, `CommandStore::finish_with_unconfirmed_add`). 다음 주기는 직전 주기 시작 뒤에 끝난 그런 명령이 있으면 정리를 한 번 건너뛰어요(`CycleReport::commands_unconfirmed`). 시험(아래 후속에서 두 시험으로 바뀜): 저장소 `unconfirmed_adds_are_counted_from_a_point_in_time`, 마이그레이션 `database_from_before_unconfirmed_adds_keeps_its_commands_as_confirmed`.
- **이미 받은 항목에 실패로 끝남 (P2)**: 규칙이 접수 뒤에 항목을 받았거나 `중복`으로 만난 뒤 명령의 추가가 실패하면, 명령은 기록 항목의 결과로 `done`이 돼요. 화면의 `endedMessage`도 `received`·`duplicate` 결과를 실패로 보이지 않아요. 시험: `a_failed_add_for_an_item_already_held_ends_with_the_items_result`.
- **메모가 늦게 남음 (P3)**: 이름 바꾸기와 메모를 명령 태스크 안에서 마친 뒤 명령을 끝내요(`receive_once::run`). 시험: `the_command_ends_only_after_its_rename_step_and_note`(이름 변경 단계의 조회를 붙잡아 그동안 명령이 `running`인지 봐요).
- **파일이 여러 개인 토렌트 (P3)**: 잠금을 잡은 채 이름 변경 시도를 끝까지 되풀이하지 않고 바로 그대로 둬요. 메모는 "파일이 여러 개인 토렌트라 이름을 바꾸지 않았어요."예요(명세의 "이름을 바꾸지 못했다는 메모"에 맞춘 짧은 문장). 파일 수 0은 자석 링크의 메타데이터를 기다리는 중이라 계속 기다려요. 시험: `a_torrent_with_several_files_is_left_as_it_is_without_retrying`.
- **링크 복원이 다른 호스트로 비밀 값을 보냄 (P2, 사용자 결정 "같은 호스트만 채움")**: 이력은 이름으로 가리므로, 다른 호스트 링크의 자기 `token`도 가려져 채널의 `token`으로 채워졌고, GUID·제목 동일성에서는 확인할 방법이 없었어요. 이제 링크 호스트가 채널 URL 호스트와 같을 때만 채우고, 아니면 지금 RSS에서 찾아요. 시험: `a_link_on_another_host_is_not_filled_with_the_channels_secret`, `a_link_on_another_host_that_left_the_feed_is_not_received`, 같은 호스트는 `a_secret_in_a_query_of_the_channels_name_is_filled_back_from_the_channel`, 단위 시험 `only_a_link_on_the_channels_host_is_filled`. 명세도 고쳤어요.
- **이미 바꾼 이름을 다시 바꿈 (P2, 재검토에서 찾음)**: 처음에는 `Existing`의 "이미 trname 형식" 판정을 trname에 맡겼어요. trname은 이름이 폴더 제목(대소문자 구분)으로 시작하고 두 자리 회차일 때만 자기 형식으로 봐요. 그래서 규칙 폴더의 제목을 바꾼 뒤(대소문자만 바꿔도), 다른 규칙·채널의 폴더에 있는 토렌트, 회차 100 이상(`S01E105` → `S01E05`)에서 이미 바꾼 이름을 다시 바꿨어요. 고치기 전 코드도 같았으니 새로 생긴 문제는 아니었어요.
  이제 `Existing`은 Transmission이 알려 준 저장 폴더가 규칙 폴더와 같을 때만 이름을 바꾸고, 폴더의 작품 제목(대소문자 무시) 뒤에 ` S01E05.mkv`·` S01E105.mkv`·` S01E05.5.mkv`만 붙은 이름은 형식으로 봐요(`looks_renamed`). 다른 제목으로 `SxxEyy`가 붙은 릴리스 이름(`Tensura S04E62.mkv`)은 형식이 아니라 회차 보정을 거쳐 바꿔요. 다른 폴더의 토렌트는 기존 바이너리라면 이 규칙의 제목으로 이름을 바꿨을 텐데, worker는 그대로 둬요. `worker_legacy_comparison`의 미리 넣은 토렌트는 기존 바이너리가 실제로 넣는 자리인 규칙 폴더로 옮겼고, 두 쪽 요청은 여전히 같아요.
  시험(`crates/trss-worker/tests/worker_cycle.rs`): `a_named_torrent_under_a_folder_whose_case_changed_is_not_renamed`, `a_torrent_in_another_rules_folder_is_not_renamed_after_this_rule`, `a_named_torrent_with_a_three_digit_episode_is_not_renamed`(세 개 모두 수정 전 실패), 지키는 동작 `a_rename_cut_short_in_the_rules_folder_is_finished_when_the_torrent_is_met_again`, `a_release_named_like_sxxeyy_under_another_title_is_still_renamed_in_the_rules_folder`(수정 전 실패), 단위 시험 `a_trname_name_is_told_apart_from_a_release_name`.
- **답이 없던 명령의 추가를 다시 보내기 (재검토 뒤 후속)**: 위 "시간 초과한 명령의 추가"는 정리를 한 주기만 늦출 뿐이라, 규칙이 고르지 않는 직접 받은 항목의 토렌트는 그다음 주기에 Transmission에서 빠졌어요.
  이제 Transmission은 연결을 만들지 못한 추가(`AddError::Unreachable`, 요청이 가지 않음)와 보냈지만 답이 없는 추가(`AddError::Rpc`)를 나눠요. 명령은 앞의 경우 바로 `추가 실패`로 끝나요. 뒤의 경우에는 마지막 시도(`MAX_ATTEMPTS`)가 아니면 끝내지 않아요. 명령에 표시를 남기고(`CommandStore::note_unconfirmed_add`, 마이그레이션 6의 `add_unconfirmed`를 실행 중에도 씀) `running`으로 두어 다음 확인에서 다시 보내요. 그동안 도는 주기는 `running` 명령이 있어 정리하지 않아요.
  다시 보낸 추가가 `중복`과 해시를 받으면, 명령에 표시가 있고 항목이 다른 경로로 받은 상태가 아니며 토렌트가 명령이 고른 폴더에 있을 때 이 명령이 넣은 것으로 봐요. 그러면 `받음`으로 기록하고 이름을 바꾸고 메모를 남겨요. 추가 전 단계에서 거절되거나 다시 연결하지 못하면 이전 표시를 이어받아, 명령이 끝날 때 다음 주기가 정리를 건너뛰어요. 포기(`GIVEN_UP`)도 표시를 지켜요. 명령 사유 문구는 "연결하지 못했어요"와 "응답하지 않았어요"로 나뉘었어요.
  재검토에서 세 틈을 더 막았어요. (1) 답 없던 추가 뒤의 시도가 거절되면(`.torrent` 링크를 다시 가져오다 429 등) 표시가 지워져 다음 주기가 토렌트를 뺐어요. Transmission은 링크를 가져온 뒤에야 이미 있는지 알 수 있으니 거절도 무엇을 가졌는지 말해 주지 않아요. (2) 그 뒤의 시도가 연결하지 못하거나(Transmission 재시작) 추가 전 단계에서 거절되면(RSS를 잠깐 못 읽음) 명령이 끝나 두 주기 뒤 토렌트가 빠졌어요. 이제 표시가 있는 명령은 마지막 시도가 아니면 어떤 실패에도 끝나지 않고 기록 항목에 실패를 쓰지도 않아요. (3) 규칙 주기가 연결하지 못한 추가를 확인된 실패로 세어 정리를 했어요. 이제 주기는 거절 말고는 모두 불확실로 세요. 또 이미 있던 토렌트를 명령 것으로 보려면 봇 라벨도 있어야 해요(사람이 그 폴더에 넣은 토렌트의 이름을 바꾸지 않도록).
  시험: `a_command_add_that_timed_out_after_transmission_took_it_is_received_on_the_next_look`(한 번 답을 늦춘 추가가 다음 확인에서 `받음`·해시·trname 이름이 되고 두 주기 뒤에도 남음), `a_command_whose_adds_never_get_an_answer_ends_add_failed_and_holds_the_next_cleanup`(다섯 번 모두 답이 없으면 `추가 실패`와 한 번의 정리 보류). 두 시험 모두 수정 전에 실패했어요. 재검토 뒤: `a_refused_add_after_an_unanswered_one_leaves_the_command_for_the_next_look`, `a_refused_connection_after_an_unanswered_add_leaves_the_command_for_the_next_look`, `a_torrent_the_bot_did_not_add_is_not_taken_as_the_commands_own_after_an_unanswered_add`(세 시험 모두 수정 전 실패), 마지막 시도가 거절돼도 표시를 지키는 `a_last_start_refused_after_an_unanswered_add_still_holds_the_next_cleanup`(수정 전 실패). 주기가 연결 실패를 불확실로 세는 것은 가짜 Transmission이 추가만 연결을 거부하게 할 수 없어 코드로만 확인했어요. 저장소 `an_unanswered_add_noted_on_a_running_command_stays_through_the_next_start_and_a_give_up`는 실행 중인 명령을 `unconfirmed_adds_since(None)`이 끝난 명령으로 세던 것도 잡았어요(`finished_at IS NOT NULL`을 더함). 연결 실패가 바로 끝나는 것은 기존 `a_stopped_transmission_leaves_add_failed_with_a_reason`이 지켜요.
- **항목 라벨 (후속)**: 해시를 끝내 모른 토렌트는 정리 보류가 한 주기뿐이라 그다음 주기에 빠졌어요. 이제 worker는 추가할 때 봇 라벨과 함께 항목 라벨 `trss-item:<채널 ID>:<동일성 키>`를 붙이고(규칙 주기와 한 번 받기 모두), 이미 있던 봇 토렌트를 `중복`으로 만나면 라벨을 덧붙여요(`torrent-set`). 사람이 넣은 토렌트의 라벨은 바꾸지 않아요. 정리는 수집 이력의 해시 말고도, 항목 라벨이 지금 피드에 있는 항목이나 읽지 못한 채널을 가리키는 토렌트를 남겨요. 기존 바이너리는 라벨을 붙이지 않고 정리 규칙도 그대로예요. 그래서 `worker_legacy_comparison`은 두 쪽 요청을 항목 라벨만 빼고 비교하고, worker 쪽에 항목 라벨이 붙었는지 따로 확인해요.
  시험(기능과 함께 씀): `a_torrent_whose_hash_was_never_learned_stays_while_its_item_is_in_the_feed`, `worker_cycle::every_torrent_the_cycle_holds_for_an_item_says_which_item_it_is`, `worker_cycle::a_labelled_torrent_stays_while_its_item_is_in_a_feed_and_goes_after`, 단위 시험 `an_item_label_names_its_channel_and_identity_key`.
- **소용없는 재시도 (후속)**: 답 없던 추가 뒤에는 다시 해도 소용없는 실패(채널 삭제, 저장 폴더 거부, 기록에 없는 항목, 읽지 못하는 요청)도 다섯 번째 시도까지 되풀이했고, 그동안 뒤의 명령이 기다렸어요. 항목 라벨이 토렌트를 지키게 된 뒤로는 이런 실패에서 바로 `추가 실패`로 끝내고 표시를 남겨요(다음 주기 한 번은 정리하지 않아요). RSS·링크·Transmission 쪽 실패는 지나갈 수 있어 계속 다시 보내요. 시험: `a_deleted_channel_after_an_unanswered_add_ends_the_command_at_once`(수정 전 실패).
- **시도 사이에 규칙이 만난 토렌트 (후속)**: 답 없던 추가 뒤 다음 확인 전에 도는 주기에서 규칙이 그 항목을 고르면, 명령의 토렌트가 규칙의 `중복`으로 기록되고 명령도 `중복`으로 끝나 이름이 원래 이름 그대로였어요. 이제 명령에 표시가 있고, 봇 라벨이 있으며 명령의 폴더에 있는 토렌트이고, 항목이 바로 그 해시로 `중복`이면 명령 것으로 봐요. 항목은 `받음`(규칙 없음, 직접 받음)이 되고 이름을 바꿔요. 이후 주기의 규칙은 직접 받은 토렌트의 이름을 건드리지 않아요. 시험: `a_torrent_a_rule_met_between_the_starts_is_still_the_commands_own`(수정 전 실패).
- **명령 라벨 (리뷰 뒤 후속, 사용자 결정 "바꾸고, 끝나면 라벨 제거")**: 답 없던 추가 뒤 `중복`을 받은 토렌트를 명령 것으로 보는 조건(표시, 봇 라벨, 명령의 폴더, 받지 않은 항목이거나 그 해시의 `중복`)은 정황이라, 같은 폴더에 다른 항목의 봇 토렌트나 전환 전 cron의 토렌트가 같은 토렌트로 있으면 그것을 명령 것으로 봤어요. 항목 라벨은 규칙 주기가 `중복`에도 덧붙이므로 근거가 되지 못해요.
  이제 명령의 추가는 `trss-cmd:<명령 ID>` 라벨을 함께 실어요. Transmission은 `중복`으로 답한 추가의 라벨을 붙이지 않고 worker도 이 라벨은 덧붙이지 않으니, 이 라벨이 있는 토렌트만 명령 것으로 봐요. 표시 여부와 상관없이 판정해서, 넣은 뒤 결과를 쓰기 전에 worker가 죽은 경우도 이제 `받음`이 되고 이름을 바꿔요(위 알려진 틈이 닫힘). 기록을 마치면 명령 라벨을 떼요(실패하거나 그 전에 죽으면 끝난 명령을 가리키는 라벨이 남을 뿐이에요). 해시를 끝내 알지 못한 채 끝난 명령의 라벨도 남는데, 같은 항목의 뒤 명령은 그 토렌트를 자기 것으로 보지 않고 `중복`으로 기록해요(앞 명령이 넣은 것이니 맞는 기록이에요).
  이 판정은 Transmission이 `중복`으로 답한 추가의 라벨을 기존 토렌트에 적용하지 않는다는 전제에 기대요. libtransmission `rpcimpl.cc`의 `add_torrent_impl`은 `duplicate_of`가 있으면 기존 토렌트 정보만 돌려주고 끝나며, 요청의 라벨은 새 토렌트를 만들 때만 쓰여요(GitHub `main` 소스를 읽어 확인했고, 실제 Transmission으로 돌려 보지는 않았어요). 사람이 넣은 토렌트에 봇 라벨이 붙지 않는 것도 같은 전제예요.
  시험: `another_items_bot_torrent_in_the_chosen_folder_is_not_taken_as_the_commands_own`(수정 전 실패), `a_bot_torrent_without_the_commands_label_is_not_taken_as_its_own`, `a_command_whose_torrent_went_in_before_the_worker_died_adds_no_second_torrent`와 `a_cycle_run_while_a_command_is_left_running_removes_nothing`(미리 넣은 토렌트에 명령 라벨을 싣고 `받음`을 기대하도록 바꿈), `a_no_match_item_is_received_into_the_chosen_folder_by_the_worker`(추가 요청의 명령 라벨과 뗀 뒤의 라벨).
- **`한 번 받기`가 `다시 받기`로 (후속, [수집 이력](../specs/collection.md#수집-이력) 개정)**: 저장 폴더 칸이 있고 규칙 없이 아무 항목이나 받던 `한 번 받기`를, 규칙이 고르고도 추가하지 못한(`추가 실패`) 항목만 그 규칙의 저장 폴더(채널 기본 폴더와 규칙 `directory`를 이은 경로)와 회차 변환으로 다시 받는 `다시 받기`로 바꿨어요. 명령 종류 문자열 `receive_once`, 명령 ID 멱등, 답 없던 추가의 재시도, `trss-cmd:`·봇·항목 라벨, 비밀 링크 복원, 결과 기록은 그대로예요. 성공하면 항목의 규칙이 그 규칙이 되어 행에 `규칙 ‘…’`이 떠요. 앱 공통 수집 폴더는 이 작업에서 만들지 않았어요. 사용자에게 보이는 결과 이름 `받음`은 `추가함`(`직접 받음`은 `직접 추가함`, 대기 중 칩은 `추가하는 중`, 수집 현황은 `최근 7일 추가`)으로 바꿨어요.
  대상 판정은 `receive_once::retry_plan` 한 곳에 있고 웹 접수(`commands_api`), 기록 행의 `can_retry`·`retry_blocked`(`history_api`), worker의 실행 시점 재확인이 함께 써요. 결과가 `add_failed`이고 기록된 규칙이 있으며 그 규칙이 남아 있고 활성일 때만 대상이에요(`버전 미상`은 생기면 `is_retryable_result`에 더해요). 규칙이 지워졌거나 보관됐거나 규칙이 없던 옛 실패 행은 버튼 없이 까닭만 보여요. 접수 뒤 조건이 바뀌면 worker는 항목을 건드리지 않고 `추가 실패`와 까닭으로 바로 끝내고, 그 사이 이미 받거나 `중복`이 된 항목은 그 결과로 끝내요. 실패한 재시도도 규칙을 그대로 두어 다시 시도할 수 있어요(`record_outcome`이 `rule_id`를 받아요).
  요청 본문에 비지 않은 `folder`가 있으면 400이에요. 저장돼 있던 옛 명령의 payload는 `folder`가 있어도 그대로 읽히고(`skip_serializing`) `canonical()`은 `{"item_id":N}`만 써요. 같은 ID로 다시 온 요청은 접수 검사보다 먼저 저장된 명령과 `canonical()`로 견줘, 옛 탭이 그대로 다시 보낸 요청(폴더가 비었든 아니든)에도 저장된 명령을 200으로 돌려줘요. 폴더를 고르던 옛 명령을 worker가 돌릴 때는 규칙 폴더로 받지 않고 `폴더를 고르던 예전 요청이라 실행하지 않았어요…`로 항목을 그대로 둔 채 `추가 실패`로 끝내요(폴더가 빈 옛 명령은 보통 재시도로 돌아요). 접수 뒤 대상이 아니게 된 항목은 채널이 지워졌어도 같은 길로 끝나 항목의 결과를 바꾸지 않고, 앞선 시작이 넣었을 수 있는 토렌트의 `trss-cmd:` 라벨을 떼요. 입력 폴더 검증 모듈 `folder.rs`는 더 쓸 곳이 없어 지웠어요(규칙 주기와 같은 경로를 쓰므로 그 주기가 없는 심볼릭 링크·`..` 검사는 재시도에도 없어요). 규칙 폴더가 비어 있으면 규칙 주기와 같이 끝에 `/`가 붙은 채널 기본 폴더로 들어가요. 규칙 없이 받은 옛 행은 `by_hand`(`직접 추가함`)로 남고 규칙 주기가 이름을 바꾸거나 지우지 않는 보호도 그대로예요.
  시험(`crates/trss-worker/tests/receive_once.rs`): `a_failed_item_is_added_again_into_its_rules_folder_by_the_worker`, `the_rules_episode_conversion_names_the_file`, `a_retry_goes_where_the_rules_own_cycle_puts_the_same_release`, `a_rule_without_a_folder_retries_into_the_channels_base_folder`, `a_request_naming_a_folder_is_refused_and_nothing_is_accepted`, `a_command_stored_with_a_folder_before_the_change_runs_into_the_rules_folder`, `a_rule_deleted_after_the_request_ends_the_command_at_once`, `a_rule_archived_after_the_request_ends_the_command_at_once`, `an_item_that_a_later_cycle_found_no_rule_for_ends_the_command_at_once`, `a_failure_with_no_rule_recorded_is_not_offered_and_ends_the_command_at_once`, `items_no_rule_picked_and_items_transmission_holds_are_not_retried`, `a_retry_that_failed_can_be_tried_again_with_a_new_command`. 웹 접수·행: `a_request_that_names_a_folder_is_refused_and_nothing_is_stored`, `an_item_that_cannot_be_retried_is_refused_with_its_reason_and_nothing_is_stored`, `the_row_says_whether_a_failed_item_can_be_retried_and_why_not`. 저장소: `an_outcome_keeps_the_rule_it_is_given_through_failure_and_success`. 옛 payload와 대상 판정 단위 시험은 `receive_once.rs` 안에 있어요. 새 통합 시험 53개 중 20개가 이전 소스에서 실패했어요. 리뷰 뒤: API `a_legacy_command_sent_again_is_answered_with_the_stored_command`, 통합 `a_command_stored_with_an_empty_folder_before_the_change_runs_as_a_retry`, `a_command_stored_with_a_folder_before_the_change_is_not_run_into_the_rules_folder`, `an_item_no_rule_picked_is_left_as_it_is_when_its_channel_is_gone_and_the_label_comes_off`, `a_held_item_whose_channel_is_gone_ends_with_its_result_and_the_label_comes_off`(라벨 떼기와 옛 폴더 검사를 빼면 4개가 실패).
  알려진 한계: 옛 `receive_once`가 실패하며 `rule_id`를 NULL로 지운 `추가 실패` 행은 규칙을 알 수 없어 `다시 받기`를 받지 못해요(`규칙 생성`만). 되채우지 않았어요.

남긴 것:

- 다섯 번째 시도까지 해시를 알지 못한 명령의 토렌트는 항목이 피드에 있는 동안 항목 라벨로 남아요. 항목이 피드에서 빠지면 다른 봇 토렌트처럼 정리돼요(데이터는 남음).
- `Existing`은 Transmission이 알려 준 폴더를 글자 그대로 규칙 폴더와 비교해요. Transmission은 추가할 때 받은 폴더 문자열을 그대로 돌려주므로 worker가 넣은 토렌트는 맞아요. 규칙 폴더나 채널 기본 폴더를 바꾼 뒤에는 옛 폴더의 토렌트를 다른 폴더로 보고 이름을 바꾸지 않아요.
- 규칙 주기에서 `.torrent` 링크를 다시 가져오다 거절되면, 해시를 모르는 채 Transmission에 있던 그 토렌트가 정리될 수 있어요(기존 바이너리와 같은 동작, 데이터는 남음). worker가 넣었거나 한 번이라도 만난 토렌트는 항목 라벨로 남으니, 전환 전 cron이 넣고 worker가 아직 만나지 못한 토렌트만 해당해요.
- 한 토렌트를 두 항목이 함께 가리키고 한 주기에 둘 다 `중복`으로 만나면, 라벨 덧붙이기가 서로의 라벨을 덮어써 하나만 남을 수 있어요. 두 항목 모두 이력에 해시가 있어 정리되지 않고, 다음 주기에 빠진 라벨을 다시 붙여요. 사람이 그 사이에 바꾼 라벨도 같은 방식으로 덮일 수 있어요.
- `추가 실패`로 끝난 명령의 결과를 이후 주기가 `규칙 불일치`로 덮어쓰는 것은 그대로예요(위).
- 브라우저로는 다시 확인하지 않았어요. 화면 쪽 변경은 `endedMessage` 한 곳이고 `bun run build`만 확인했어요.
