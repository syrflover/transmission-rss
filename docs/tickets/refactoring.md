# 목표 5와 6 사이의 리팩터링 명세

## 목표와 범위

trss는 결과 목표 1–4를 만들며 2주 사이에 커졌어요. 2026-10-07 `6c175f1`에서 Rust 코드는 크레이트 14개, 229,557줄이었고 테스트는 2,759개였어요.
사용자는 전체 그림, 동작 규칙, 검증 현황을 파악하기 어렵다고 했어요. 변경은 오래 걸리고, 보고를 따라가기 힘들고, 회귀는 사용자가 전해 들어야 알았어요.
2026-10-07 구조 조사에서 그 원인 가운데 코드에서 보이는 것을 찾았어요.

- 같은 개념을 여러 곳에서 따로 구현해, 같은 수정을 곳마다 따로 했어요.
- 웹 API와 화면이 기능 크레이트가 가진 규칙을 다시 계산하는 곳이 있어요.
- 테스트마다 DB의 마이그레이션 62개를 처음부터 돌리고(테스트 프로세스마다 한 번으로 줄인 것은 [0093](0093-migrated-test-db-once.md)), 같은 규칙을 여러 층에서 테스트해요.

이 리팩터링은 동작을 바꾸지 않고 코드를 정리해요. 얻으려는 결과는 다음 세 가지예요.

- 한 개념은 한 곳에 있어요.
- workspace 테스트 시간이 줄어요.
- 정리한 영역마다 명세에 요구별 검증 표가 생겨서, 무엇을 확인했는지 사용자가 볼 수 있어요.

범위는 모든 Rust 크레이트와 `web/src`이고, 영역마다 깊이를 달리해요(사용자 결정, 2026-10-07).
흩어진 개념은 모든 영역에서 효과가 큰 순서로 모아요. 모듈 구조를 다시 짜는 깊은 정리는 자주 함께 바뀌는 두 줄기에서만 해요. 수집 받기 줄기를 먼저, 자막 작업 줄기를 다음에 해요(사용자 결정, 2026-10-07).
그 밖의 영역은 나중의 변경이 그곳에 닿을 때 닿는 부분만 정리해요.

다음은 범위 밖이에요.

- 동작을 바꾸는 수정. 구조 조사에서 찾은 결함은 목표 5의 [0086](../archive/tickets/5-deployed-verification/0086-feed-redirect-referer.md)–[0091](../archive/tickets/5-deployed-verification/0091-trname-long-and-half-episodes.md)에서 따로 고쳐요. 리팩터링 중에 찾은 결함도 정리 커밋에 섞지 않고 따로 고쳐요.
- 새 기능. 앱 YAML 내보내기·가져오기는 [목표 6](README.md#6-앱-yaml-내보내기와-가져오기)이에요.
- API 타입 생성 도구. Rust 응답 구조체와 TypeScript 타입은 손으로 맞춰요([모듈 구성](../specs/web-app.md#공통-라이브러리의-모듈-구성), 사용자 결정, 2026-10-07).
- 실제 브라우저로 화면을 여는 테스트와 Docker 배포 환경의 테스트([테스트 나눔 ADR](../adr/0015-test-a-rule-once-in-its-crate.md)).

크레이트 지도([ADR 0011](../adr/0011-feature-crate-workspace.md))는 바꾸지 않는다고 가정해요. 공용 부품은 새 크레이트 대신 trss-core의 모듈로 둬요([ADR 0016](../adr/0016-shared-parts-in-core.md)). 크레이트를 나누거나 합칠 까닭이 생기면 사용자 결정을 받아요.

## 요구사항과 완료 기준

### 동작 보존

- 한 영역을 정리하는 동안 그 영역의 기존 테스트는 고치지 않고 모두 통과해요. 테스트 정리는 그 영역의 리팩터링을 마친 뒤에 해요([ADR 0015](../adr/0015-test-a-rule-once-in-its-crate.md)).
- 웹 API의 응답은 JSON 모양을 지켜요. 화면이 계산하던 규칙을 서버로 옮길 때는 응답에 필드를 더하고, 화면에 보이는 내용은 같아요.
- 저장된 값은 이전 형식도 계속 읽어요. 예를 들면 같은 파일 식별의 세 저장 형식과, 자막 회차 예외에 저장한 `n:13` 꼴 키예요.
- DB 구조는 데이터를 보존하는 마이그레이션으로만 바꾸고, 실제 서버에 올리기 전에 서버 DB 복사본에서 먼저 확인해요(사용자 결정, 2026-10-07).
- 저장 데이터, 신뢰 경계, 동시 실행을 바꾸는 정리는 독립 리뷰를 받아요. 수집 받기 줄기가 여기에 들어요.

### 잣대와 기록

- 효과는 "한 개념은 한 곳에"와 workspace 테스트 시간으로 판단해요. 변경 하나가 건드리는 범위(커밋당 크레이트와 파일 수)와 테스트 개수는 앞뒤를 기록만 해요(사용자 결정, 2026-10-07).
- 리팩터링을 시작할 때 `cargo test --workspace -j 4`의 시간, 테스트 개수, 변경 범위를 기준값으로 재서 아래 [기록](#기록)에 적어요. 명령, 빌드 상태(처음 빌드인지, 빌드 뒤 다시 돈 것인지), 커밋을 함께 적어요.
- 마지막에 같은 명령과 같은 빌드 상태로 다시 재요. 시간이 줄지 않았으면 까닭을 찾아 적어요.

### 테스트 DB를 한 번만 만들기

테스트가 새 DB를 열 때, 마이그레이션을 처음부터 돌리지 않고 테스트 프로세스마다 한 번 만든 DB를 복사해 받아요.

- 마이그레이션 자체를 확인하는 테스트는 지금처럼 처음부터 돌려요.
- 제품 코드의 DB 열기는 그대로예요. 테스트용 경로가 제품 빌드에 들어가지 않는 것을 Cargo feature 통합까지 확인해요.
- 완료는 trss-jobs, trss-web, trss-worker, trss-collect 테스트 바이너리의 실행 시간을 앞뒤로 견줘 기록한 때예요.

2026-10-07 `07c454d`의 측정이에요. 마이그레이션 62개로 DB를 여는 데 한 스레드에서 64 ms, 열두 스레드가 함께 열 때 110 ms가 걸렸어요. 만든 DB 파일을 복사해 여는 데는 1.09 ms, 메모리의 DB에서 backup으로 받는 데는 52 µs가 걸렸어요.
workspace 테스트 한 번에 새 DB를 약 1,814번 열어요. 테스트별 시간 합의 25–40%가 DB를 만드는 데 든다고 추정해요.

### SQL 문 캐시

2026-10-07 [0116](../archive/tickets/5-deployed-verification/0116-allocator-measurement.md)의 측정에서, 운영 한도로 띄운 trss-web의 user CPU 표본 중 8–10%가 SQL을 해석하는 함수였어요.
같은 날 `tests/` 폴더와 `_tests.rs` 파일을 빼고 `prepare`를 부르는 곳이 175곳이었고, `prepare_cached`를 쓰는 곳은 없었어요. 그래서 요청마다 같은 SQL을 다시 해석했어요. [0117](0117-sql-statement-cache.md)에서 고쳤어요.

- 되풀이해 쓰는 SQL 문은 연결의 문 캐시(`prepare_cached`)로 받아요. 캐시 크기는 쓰는 문의 수에 맞춰요.
- 결과와 동작은 그대로예요. 마이그레이션이 스키마를 바꾼 뒤에 캐시한 문이 맞게 다시 준비되는지 확인해요.
- 효과는 0116과 같은 web 부하에서 trss-web의 CPU 시간과, workspace 테스트 시간으로 앞뒤를 견줘요.

리팩터링 계획에 넣는 것은 사용자 결정이에요(2026-10-07).

### 흩어진 개념 모으기

다음 개념은 각각 한 곳에만 구현해요. 행마다 완료는 예전 복사본이 없어지고 모든 호출이 그 한 곳을 쓰며, 그 행의 "지킬 것"을 지킨 때예요.
"지킬 것"은 호출하는 쪽의 기존 테스트를 고치지 않고 통과시켜 확인하고, 모은 부품에는 그 부품을 가진 크레이트의 테스트를 둬요.

| 개념 | 지금 | 둘 곳 | 지킬 것 |
| --- | --- | --- | --- |
| 같은 파일 알아보기 | trss-jobs의 `area`, trss-collect의 `revision`과 `revisions`, trss-library의 표지, 모두 6곳이었어요. [0094](0094-file-identity-in-core.md)에서 trss-core의 `file_id`로 모았어요. 저장 형식은 `dev:ino`, `dev:ino:len:mtime:ctime`, 표지의 `dev`·`ino` 열이에요. | trss-core | inode만 견주는 규칙과 그 남은 위험(사용자 결정, 2026-10-06·07). 세 저장 형식을 모두 읽어요. 해시, CRC, 수정 시각 검사는 각 크레이트에 남아요. 감시 폴더 발견의 `FileIdentity`(크기와 수정 시각)는 다른 개념이라 이름만 바꿔요. |
| 회차 텍스트의 키, 정렬, `N화` 표기 | trss-subtitles, trss-collect의 Anissia 자막 목록, trss-library의 작품 상세와 목록, trss-jobs, trss-web, 모두 8곳이었어요. [0095](0095-episode-text-key-in-core.md)에서 trss-core의 `episode`로 모았어요. | trss-core | 저장된 `n:13` 꼴 키. 작품 상세와 목록이 `13.0`을 달리 다루는 차이는 맞는 쪽을 정하고, 화면이 바뀌면 따로 커밋해요. |
| 요청 간격, `429` 대기, `Retry-After`, 응답 크기 제한 | 요청 간격 저장이 AniList, Anissia, 지난 회차 검색에 3벌, `Retry-After` 읽기가 3벌, 응답 크기 제한이 5벌이었어요. [0102](0102-request-pace-in-core.md)에서 trss-core의 `pace`와 `response`로 모았어요. | trss-core([ADR 0016](../adr/0016-shared-parts-in-core.md)) | 간격 표 세 개는 그대로 둬요(마이그레이션 없음). 자막 출처의 메모리 안 간격은 다른 개념이라 남겨요. |
| worker 백그라운드 큐의 루프 | 표지, 시즌 정보, Anissia 편성, Anissia 자막 목록의 4벌이고, 잠금 파일 경로가 5곳에 있어요. [0103](0103-queue-loop-in-core.md)에서 trss-core의 `queue::Queue`와 `LockFile`로 모았어요. | trss-core | 큐마다의 간격과 재시도, [0088](../archive/tickets/5-deployed-verification/0088-queue-survives-panic.md)의 패닉 격리. |
| 파일을 안전하게 쓰기 | 임시 파일, fsync, `rename_noreplace`, 폴더 fsync를 5개 크레이트의 10곳 넘게 손으로 써요. [0104](0104-durable-file-writes-in-core.md)에서 trss-core `files`로 모았어요. | trss-core | 서버 브라우저가 다른 파일 시스템으로 옮길 때의 대비(EXDEV, linkat). 표지 파일 권한(0700·0644)은 까닭을 확인한 뒤 맞춰요. |
| 릴리스 이름과 회차 읽기 | trss-collect 안의 읽기 3곳과 trname이 따로 읽었어요. 회차 변환 계산이 3벌, trname이 쓴 이름과 시즌 폴더를 읽는 곳이 4곳이었어요. [0098](0098-release-name-reading.md)에서 모았어요. | 릴리스 이름 읽기는 trss-collect의 `release_name`. trname이 쓴 이름과 시즌 폴더 읽기는 trss-core의 `trname_names`. 받은 영상의 회차 예상은 trss-collect의 읽기로 하고, 묶음 테스트가 실제 이름과 같은지 확인해요(사용자 결정, 2026-10-08). | [0090](../archive/tickets/5-deployed-verification/0090-release-name-corpus.md)의 실제 릴리스 이름 묶음이 먼저 있어야 해요. 예제 테스트 약 35개는 묶음 테스트로 바꿔요. |
| 회차 대응의 계산과 까닭 문구 | trss-jobs의 `mapping`과, 대응한 회차를 배치 대상이나 까닭 문구로 바꾸는 5곳이었어요. trss-collect도 회차 이동을 따로 더했어요. [0105](0105-episode-mapping-in-library.md)에서 trss-library의 `mapping`과 `mapping::reason`으로 옮겼어요. | trss-library(ADR 0015가 회차 대응의 주인으로 적은 곳) | 화면 문구는 글자 그대로 둬요. |
| 웹이 다시 구현한 규칙 | worker가 살아 있는지 판정(웹의 상태와 편성 API), 할 일 집계(약 630줄), 규칙 미리보기의 분류, 폴더 검사 2벌, 날짜·분기·자막 형식·방영 시각 계산이에요. 폴더 검사는 [0096](0096-file-and-path-helpers.md)에서 trss-core의 `folder_check`로 모았어요. 할 일 집계와 화면이 다시 계산하던 변경 합계·배지는 [0106](0106-todo-aggregation-in-jobs.md)에서 trss-jobs의 `todo`로 옮겼어요. 나머지와 자막 작업 줄기의 웹 규칙 아홉 곳은 [0111](0111-web-and-screen-rules.md)에서 주인 크레이트로 옮겼어요. | 살아 있음 판정은 trss-core의 하트비트, 할 일 집계는 trss-jobs([모듈 구성](../specs/web-app.md#공통-라이브러리의-모듈-구성)의 작업 관리), 미리보기 분류는 trss-collect, 폴더 검사는 trss-core. 수정본 `다시 받기`를 trss-collect의 판정보다 먼저 거절하는 `in_place`는 그 순서가 웹의 것이라 웹에 둬요. | JSON 모양. 미리보기와 실제 처리가 같은 결과. 웹의 사전 검사는 worker의 실행 검사를 대신하지 않아요. |
| 화면이 다시 계산하는 규칙 | 변경 합계, 배지, 회차 범위 5벌, 앞자리 0 지우기 3벌, 같은 날 공유본 판정 2벌, 서버 한도 숫자예요. 변경 합계와 배지는 [0106](0106-todo-aggregation-in-jobs.md)에서, 나머지는 [0111](0111-web-and-screen-rules.md)에서 모았어요. | 서버가 계산해 보내요. 입력하는 동안의 미리보기도 서버의 미리보기 요청으로 계산해요(사용자 결정, 2026-10-09). 요일 이름 6벌과 크기 표기 2벌처럼 표기만 하는 것과, 보는 사람의 시간대를 쓰는 같은 날 공유본 판정은 화면의 한 모듈이에요(사용자 결정, 2026-10-09). | 화면에 보이는 내용. 크기 표기의 `22 KB`와 `22KB`처럼 화면이 달라지는 통일은 따로 커밋해요. |
| 작은 도우미 | 파일 존재 확인, 기록한 파일 지우기, 16진수 표기, 내려받는 중인 `.part`, 폴더 fsync, 파일 이름 다듬기 4벌, 앱 데이터 폴더 이름, 경로 정규화 3벌, `/proc/net/route` 읽기 2벌, 바쁨 응답, worker 깨우기 14곳, 같은 요청 ID 받기 4벌, 실행 task와 종료 유예 4–5벌이에요. | 쓰는 곳들의 가장 아래 크레이트 | 서버 브라우저의 파일 이름 다듬기는 trss-jobs가 다시 다듬는 두 단계 설계예요([0096](0096-file-and-path-helpers.md)에서 확인). 파일 존재 확인, 기록한 파일 지우기, 16진수 표기, `.part`, 앱 데이터 폴더 이름, 경로 정규화는 0096에서 모았어요. 같은 요청 ID 받기는 [0107](0107-split-job-store.md)에서 작업 만들기 3벌을 trss-jobs의 함수 하나로 모았고, 명령 큐의 것은 키와 견주는 것이 달라 따로 둬요(사용자 결정, 2026-10-09). 실행 task와 종료 유예는 [0108](0108-split-job-runner.md)에서 동작이 같은 수집 주기와 자막 작업 실행만 trss-worker의 함수 하나로 모았어요. `/proc/net/route` 읽기, 바쁨 응답, worker 깨우기는 [0112](0112-small-helpers-and-test-helpers.md)에서 trss-core의 `net_route`, trss-web의 `ApiError` 변환, trss-core의 `wake_worker_if`로 모았어요. 표를 읽지 못할 때 웹은 시작하지 않고 송신 프록시는 빈 표로 보는 차이는 그대로예요. |

### 수집 받기 줄기의 재구성

2026-10-07에는 worker의 수집 주기(`cycle.rs`)와 trss-collect의 한 번 받기 명령(`receive_once.rs`)이 토렌트를 더하고, 기록하고, 이름을 바꾸는 일을 각자 했어요.
이름 바꾸기와 그 재시도도 trss-transmission과 한 번 받기에 따로 있었고, 같은 실패를 한쪽은 영어로, 다른 쪽은 한국어로 적었어요.
이력 전체에서 `cycle.rs`를 바꾼 커밋 49개 중 22개, `receive_once.rs`를 바꾼 커밋 52개 중 29개가 수정 커밋이에요. [ADR 0015](../adr/0015-test-a-rule-once-in-its-crate.md)에 적은, 다시 받은 토렌트의 이름을 58분 사이에 네 번 고친 일도 여기서 났어요.
수집 판단(`process_job`, `withhold`, `start_replacement`, `make_plans`)이 worker에 남아 있어서, worker에는 실행 루프, 명령 분배, 주기만 남긴다는 [ADR 0011](../adr/0011-feature-crate-workspace.md)과 어긋났어요.
[0099](0099-one-receive-path.md)에서 trss-collect의 `receive`(더하기, 기록, 이름 바꾸기)와 `cycle`(수집 판단)로 모았어요.

- 항목 하나를 더하고, 기록하고, 이름을 바꾸는 일은 trss-collect의 한 경로가 맡고, 수집 주기와 한 번 받기 명령이 그 경로를 불러요.
- 이름 바꾸기와 재시도는 한 곳에 있어요. trss-transmission에는 RPC 호출과, 호출하는 쪽이 넘긴 이름 계산만 남아요.
- worker에는 실행 루프, 명령 분배, 주기만 남아요.
- 명령 표의 한 번 받기 전용 칸(`add_unconfirmed`, `original_name`)이 지금 뜻을 지켜요. 수집 주기가 그 칸을 쓰지 않는 지금 동작도 그대로예요.
- 같은 실패를 다르게 적는 두 문구는, [문구와 표기](../specs/web-app.md#문구와-표기)를 따르지 않는 쪽을 고치는 결함 수정으로 따로 커밋해요.
- 위의 "릴리스 이름과 회차 읽기"를 이 줄기에서 함께 모아요.

worker 잠금은 멈춘 worker가 마지막 하트비트 뒤에 따로 띄운 task에서 풀어요. 타이밍 때문에 흔들린 테스트를 고친 커밋 네 개(`396145e`, `756ecdc`, `89eeb6c`, `bd9f274`)가 이 비동기 해제에서 비롯됐어요.
따로 띄운 task에서 푸는 것은 패닉이나 중단으로 끝난 경우뿐이라서, 사용자는 지금처럼 두기로 했어요([0100](0100-worker-lock-release.md), 2026-10-08). 해제를 기다리는 테스트 도우미는 worker 테스트의 `wait_lock_free` 하나예요.

테스트는 정리를 마친 뒤 [0101](0101-receive-line-tests.md)에서 ADR 0015대로 나눴어요(2026-10-09).

- Transmission 가짜 서버는 trss-worker의 테스트 폴더에만 있었어요. [0097](0097-shared-transmission-fake.md)에서 trss-transmission의 `fake`로 옮겨, trss-collect의 테스트도 써요. 피드 서버와 nyaa 검색의 가짜는 trss-collect의 `fake`에 있어요.
- 2026-10-07 worker의 해당 테스트는 한 번 받기 78개, 영상 수정본 86개, 수집 주기 41개, 영상 회차 변환 44개, 보관 폴더 이동 30개였어요. 0101에서 worker의 다섯 파일에 있던 297개 중 211개를 trss-collect와 trss-transmission으로 내리고 28개를 지워 60개가 남았어요. worker에는 잠금, 주기와 명령의 순서, 동시 실행, 중단 뒤 이어 하기, 웹에서 worker까지 이어지는 명령만 남아요.
- 조사에서는 그대로 겹치는 테스트를 30–60개, 내릴 규칙 테스트를 약 100개로 추정했어요. 테스트마다 몸통을 견주어, 남는 테스트가 확인하지 않는 입력이나 결과가 있으면 지우지 않고 내렸어요.
- 완료는 위의 요구가 코드에 있고, 기존 worker 테스트가 정리 전까지 고치지 않고 통과했고, 테스트를 나눈 뒤 [RSS 수집 명세](../specs/collection.md#검증-표)에 검증 표가 있는 때예요. 2026-10-09에 셋 다 갖춰 이 줄기를 마쳤어요. worker의 수집 영역 중 0101의 다섯 파일 밖에 있는 테스트는 [0113](0113-remaining-area-tests.md)에서 나눠요.

### 자막 작업 줄기의 재구성

크레이트 분리 뒤 코드가 바뀐 커밋 109개 중, `db.rs`와 trss-jobs의 `store.rs`가 21번, `store.rs`와 trss-web의 `jobs_api.rs`가 19번, `jobs_api.rs`와 화면의 `todo/api.ts`가 18번 함께 바뀌었어요.
2026-10-07에 `JobStore`의 공개 함수는 70개였고, 웹이 읽는 모양, 세 종류의 작업 만들기, 집기와 상태 전이, 찾기 작업, 받은 기록과 사건을 한 타입이 다 가졌어요.
`runner.rs`(2,607줄)는 작업 루프 옆에 원격 화면 돌보기 약 400줄과 받기·게시·복구 약 800줄을 함께 가졌어요.
수정 커밋은 적어요(`store.rs` 0개). 그래서 이 줄기의 목표는 회귀를 막는 것보다 기능 하나를 더할 때 건드리는 범위를 줄이는 것이에요.

- `JobStore`는 쓰임에 따라 나눠, 기능 하나가 필요한 부분만 건드려요. 나누는 선은 티켓에서 설계해요. [0107](0107-split-job-store.md)에서 웹이 읽는 `JobViews`, 작업을 만드는 `JobRequests`, worker가 쓰는 `JobRun`, 배치·교체·정리의 `place::PlaceStore`로 나눴어요(사용자 결정, 2026-10-09).
- 원격 화면 돌보기와 받기·게시·복구는 작업 루프에서 떼어 각자의 인터페이스를 가져요. 작업 루프는 차례만 정해요. [0108](0108-split-job-runner.md)에서 `Receiver`와 `ScreenTender`로 뗐어요.
- 할 일의 묶음, 정렬, 변경 합계, 배지는 trss-jobs가 계산하고, trss-web은 직렬화만 해요. 화면은 서버가 보낸 변경 합계와 배지를 보여줘요.
- 교체와 재배치가 따로 가진 "옆으로 옮기고, 확인하고, 되돌리거나 지우기"는 한 구현을 써요. 두 기능의 기록 표는 따로 둬요. [0109](0109-shared-aside-and-remove.md)에서 trss-jobs의 `place/aside.rs`로 모았어요.
- trss-jobs 테스트 파일 18개가 각자 가진 시계 도우미와 19개가 각자 가진 준비 함수는 하나로 모아요. [0110](0110-job-line-tests.md)에서 `tests/it/world.rs`로 모았어요.
- 실제 시간을 기다리는 원격 화면과 찾기 테스트 14개(합 49초)는 주입한 시계로 기다려요. [0110](0110-job-line-tests.md)에서 보니 기다림은 주입한 시계가 아니라 제품의 tokio 타이머가 정해서, 그 타이머를 기다리는 15개를 멈춘 tokio 시간으로 기다리게 했어요(합 45.8초에서 0.5초로).
- 완료는 위의 요구가 코드에 있고, 테스트를 나눈 뒤 [작업과 인증 명세](../specs/jobs.md#검증-표)와 [자막 명세](../specs/subtitles.md#검증-표)에 검증 표가 있는 때예요. 2026-10-09에 다 갖춰 이 줄기를 마쳤어요. 웹 코드에 남은 이 줄기의 규칙 아홉 곳은 [0111](0111-web-and-screen-rules.md)에서 trss-collect와 trss-jobs로 옮겼어요.

### 그 밖의 테스트 정리

- trss-web의 `folders_on_different_filesystems_are_refused`는 `/proc`를 훑어 혼자 4.9초가 걸리고, trss-collect의 같은 이름 테스트와 같은 규칙을 확인해요. 웹 쪽은 ADR 0015대로 정리해요.
- 웹 API 테스트 461개 중 70–110개가 다른 크레이트의 규칙을 다시 확인한다고 추정했어요. 그 규칙의 영역을 정리할 때 함께 나눠요.
- 가짜 AniList와 가짜 Anissia의 같은 뼈대, 웹 테스트 모듈 19곳의 요청 도우미, ZIP 만들기 4벌, 루프백 서버 띄우기 약 14곳은 하나로 모아요. [0112](0112-small-helpers-and-test-helpers.md)에서 trss-core의 `loopback`과 `fake_http`, trss-web의 `testing`, trss-archive의 `testing`(사용자 결정, 2026-10-10)으로 모았어요.
- trss-subtitles의 `testing.rs`에 섞인 테스트 20개는 도우미와 떼어요. 0112에서 `testing/tests.rs`로 옮겼어요.

### 영역별 검증 표

한 영역의 테스트를 정리할 때 그 영역 명세에 요구별 검증 표를 둬요(사용자 결정, 2026-10-07).
표는 요구마다 실제 사용에서 관찰한 것(날짜와 티켓), 테스트로 확인한 것(테스트 이름이나 파일), 확인하지 않은 것을 나눠 적어요.
완료는 여섯 명세([웹 앱 공통](../specs/web-app.md), [라이브러리와 작품](../specs/library.md), [RSS 수집](../specs/collection.md), [자막](../specs/subtitles.md), [작업과 인증](../specs/jobs.md), [설정과 이전](../specs/settings.md))에 모두 표가 있는 때예요.

### 순서

1. 기준값을 재요.
2. 테스트 DB를 한 번만 만들고, 되풀이해 쓰는 SQL 문을 캐시해요. 뒤의 모든 확인이 빨라져요. 테스트 DB는 [0093](0093-migrated-test-db-once.md)에서 기준값보다 먼저 만들었어요(사용자 요청, 2026-10-07).
3. 위험이 낮은 개념을 먼저 모아요. 같은 파일 알아보기, 회차 텍스트의 키, 작은 도우미의 일부예요.
4. 수집 받기 줄기를 재구성하고 그 테스트를 나눠요. 릴리스 이름과 회차 읽기가 여기 들어가며, [0090](../archive/tickets/5-deployed-verification/0090-release-name-corpus.md)과 [0091](../archive/tickets/5-deployed-verification/0091-trname-long-and-half-episodes.md)이 끝나 있어야 해요.
5. 요청 간격, 큐 루프, 파일 쓰기를 모아요.
6. 자막 작업 줄기를 재구성하고 그 테스트를 나눠요. 회차 대응, 할 일 집계, 화면이 다시 계산하는 규칙이 여기 들어가요.
7. 남은 웹 규칙과 작은 도우미를 모으고, 라이브러리, 설정, 웹 앱 공통 영역의 테스트를 나눠요.
8. 마지막으로 다시 재고, 정리한 릴리스를 실제 서버에 올려 목표 5의 [0083](../archive/tickets/5-deployed-verification/0083-deployed-end-to-end.md) 흐름을 한 번 더 이어 봐요. 실제 서버에 올리는 일은 사용자 승인을 받아요.

### 티켓에서 받을 사용자 결정

| 결정 | 언제 | 사용자 결정 (2026-10-08) |
| --- | --- | --- |
| fsync가 없는 이름 바꾸기와 내려받기(trss-collect의 영상 이름 바꾸기, 서버 브라우저의 내려받기, 표지 폴더 일부)에 fsync를 더할지예요. 미디어 디스크에 비용이 생겨요. | 파일 쓰기를 모을 때 | 더해요. [0104](0104-durable-file-writes-in-core.md)에서 해요. |
| 큐마다 따로 쥐는 파일 잠금을 단일 worker([ADR 0006](../adr/0006-separate-web-worker-binaries.md))에서도 남길지예요. | 큐 루프를 모을 때 | 남겨요. 두 번째 worker 프로세스도 뜨면 큐를 돌리기 때문이에요. [0103](0103-queue-loop-in-core.md)에서 해요. |
| worker 잠금을 동기로 풀지예요. 중단 뒤 웹이 바쁨으로 보이는 시간이 달라져요. | 수집 받기 줄기에서 | 지금처럼 둬요([0100](0100-worker-lock-release.md)). |
| 회차 키가 `13.0`을 13으로 볼지예요. 작품 상세와 목록이 지금 달라졌어요. | 회차 텍스트의 키를 모을 때 | 목록도 13으로 봐요([0095](0095-episode-text-key-in-core.md)). |

### 완료 판정

리팩터링은 위의 요구별 완료가 모두 이루어지고, 기준값과 마지막 측정, 변경 범위와 테스트 개수의 앞뒤가 [기록](#기록)에 있을 때 끝나요.
끝나면 결과와 남은 일을 이 문서에 적고, 이 문서와 리팩터링 티켓을 `docs/archive/tickets/` 아래의 한 폴더로 옮겨요.

## 기록

### 구조 조사(2026-10-07)

`07c454d`에서 조사했어요. 이력은 크레이트 분리를 마친 `a8e061c` 뒤의 커밋을 봤어요.

- 병합이 아닌 커밋 161개 중 Rust 크레이트나 `web/src`를 바꾼 것은 109개예요. 이 109개가 건드린 크레이트 수(화면은 하나로 셈)는 1개 42번, 2개 20번, 3개 8번, 4개 15번, 5개 11번, 6개 8번, 7개 3번, 9개와 10개가 1번씩이에요.
- trss-core를 건드린 커밋은 45개이고, 그중 35개가 마이그레이션 목록 `db.rs`예요.
- 2026-10-04부터 10-07까지의 기능·수정·성능 커밋 38개(`feat` 25개, `fix` 10개, `perf` 3개)는 평균 크레이트 3.05개, 파일 15.6개를 건드렸어요. 10-07의 `6c175f1`까지 셌고, 세는 방법은 아래 [기준값](#기준값)의 변경 범위와 같아요.
- 조사에서 찾은, 같은 수정이 여러 복사본에 따로 들어간 수정 커밋은 같은 파일 알아보기의 다섯 번(2026-10-06–07)이에요. 크레이트 분리 전인 2026-10-02에는 worker 살아 있음 판정의 수정 두 번(`24024b5`, `86189e4`)이 웹의 상태 API와 편성 API에 똑같이 들어갔어요.

크레이트별 테스트 바이너리 실행 시간이에요. 명령은 `RUSTC_BOOTSTRAP=1 cargo test -p <크레이트> -j 4 -- -Zunstable-options --report-time`이고, 시간은 cargo가 출력한 `finished in`의 합이에요. 크레이트마다 따로 돌려서 빌드 시간은 견줄 수 없고, 이것은 기준값이 아니에요.

| 크레이트 | 테스트 | 실행 시간 |
| --- | --- | --- |
| trss-jobs | 520 | 19.12초 |
| trss-web | 464 | 15.38초 |
| trss-worker | 411 | 14.98초 |
| trss-collect | 473 | 4.69초 |
| trss-browser | 127 | 4.65초 |
| trss-library | 235 | 3.00초 |
| trss-archive | 85 | 2.32초 |
| trss-core | 130 | 1.24초 |
| trss-subtitles | 160 | 1.05초 |
| 나머지 다섯 | 133 | 각 0.5초 이하 |

- 1초 넘게 걸린 테스트는 64개이고 합이 156초예요. 열두 스레드로 돈 테스트별 시간의 합 478초의 3분의 1이에요.
- 실제 `sleep`은 테스트에 142곳, 합 47.4초가 있어요. 5–25 ms짜리 짧은 기다림이 많아서 이 합은 상한이에요.

### 기준값과 마지막 측정

#### 기준값

2026-10-08에 [0092](0092-refactor-baseline.md)에서 `ff7e6df`를 측정했어요. 코드는 0127을 고친 `ef90d40`과 같고, 그 뒤의 커밋은 문서만 바꿨어요.
개발 PC(Ryzen 5 5600X, 12 스레드, RAM 32 GB, glibc)에서 측정했고, 측정을 시작할 때 다른 cargo 빌드나 테스트는 돌고 있지 않았어요. 원자료는 이 PC의 `dev/local/measure/0092/`에 있어요.

테스트 시간이에요.

- 명령은 `cargo test --locked --workspace -j 4`예요.
- 빌드 상태는 같은 명령에 `--no-run`을 붙여 한 번 빌드한 뒤, 바꾸지 않고 다시 돈 것이에요. 이 상태로 두 번 측정했어요.
- 처음 빌드는 ``Finished `test` profile [unoptimized + debuginfo] target(s) in 7.98s``였어요. 이미 있던 `target/`에서 trss-worker 하나만 다시 컴파일했고, rustc는 kache를 거쳤어요. 처음부터 빌드한 시간이 아니라서 견주는 데 쓰지 않아요.

| | 1회 | 2회 |
| --- | ---: | ---: |
| cargo의 `Finished … in` | 0.12s | 0.13s |
| 테스트 바이너리 31개의 `finished in` 합 | 56.6초 | 56.3초 |
| 걸린 시간(doc test 포함) | 59.36초 | 58.71초 |
| 통과·실패·무시 | 2,818·0·14 | 2,818·0·14 |

- 두 번의 차이는 합으로 0.5%, 걸린 시간으로 1.1%예요.
- doc test는 13개 크레이트에서 돌지만 테스트가 0개예요.
- [0093](0093-migrated-test-db-once.md)의 뒤 값(합 53.0초, 통과 2,756개)과 견주면 합이 3.6초 길고, 통과한 테스트가 62개 많아요.

테스트 바이너리마다의 `finished in`이에요. `dev/measure/test-times.py`로 모았어요.

| 바이너리 | 1회 | 2회 |
| --- | ---: | ---: |
| trss-worker `tests/it` | 11.73초 | 11.71초 |
| trss-jobs `tests/it` | 10.89초 | 10.97초 |
| trss-web lib | 7.58초 | 7.51초 |
| trss-browser `tests/it` | 3.16초 | 3.15초 |
| trss-web `tests/subtitle_upload_peak.rs` | 3.12초 | 3.02초 |
| trss-archive `tests/it` | 2.84초 | 2.95초 |
| trss-jobs `tests/extract_process.rs` | 2.72초 | 2.72초 |
| trss-library lib | 2.29초 | 2.22초 |
| trss-jobs lib | 2.07초 | 2.05초 |
| trss-collect `tests/revision_crc_peak.rs` | 2.00초 | 1.99초 |
| trss-web `tests/artwork_serving_peak.rs` | 1.50초 | 1.46초 |
| trss-browser lib | 1.50초 | 1.50초 |
| trss-subtitles lib | 1.22초 | 1.13초 |
| trss-core lib | 1.04초 | 1.05초 |
| trss-collect lib | 1.04초 | 1.03초 |
| trss-anissia lib | 0.70초 | 0.71초 |
| trss-anilist lib | 0.69초 | 0.69초 |
| trss-library `tests/artwork_header_check.rs` | 0.18초 | 0.17초 |
| trss-web `tests/artwork_upload_peak.rs` | 0.13초 | 0.11초 |
| trss-worker lib | 0.09초 | 0.09초 |
| trss-probe main | 0.08초 | 0.08초 |
| trss-import lib | 0.01초 | 0.01초 |
| 나머지 9개 | 각 0.00초 | 각 0.00초 |

나머지 9개는 trss-archive lib, trss-transmission lib, trss-subtitles `tests/it`, trss-web `tests/remote_screen_docker.rs`, trss-jobs의 `trss-extract`, trss-web·trss-worker·trss-browser의 main, trss-browser의 `fake_chromium`이에요.

변경 범위예요.

- 세는 방법은 `dev/measure/change-spread.sh <첫날> <끝날> [<rev>]`예요. 구조 조사의 스크립트를 옮긴 것이고, 조사 기간(2026-10-04–07, `6c175f1`)에 돌려 조사와 같은 값이 나오는 것을 확인했어요.
  - 병합이 아닌 `feat`·`fix`·`refactor`·`perf` 커밋을 author date로 골라요. 0092의 작업에는 "기능·수정 커밋"이라고 적었지만 조사는 `refactor`와 `perf`도 셌으므로, 견줄 수 있게 조사의 방법을 따랐어요.
  - 파일은 `docs/`와 `*.md`를 뺀, 바뀐 텍스트 파일이에요. 크레이트는 `crates/<이름>/`이고, `web/` 아래는 하나로 세요. 크레이트를 건드리지 않은 커밋도 0개로 평균에 들어가요.
- 기준값의 기간은 조사와 같은 나흘인 2026-10-05–08이고, `ff7e6df`까지 셌어요. 조사 기간과 10-05–07의 사흘이 겹쳐요.

| 기간 | 커밋 | 크레이트 | 파일 |
| --- | --- | ---: | ---: |
| 2026-10-04–07, `6c175f1`까지(구조 조사) | 38개(`feat` 25, `fix` 10, `perf` 3) | 3.05개 | 15.6개 |
| 2026-10-05–08, `ff7e6df`까지(기준값) | 49개(`feat` 25, `fix` 19, `perf` 5) | 2.45개 | 12.0개 |

이 방법은 `refactor` 커밋도 세요. 리팩터링 동안의 커밋은 대부분 `refactor`일 것이므로, 마지막 측정에서 기간을 고를 때 이 점을 함께 봐요.

#### 마지막 측정

[0114](0114-refactor-final-check.md)에서 적어요.
