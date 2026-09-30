# 0004 worker가 DB 설정으로 상시 수집하고 수집 이력을 남겨요

- 상태: 완료
- 출처: [채널과 다운로드 규칙](../specs/collection.md#채널과-다운로드-규칙), [수집 이력](../specs/collection.md#수집-이력), [구현 경계와 실행 순서](../specs/web-app.md#구현-경계와-실행-순서)
- 막는 티켓: [0001](0001-rule-evaluation.md), [0002](0002-app-state-db.md)

## 작업

지금은 cron이 5분마다 컨테이너를 띄워 `CHANNELS_CONFIG_URL`의 YAML을 읽고 한 번 처리한 뒤 끝나요.
이를 상시 실행하는 `trss-worker` 바이너리로 바꿔, 5분마다 DB의 채널·규칙을 읽어 0001의 판정으로 항목을 고르고 Transmission에 넣으며, 본 모든 RSS 항목을 수집 이력에 남겨요.

- `CHANNELS_CONFIG_URL`은 더 읽지 않아요.
  Transmission 주소·속도 제한·큐 크기·다운로드 폴더는 지금처럼 환경 변수로 받아 적용해요.
- Transmission에 넣는 동작, 이미 있는 토렌트의 처리, trname 이름 변경, 피드에서 빠진 봇 라벨 토렌트의 제거는 지금 동작을 유지해요.
  보관한 규칙(`archived`)은 판정에서 빼요.
- 수집 이력은 채널, 항목 제목, 처음 본 시각, 결과(받음·규칙 불일치·제외·중복·추가 실패), 적용한 규칙, 받았으면 토렌트 해시를 남겨요.
  같은 항목을 다음 처리에서 다시 보면 새 기록을 만들지 않고 처음 본 시각을 유지하며, 결과가 바뀐 경우(예: 규칙이 새로 맞음)는 그 변화를 남겨요.
  항목의 동일성 기준(채널과 GUID 또는 링크)은 구현 시 정하고 테스트로 고정해요.
  받은 영상의 CRC32는 결과 목표 2의 영상 수정본 티켓에서 더해요.
- 비밀 값은 로그·오류·수집 이력에 원문으로 남기지 않아요.
- worker를 두 개 띄워도 같은 정기 처리가 중복으로 돌지 않아야 해요.
  배타성의 수단(DB 잠금 등)은 구현 시 정하되, 임대 시각이 지났다는 이유만으로 두 worker가 동시에 처리하지 않아요.
- 0001의 판정은 규칙을 채널 안 순번으로 가리키므로, 판정 결과를 0002의 규칙 ID로 옮겨 수집 이력에 남겨요.
  DB의 일치 문구는 제목 대기면 `NULL`이고 빈 문자열을 저장하지 않으니, `NULL`을 판정의 `None`으로 넘겨요.
- 기존 바이너리를 위한 `rss::legacy`·`Rule`·`ChannelConfig`는 cron을 걷어내는 [0009](0009-deploy-web-worker.md)까지 두고, 그때 함께 없애요.
- `추가 실패`를 할 일의 `받기 실패`로 보여주는 것은 할 일이 생기는 결과 목표 3에서 해요. 이 티켓은 기록까지예요.

## 완료 기준

격리된 검증 환경(시험용 Transmission과 고정한 RSS 표본)에서 확인해요.

| 입력·상태 | 기대 결과 |
| --- | --- |
| DB에 기존 YAML과 같은 채널·규칙, 같은 RSS 표본 | 지금 실행 파일과 같은 항목을 같은 저장 위치로 Transmission에 넣고 같은 이름으로 바꿔요. |
| 규칙에 맞지 않은 항목과 채널 제외 항목 | 받지 않고, 수집 이력에 각각 규칙 불일치·제외로 남아요. |
| 같은 RSS 표본으로 두 번 처리 | 수집 이력의 기록 수가 늘지 않고 처음 본 시각이 첫 처리 시각이에요. Transmission에도 중복으로 넣지 않아요. |
| 시험용 Transmission을 멈춘 채 처리 | 맞은 항목이 `추가 실패`로 남고 worker는 다음 주기에 계속 돌아요. |
| 처리 도중 웹에서 규칙을 고침 | 이번 처리는 시작할 때 읽은 설정으로 끝나고, 다음 처리부터 고친 규칙을 써요. |
| worker 두 개를 동시에 띄움 | 한 주기의 처리가 한 번만 일어나요. |
| 비밀로 표시한 쿼리 값이 있는 채널 | 로그와 수집 이력 어디에도 그 값의 원문이 없어요. |

## 결과

`trss-worker`가 5분마다 DB의 채널·규칙으로 수집하고, 본 모든 RSS 항목을 수집 이력에 남겨요. 실제 Transmission으로는 돌려 보지 못했고, 시험용 가짜 Transmission으로만 확인했어요.

### 구현한 것

- **구성**: `src/bin/trss-worker.rs`(진입부), `src/worker/`(주기 실행·한 번의 처리·배타 잠금·환경 변수·저장소→판정 변환·피드 읽기), `src/transmission/`(`src/main.rs`에서 옮긴 Transmission 처리), `src/store/history/`(수집 이력 저장소와 `schema.sql`).
  `src/main.rs`는 같은 함수를 `transmission` 모듈에서 불러 쓰고, 출력과 동작은 그대로예요. `src/rss/`·`src/config.rs`·`src/rule.rs`·`src/store/channels/`는 건드리지 않았어요.
- **환경 변수**: `TRSS_DB_PATH`(필수), `TRANSMISSION_URL`(필수), `DOWNLOAD_DIR`·`SPEED_LIMIT_UP`·`SPEED_LIMIT_DOWN`·`DOWNLOAD_QUEUE_SIZE`·`SEED_QUEUE_SIZE`(기존 이름 그대로, 선택), 새로 더한 `TRSS_WORKER_INTERVAL_SECS`(주기, 기본 300).
  `CHANNELS_CONFIG_URL`은 읽지 않아요. 잘못된 값은 변수 이름만 알리고 값은 찍지 않아요(`TRANSMISSION_URL`에 계정이 들어갈 수 있어서예요).
- **한 번의 처리**: 시작할 때 채널·규칙을 한 번 읽고(`list_channels_with_rules`), 보관한 규칙을 빼고 판정에 넘겨요. 판정의 규칙 번호는 저장된 규칙 ID로 되돌리고, `match_text`가 `NULL`이면 `pattern: None`으로 넘겨요.
  처리 도중 고친 설정은 다음 처리부터 쓰여요. 피드는 5개씩 읽고, 고른 항목은 최대 100개씩 Transmission에 넣어요. 항목마다 따로 태스크로 돌아 한 항목의 패닉이 worker를 멈추지 않고, 그 항목은 `추가 실패`로 남아요.
  Transmission이 멈춰 `session-set`이 실패해도 처리를 이어 가고(맞은 항목은 `추가 실패`), 다음 주기에 다시 시도해요.
- **Transmission 동작**: 추가, 이미 있는 토렌트(끝난 봇 라벨 토렌트는 정지), trname 이름 변경(파일이 하나일 때만), 피드에서 빠진 봇 라벨 토렌트 제거(파일은 유지)는 기존 코드를 그대로 옮겼어요.
- **수집 이력**(`store::history`, 마이그레이션 2):
  - 테이블은 `history_items`(항목당 한 줄), `history_changes`(결과가 바뀐 기록), `collection_cycle`(마지막 처리의 시작·끝 시각 한 줄)이에요.
  - 항목 한 줄의 열은 채널 ID, 채널 표시(가린 URL), 동일성 키, 제목, 링크(비밀 쿼리 값을 가림), 처음 본 시각, 마지막으로 본 시각, 결과, 결과가 정해진 시각, 적용한 규칙 ID, 실패 까닭, 토렌트 해시예요. 시각은 Unix 밀리초예요.
  - 결과 코드는 `received`(받음), `no_match`(규칙 불일치), `excluded`(제외), `duplicate`(중복: 규칙이 골랐지만 Transmission에 이미 있음), `add_failed`(추가 실패)예요. `result` 열에는 CHECK를 두지 않아 뒤 티켓의 `버전 미상`을 마이그레이션 없이 더할 수 있어요. 모르는 코드는 읽을 때 거부해요.
  - 채널·규칙에는 외래 키를 걸지 않아 채널·규칙을 지워도 이력이 남아요.
  - 동일성 키는 채널 안에서 GUID(`guid:`), 없으면 링크(`link:`), 둘 다 없으면 제목(`title:`)이에요. 출처 접두사가 키에 들어가고, 값은 손대지 않고 그대로 써요(비밀 쿼리 값만 가려요).
  - 같은 항목을 다시 보면 새 기록 없이 처음 본 시각을 두고 마지막으로 본 시각·제목·링크만 새로 해요. 결과가 다르면 아래 규칙으로 바꾸고 `history_changes`에 남겨요.
    `no_match`·`excluded`·`add_failed`는 가장 최근 판정을 따르고, 같은 `add_failed`가 되풀이되면 최근 까닭으로 바꿔요. `received`는 되돌리지 않아요(다음 주기의 `duplicate` 답, 다시 실패한 재시도, 규칙을 고쳐 더는 맞지 않는 경우에도 유지). `duplicate`는 `received`로만 바뀌어요.
  - 조회 API는 `HistoryStore::list(HistoryQuery { result, channel_id, after, limit })`이고, 처음 본 시각·기록 순서 기준 최신순으로 `HistoryPage { items, next }`를 돌려줘요. `next`(`HistoryCursor`, 문자열로 `시각.번호`)를 `after`에 넣으면 이어 읽고, 새 항목이 생겨도 위치가 튀지 않아요. 그 밖에 `changes(item_id)`, `last_cycle()`이 있어요.
- **배타성**은 두 겹이에요.
  1. 처리하는 동안 `<DB 경로>.worker.lock`에 OS 권고 잠금(`flock`, `File::try_lock`)을 잡아요. 잠금이 잡혀 있으면 그 주기는 건너뛰어요. 커널이 프로세스 종료 때 풀어 주므로 죽은 worker가 잠금을 남기지 않고, 시간 제한이 없어서 느린 worker가 처리하는 중에 다른 worker가 들어오는 일이 없어요. SQLite 쓰기 트랜잭션은 잡지 않아 웹이 계속 쓸 수 있어요.
  2. DB의 `collection_cycle`에 마지막 처리의 시작 시각을 남기고, 직전 시작이 주기의 절반보다 가까우면 시작하지 않아요. 잠금만 있으면 위상이 어긋난 두 worker가 번갈아 주기마다 한 번씩 돌아 처리가 두 배가 되기 때문이에요.
- **종료**: SIGTERM·SIGINT를 받으면 취소 토큰을 세워요. 주기 사이에는 바로 멈추고, 피드를 읽는 중이면 기다리지 않고 멈추며 아무것도 기록하지 않아요. Transmission에 넘긴 항목은 답을 받는 대로 항목마다 한 트랜잭션으로 기록하고, 새 항목은 시작하지 않으며 이름 변경과 피드에서 빠진 토렌트 제거는 건너뛰어요(제거는 불완전한 정보로 판단하게 되므로 건너뛰어요). 중단한 처리는 `finished_at`을 채우지 않아요.
- **비밀 값**: 로그에는 가린 URL만 찍어요(기존 바이너리의 `channel.link()`는 쓰지 않아요). 피드 요청 오류는 `without_url()`로 URL을 떼고, 오류 문구는 그 채널의 비밀 쿼리 값(원문·퍼센트 인코딩·디코딩 형태)과 `TRANSMISSION_URL`의 계정을 가리는 `Redactor`를 거쳐 로그와 `추가 실패` 까닭에 나가요. 항목 링크와 GUID는 채널의 비밀 쿼리 이름으로 가려 저장하지만, Transmission에는 원래 링크를 넘겨요.
- **Dockerfile**: 최종 이미지에 `trss-worker`도 복사해요. ENTRYPOINT는 그대로예요.

### 확인한 것

`cargo build`, `cargo test`(전부 오프라인, 실패 없음), `cargo clippy --all-targets`(경고 없음)를 돌렸어요.
가짜 Transmission JSON-RPC 서버와 고정 RSS 표본 서버를 `tests/common/mod.rs`에 만들어 `transmission-rpc` 클라이언트를 그대로 태웠어요(`session-set`·`torrent-add`·`torrent-get`·`torrent-rename-path`·`torrent-remove`·`torrent-stop`, 세션 ID 핸드셰이크 포함). 표본은 `tests/fixtures/worker_feed_a.xml`·`worker_feed_b.xml`·`worker_channels.yml`이고 토큰은 만든 값이에요.

| 완료 기준 | 테스트 |
| --- | --- |
| 기존 YAML과 같은 채널·규칙, 같은 RSS 표본 | `worker_legacy_comparison`: 실제 `transmission-rss` 실행 파일을 한쪽 가짜 Transmission에, worker를 다른 쪽에 돌려 `session-set`·`torrent-add`(주소·저장 위치·라벨)·`torrent-rename-path`·`torrent-stop`·`torrent-remove` 요청 집합과 최종 토렌트 상태를 비교해요. 추가 4·이름 변경 4·정지 1·제거 1이 같아요. |
| 규칙 불일치·제외 | `worker_cycle::a_cycle_adds_the_selected_items_and_records_every_item` |
| 같은 표본으로 두 번 처리 | `processing_the_same_feed_twice_adds_no_records_and_no_torrents`, `a_finished_bot_torrent_is_stopped_when_it_is_met_again` |
| Transmission을 멈춘 채 처리 | `a_stopped_transmission_records_add_failed_and_the_worker_carries_on`, `a_refused_torrent_is_add_failed_with_transmissions_reason`, 실제 바이너리로 `worker_process::the_worker_keeps_running_while_transmission_is_down_and_adds_when_it_returns` |
| 처리 도중 웹에서 규칙을 고침 | `edits_made_during_a_cycle_apply_from_the_next_cycle` |
| worker 두 개를 동시에 띄움 | `a_second_worker_skips_while_the_first_is_running`(처리 중 시계를 한 시간 넘겨도 두 번째는 들어오지 못해요), `workers_started_together_run_each_period_once`, 실제 프로세스 둘로 `worker_process::two_worker_processes_run_a_period_once`, 잠금 단위 시험 |
| 비밀 쿼리 값 | `worker_process::logs_and_history_never_contain_secret_query_values`(실제 바이너리의 출력 전체와 이력에 원문이 없어요. 로그에 원문을 찍게 바꿔 이 시험이 실패하는 것도 확인했어요), `worker_cycle::secret_query_values_never_reach_history`, `secrets_in_item_links_and_guids_are_masked_in_history_but_used_for_adding`, `Redactor` 단위 시험 |

그 밖에 확인한 것이에요.

- 이력: 결과 전이 표 전체, 처음 본 시각 유지, 결과 변화 기록, 채널·규칙 삭제 뒤 생존, 커서 페이징(같은 시각에 걸침·새 항목 도착·필터), 마이그레이션 1→2, 동일성 키(GUID→링크→제목, 비밀 값 가림)를 `store::history`의 단위 시험 25개로 봤어요.
- 규칙 변환: 보관한 규칙 제외, 제목 대기(`NULL`)는 아무것도 받지 않고 뒤 규칙을 가리지 않음, 판정 번호→저장 규칙 ID(보관한 규칙이 앞에 있는 경우), 잘못된 정규식 규칙을 확인했어요.
- 종료: 실제 SIGTERM으로 유휴 상태·항목 추가 중·피드 읽는 중 세 경우를 봤어요. 추가 중에는 넘긴 3개가 `받음`으로 남고 이름 변경·제거 요청이 없어요. 같은 상황을 프로세스 안에서 취소 토큰으로 재현한 시험은 다음 주기에 이름이 바뀌는 것까지 확인해요.
- 시작 실패: `TRANSMISSION_URL`·`TRSS_DB_PATH`가 없으면 변수 이름만 밝히고 끝나며 `CHANNELS_CONFIG_URL`은 요구하지 않아요.
- 통합 시험 세 묶음을 6번 되풀이해도 결과가 같았어요.

### 확인하지 못한 것

- **실제 Transmission**: 어느 시험도 실제 Transmission 데몬을 쓰지 않았어요. 가짜는 `transmission-rpc` 0.5가 보내고 읽는 형식만 흉내 내므로, 실제 응답과의 차이(예: 이름 변경 요청의 결과, 정지된 토렌트의 상태 값)는 [0009](0009-deploy-web-worker.md)에서 실제 서버로 관찰해야 해요.
- 실제 RSS 채널·사용자의 실제 채널 YAML로는 돌려 보지 않았어요. 비교 표본은 같은 구조를 흉내 낸 것이에요.
- musl 이미지 빌드(`docker build`)는 하지 않았어요. 새로 쓴 것은 표준 라이브러리의 `File::try_lock`(Rust 1.89 이상)과 이미 의존성 트리에 있던 `tokio-util`뿐이에요.
- 시각이 크게 뒤로 가는 시계, 며칠 이어지는 장시간 실행, 수천 건 이력에서의 성능은 확인하지 못했어요.
- 프로세스가 강제 종료(SIGKILL·전원 차단)될 때 Transmission이 토렌트를 받은 뒤 기록하기 전이면, 다음 처리에서 그 항목은 `받음`이 아니라 `중복`으로 남아요.

### 남은 일과 참고

- **기존 동작을 그대로 옮겼지만 문제가 될 수 있는 것**이에요. 고치지 않고 알려요.
  - 이름 변경을 처리할 때마다(이미 받은 항목이 피드에 있는 동안 5분마다) 다시 시도해요. 규칙의 `episode`가 음수이고 Transmission이 바뀐 이름을 `name`으로 돌려주면, 가짜 서버에서 `S04E38`이 다음 주기에 `S04E14`로 다시 바뀌었어요(`trname`이 이미 `SxxExx`인 이름에도 회차 변환을 적용해서예요). 실제 Transmission에서도 그런지는 확인하지 못했으니 0009에서 관찰이 필요해요.
  - 피드 하나를 읽지 못한 주기에도 다른 피드를 읽었다면, 못 읽은 채널의 봇 라벨 토렌트를 "피드에서 빠졌다"고 보고 제거해요(파일은 유지).
- **기존과 다르게 한 것**이에요. 읽은 피드가 하나도 없는 주기(채널이 없는 DB, 모든 피드 장애)에는 빠진 토렌트 제거를 하지 않아요. 기존 바이너리는 이때 봇 라벨 토렌트를 전부 제거해요. 피드 요청에 60초 제한을 두었고(기존은 무제한), HTTP 오류 응답은 읽기 실패로 다뤄요. 같은 피드 안에서 동일성 키가 같은 항목은 한 번만 처리해요.
- 재시작 직후 이전 처리가 주기의 절반보다 가깝게 시작했다면 첫 주기를 건너뛰고 다음 주기(최대 한 주기 뒤)에 돌아요.
- 이력의 `link`는 채널의 비밀 쿼리 이름이 든 링크에서 그 값을 가려 저장해요. 이런 링크(개인 트래커의 passkey 등)는 0008의 `한 번 받기`에서 저장된 링크로 다시 받을 수 없으므로, 그 경우의 처리는 0008에서 정해야 해요.
- `collection_cycle`의 시각과 `history_items.last_seen_at`은 0006의 `마지막 수집`과 현재 RSS 항목 미리보기에 쓸 수 있도록 더했어요.
- `추가 실패`의 까닭(`reason`)은 Transmission이 준 문구를 300자까지, 비밀 값을 가려 저장해요. 화면 표시는 뒤 티켓의 몫이에요.
- `docs/tickets/README.md`의 표는 이 티켓의 범위 밖이라 고치지 않았어요. 이 티켓의 상태를 `완료`로 바꿔야 해요.
