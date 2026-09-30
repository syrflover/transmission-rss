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

- 웹 명령의 계약을 `src/store/commands/`(저장소, 마이그레이션 4), `src/web/commands_api.rs`, `src/worker/commands.rs`로 세웠어요. 다음 명령은 종류와 payload만 더하면 돼요.

  | 호출 | 응답 |
  | --- | --- |
  | `POST /api/commands` `{ id, kind, payload }` | 새로 접수하면 `202`, 같은 ID·같은 내용이면 `200`과 지금 상태. 같은 ID에 다른 내용이거나 같은 항목에 열린 명령이 있으면 `409`(`current`에 저장된 명령). 폴더·항목이 잘못이면 `400`·`404`이고 아무것도 저장하지 않아요. |
  | `GET /api/commands/{id}` | 명령과 상태, 끝났으면 `outcome{result, reason}`. 모르는 ID는 `404`(서버가 접수한 적 없음). |

  상태는 `pending → running → done | failed`예요. 접수 응답은 `pending`일 뿐이고 결과는 worker가 남긴 `outcome`과 기록 항목에서만 나와요.
  명령 ID는 브라우저가 사용자 동작마다 `crypto.getRandomValues`로 만들어요(8~64자 `[A-Za-z0-9_-]`). 저장소는 `commands(seq, id UNIQUE, kind, payload(정규 JSON), subject, state, attempts, created_at, updated_at, finished_at, outcome)`이고, 같은 ID의 접수는 한 쓰기 트랜잭션이라 동시 전달도 한 건이에요.
- worker는 3초마다(`TRSS_` 환경 변수 없이 `Worker::with_command_poll`로 조정) 열린 명령을 확인하고, 수집 주기와 같은 flock을 잡았을 때만 실행해요. 잠겨 있으면 다음 확인으로 미뤄요. 루프 훅은 `src/worker/mod.rs`의 두 번째 ticker 하나예요.
  실행 중이던 worker가 죽으면 다음 worker가 `running` 명령을 다시 집어요(최대 5번). 이때 Transmission이 이미 받았으면 `중복`으로 끝나 두 번 넣지 않아요.
- `한 번 받기`(`receive_once`) 실행: 채널 기본 폴더 아래로 폴더를 다시 확인(절대 경로·`..`·링크로 벗어남 거부)하고, 원래 링크를 되살려 채널 라벨로 넣은 뒤 결과를 기록 항목(`record_outcome`, 규칙 없음)과 명령에 함께 남겨요. 저장 폴더가 `Title/Season NN`이면 회차 변환 없이(`starts_episode_at = 0`) trname 이름으로 바꾸고, 기본 폴더에 받으면 이름을 바꾸지 않아요(기존 삭제 후 다시 추가 경로는 쓰지 않아요).
- 링크 복원(`src/worker/commands/link.rs`): (1) 가린 자리를 채널 URL에서 같은 이름의 비밀 값으로 채워요. 채운 링크가 항목의 동일성 키와 맞는지 확인해요. (2) 가림이 남으면 채널의 지금 RSS를 읽어 같은 동일성 키의 항목 링크를 써요. (3) 못 찾으면 `추가 실패`와 "원래 링크를 되살리지 못했어요."로 시작하는 까닭을 남기고 Transmission에는 아무것도 보내지 않아요. 복원한 링크는 이력·응답·로그에 남지 않고, 로그 가림 목록에만 더해요.
- `cycle.rs`: 청소 단계가 지우지 않을 해시에, 이번 주기에 읽은 RSS에 아직 있는 항목 중 이력이 받음·중복으로 기록한 해시를 더했어요. 규칙 없이 받은 토렌트가 다음 주기에 지워지지 않아요.
- 기록 API `GET /api/history?result=a,b&channel=&after=&limit=`(기본 50, 최대 200)와 `GET /api/history/{id}`(`src/web/history_api.rs`)는 최신순 커서 페이지와 `counts`를 주고, 링크는 보내지 않아요. 열린 명령은 항목의 `command`로 실려 새로고침해도 받는 중이 이어져요.
- 화면(`web/src/screens/collect/history/`): 결과 칩(개수 포함)·채널 선택, `9월 28일 (월)` 날짜 머리, IntersectionObserver 무한 스크롤, 펼침 줄(받지 않은 까닭, 저장 폴더 칸, `한 번 받기`, `규칙 생성`)이에요. 필터는 `/collect/history?result=add_failed,duplicate&channel=<id>`로 주소에 남아요.
  저장 폴더 칸은 채널 기본 폴더를 앞에 고정해 보여주고 그 아래 경로만 적어요(비우면 기본 폴더). `규칙 생성`은 `/collect/rules/new?channel=<id>&match=<제목>`으로 이동해요.
  받는 중에는 2초마다 명령을 조회하고, 끝나면 항목을 다시 읽어 그 자리에서 바꿔요. 응답을 잃으면 같은 ID로 조회하고, 서버에 없다고 하면(404) 같은 ID·같은 내용으로만 다시 보낼 수 있어요. "확인하지 못했어요"와 "추가 실패"는 다른 문구와 색이에요.

### 검증한 것

백엔드 행은 `tests/receive_once.rs`(26개, 시험용 Transmission·RSS)로 확인했어요.

| 완료 기준 | 근거 |
| --- | --- |
| `LIAR GAME - 26`을 `LIAR GAME/Season 01`로 | `a_no_match_item_is_received_into_the_chosen_folder_by_the_worker`: 접수 뒤 worker 실행 전에는 Transmission이 비어 있고 항목도 `규칙 불일치`, 실행 뒤 그 폴더·`LIAR GAME S01E26.mkv`·봇 라벨·규칙 수 그대로. 화면은 아래 브라우저 확인. |
| 폴더를 바꾸지 않음 | `without_a_chosen_folder_it_goes_to_the_channels_base_folder` |
| `../../etc` | `a_folder_that_leaves_the_base_folder_is_refused_and_nothing_is_accepted`(접수·추가 없음), 접수 뒤 링크가 생긴 경우 `a_link_made_after_the_request_was_accepted_is_caught_when_the_worker_runs_it`, 폴더 규칙 단위 테스트(심볼릭 링크 포함) |
| 같은 ID 두 번 | `the_same_command_id_delivered_twice_adds_one_torrent_and_returns_the_result`: `torrent-add` 한 번, 두 번째와 늦은 세 번째 전달은 기존 결과 |
| 같은 ID·다른 항목 | `the_same_command_id_with_another_item_is_refused`(`409`, 첫 요청만 실행) |
| 접수 응답 전 끊김 | 서버: `after_a_lost_answer_the_command_is_looked_up_by_the_same_id`(GET으로 조회, 모르는 ID는 404). 브라우저: 응답을 버린 경우와 요청이 서버에 닿지 않은 경우 모두 POST의 ID가 같고 새 ID가 없음 |
| Transmission 멈춤 | `a_stopped_transmission_leaves_add_failed_with_a_reason`, `a_transmission_that_refuses_the_torrent_gives_its_reason`, 실패 뒤 새 명령으로 다시 받기 `a_failed_item_can_be_received_again_with_a_new_command` |
| 같은 이름 `token` 비밀 값 | `a_secret_in_a_query_of_the_channels_name_is_filled_back_from_the_channel`(복원한 링크가 Transmission에 가고 RSS를 다시 읽지 않음), `assert_secret_nowhere`가 API 본문·이력·변경 이력·명령에서 원문을 찾지 않음. 실제 `trss-worker` 프로세스 출력에서도 `a_started_worker_process_runs_an_accepted_command_and_prints_no_secret` |
| 다른 이름·경로, 지금 RSS에 있음 | `a_secret_under_another_name_is_found_in_the_current_feed`, 동일성이 링크인 경우 `a_link_whose_own_value_differs_from_the_channels_is_taken_from_the_feed_instead` |
| 지금 RSS에서 빠짐 | `when_the_item_has_left_the_feed_the_link_cannot_be_recovered`, RSS가 500인 경우 `when_the_feed_cannot_be_read_the_link_cannot_be_recovered_either`(모두 Transmission 호출 없음) |
| 1,000건 이상에서 `추가 실패` | 서버: `over_a_thousand_records_the_add_failed_filter_pages_without_jumping`(1,300건을 도착분과 함께 페이지). 브라우저: 1,306건 DB에서 `add_failed` 260건이 50건씩 6번에 중복·누락 없이 오고, 따라가던 줄의 문서 위치가 95프레임 내내 같아요. 날짜 머리 28개. |
| `규칙 생성` 후 저장 안 함 | 화면은 링크로 이동만 하고 저장 API를 부르지 않아요. 브라우저 흐름 끝에서 채널 `rule_count`가 0이에요. 0006의 화면과 이어서 열리는 것은 확인하지 못했어요(아래). |

그 밖에 확인한 것:

- 정리 단계: `a_torrent_received_by_hand_survives_the_next_cycle_while_its_item_is_in_the_feed`(두 주기를 지나도 남고, 항목이 RSS에서 빠지면 그때 지워져요), `a_torrent_transmission_already_had_is_kept_too`.
- 재시작·동시 실행: 재시작한 worker가 접수된 명령을 한 번 실행, 실행 중 죽은 명령의 재실행, 넣은 뒤 죽은 경우 토렌트 한 개(`중복`), worker 둘이 동시에 잡으면 한 쪽만 실행하고 다른 쪽은 `Busy`, 수집 주기가 잠금을 잡은 동안 대기, 돌고 있는 worker가 다음 주기를 기다리지 않고 명령을 실행.
- 저장소·API 단위 테스트(마이그레이션 4가 기존 DB의 데이터를 지키는지 포함). `cargo test --offline` 전체 통과, `cargo clippy --offline --all-targets` 경고 없음, `cargo fmt --check` 통과, `bun run build` 통과.
- 브라우저(헤드리스 Chromium, playwright-core, 로컬 `trss-web`·`trss-worker`, 시험용 RSS·Transmission 대역, 임시 DB): 44개 확인 통과. 1440·768·390·320px의 라이트·다크 모두에서 목록·펼침·긴 폴더 입력 상태에 가로 넘침 없음. `../../etc`는 문장으로 거부하고 접수하지 않음. 접수 뒤 줄이 `받는 중`이고 Transmission에 들어가기 전에는 `받음`으로 바뀌지 않음(4초 지연을 걸어 확인). 다시 불러온 화면도 진행 중 명령을 이어 보여줘요. 멈춘 Transmission은 빨간 까닭과 함께 `추가 실패`. 응답·페이지·웹/worker 로그에 비밀 값 없음.

### 검증하지 못한 것과 남은 점

- `규칙 생성`이 0006의 새 규칙 화면을 여는 것은 0006을 합치기 전이라 링크 주소(`/collect/rules/new?channel=…&match=…`)까지만 확인했어요. 합친 뒤 실제로 열리고 제목이 채워지는지 한 번 봐야 해요. 상태 판의 `실패·중복 N개` 링크는 0006의 몫이고, 이 탭은 `?result=add_failed,duplicate`를 받아 해당 칩을 눌린 상태로 열어요.
- 브라우저 확인은 Rust 대역이 아니라 같은 프로토콜의 Node 대역으로 했어요. 실제 Transmission·실제 RSS는 쓰지 않았어요.
- 저장 폴더의 링크 검사는 web/worker가 미디어 볼륨을 볼 수 있을 때만 링크를 따라가요. 볼 수 없으면 문자열 규칙(절대 경로·`..`)만 적용돼요. worker가 실행 직전에 다시 검사해요.
- 알려진 틈: Transmission에 넣은 직후 결과를 쓰기 전에 worker가 죽으면 그 명령은 `중복`으로 끝나요(토렌트는 한 개). 추가 요청이 시간 초과로 실패했는데 Transmission이 실제로는 받았다면, 다음 주기의 정리가 그 토렌트를 지울 수 있어요.
- 이력 전이 규칙상 `추가 실패`로 남은 `한 번 받기` 결과를 이후 주기가 `규칙 불일치`로 덮어쓸 수 있어요(받음·중복만 고정). 명령 기록에는 실패와 까닭이 남아요.
- 제목 검색 상자와 상태 판에서 넘어오는 추천 필터는 넣지 않았어요(완료 기준 밖).

