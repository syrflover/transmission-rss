# 목표 5와 6 사이의 리팩터링 명세

이 리팩터링은 2026-10-10에 마쳤어요. 결과와 남은 일은 [결과](#결과)에 있어요.
이 문서는 그때까지의 기록이라 뒤의 변경에 맞춰 고치지 않아요. 지금 지키는 규칙은 명세, ADR, 코드의 모듈 문서에서 봐요.

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

- 동작을 바꾸는 수정. 구조 조사에서 찾은 결함은 목표 5의 [0086](../5-deployed-verification/0086-feed-redirect-referer.md)–[0091](../5-deployed-verification/0091-trname-long-and-half-episodes.md)에서 따로 고쳐요. 리팩터링 중에 찾은 결함도 정리 커밋에 섞지 않고 따로 고쳐요.
- 새 기능. 앱 YAML 내보내기·가져오기는 [목표 6](../../../tickets/README.md#6-앱-yaml-내보내기와-가져오기)이에요.
- API 타입 생성 도구. Rust 응답 구조체와 TypeScript 타입은 손으로 맞춰요([모듈 구성](../../../specs/web-app.md#공통-라이브러리의-모듈-구성), 사용자 결정, 2026-10-07).
- 실제 브라우저로 화면을 여는 테스트와 Docker 배포 환경의 테스트([테스트 나눔 ADR](../../../adr/0015-test-a-rule-once-in-its-crate.md)).

크레이트 지도([ADR 0011](../../../adr/0011-feature-crate-workspace.md))는 바꾸지 않는다고 가정해요. 공용 부품은 새 크레이트 대신 trss-core의 모듈로 둬요([ADR 0016](../../../adr/0016-shared-parts-in-core.md)). 크레이트를 나누거나 합칠 까닭이 생기면 사용자 결정을 받아요.

## 요구사항과 완료 기준

### 동작 보존

- 한 영역을 정리하는 동안 그 영역의 기존 테스트는 고치지 않고 모두 통과해요. 테스트 정리는 그 영역의 리팩터링을 마친 뒤에 해요([ADR 0015](../../../adr/0015-test-a-rule-once-in-its-crate.md)).
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

2026-10-07 [0116](../5-deployed-verification/0116-allocator-measurement.md)의 측정에서, 운영 한도로 띄운 trss-web의 user CPU 표본 중 8–10%가 SQL을 해석하는 함수였어요.
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
| 요청 간격, `429` 대기, `Retry-After`, 응답 크기 제한 | 요청 간격 저장이 AniList, Anissia, 지난 회차 검색에 3벌, `Retry-After` 읽기가 3벌, 응답 크기 제한이 5벌이었어요. [0102](0102-request-pace-in-core.md)에서 trss-core의 `pace`와 `response`로 모았어요. | trss-core([ADR 0016](../../../adr/0016-shared-parts-in-core.md)) | 간격 표 세 개는 그대로 둬요(마이그레이션 없음). 자막 출처의 메모리 안 간격은 다른 개념이라 남겨요. |
| worker 백그라운드 큐의 루프 | 표지, 시즌 정보, Anissia 편성, Anissia 자막 목록의 4벌이고, 잠금 파일 경로가 5곳에 있어요. [0103](0103-queue-loop-in-core.md)에서 trss-core의 `queue::Queue`와 `LockFile`로 모았어요. | trss-core | 큐마다의 간격과 재시도, [0088](../5-deployed-verification/0088-queue-survives-panic.md)의 패닉 격리. |
| 파일을 안전하게 쓰기 | 임시 파일, fsync, `rename_noreplace`, 폴더 fsync를 5개 크레이트의 10곳 넘게 손으로 써요. [0104](0104-durable-file-writes-in-core.md)에서 trss-core `files`로 모았어요. | trss-core | 서버 브라우저가 다른 파일 시스템으로 옮길 때의 대비(EXDEV, linkat). 표지 파일 권한(0700·0644)은 까닭을 확인한 뒤 맞춰요. |
| 릴리스 이름과 회차 읽기 | trss-collect 안의 읽기 3곳과 trname이 따로 읽었어요. 회차 변환 계산이 3벌, trname이 쓴 이름과 시즌 폴더를 읽는 곳이 4곳이었어요. [0098](0098-release-name-reading.md)에서 모았어요. | 릴리스 이름 읽기는 trss-collect의 `release_name`. trname이 쓴 이름과 시즌 폴더 읽기는 trss-core의 `trname_names`. 받은 영상의 회차 예상은 trss-collect의 읽기로 하고, 묶음 테스트가 실제 이름과 같은지 확인해요(사용자 결정, 2026-10-08). | [0090](../5-deployed-verification/0090-release-name-corpus.md)의 실제 릴리스 이름 묶음이 먼저 있어야 해요. 예제 테스트 약 35개는 묶음 테스트로 바꿔요. |
| 회차 대응의 계산과 까닭 문구 | trss-jobs의 `mapping`과, 대응한 회차를 배치 대상이나 까닭 문구로 바꾸는 5곳이었어요. trss-collect도 회차 이동을 따로 더했어요. [0105](0105-episode-mapping-in-library.md)에서 trss-library의 `mapping`과 `mapping::reason`으로 옮겼어요. | trss-library(ADR 0015가 회차 대응의 주인으로 적은 곳) | 화면 문구는 글자 그대로 둬요. |
| 웹이 다시 구현한 규칙 | worker가 살아 있는지 판정(웹의 상태와 편성 API), 할 일 집계(약 630줄), 규칙 미리보기의 분류, 폴더 검사 2벌, 날짜·분기·자막 형식·방영 시각 계산이에요. 폴더 검사는 [0096](0096-file-and-path-helpers.md)에서 trss-core의 `folder_check`로 모았어요. 할 일 집계와 화면이 다시 계산하던 변경 합계·배지는 [0106](0106-todo-aggregation-in-jobs.md)에서 trss-jobs의 `todo`로 옮겼어요. 나머지와 자막 작업 줄기의 웹 규칙 아홉 곳은 [0111](0111-web-and-screen-rules.md)에서 주인 크레이트로 옮겼어요. | 살아 있음 판정은 trss-core의 하트비트, 할 일 집계는 trss-jobs([모듈 구성](../../../specs/web-app.md#공통-라이브러리의-모듈-구성)의 작업 관리), 미리보기 분류는 trss-collect, 폴더 검사는 trss-core. 수정본 `다시 받기`를 trss-collect의 판정보다 먼저 거절하는 `in_place`는 그 순서가 웹의 것이라 웹에 둬요. | JSON 모양. 미리보기와 실제 처리가 같은 결과. 웹의 사전 검사는 worker의 실행 검사를 대신하지 않아요. |
| 화면이 다시 계산하는 규칙 | 변경 합계, 배지, 회차 범위 5벌, 앞자리 0 지우기 3벌, 같은 날 공유본 판정 2벌, 서버 한도 숫자예요. 변경 합계와 배지는 [0106](0106-todo-aggregation-in-jobs.md)에서, 나머지는 [0111](0111-web-and-screen-rules.md)에서 모았어요. | 서버가 계산해 보내요. 입력하는 동안의 미리보기도 서버의 미리보기 요청으로 계산해요(사용자 결정, 2026-10-09). 요일 이름 6벌과 크기 표기 2벌처럼 표기만 하는 것과, 보는 사람의 시간대를 쓰는 같은 날 공유본 판정은 화면의 한 모듈이에요(사용자 결정, 2026-10-09). | 화면에 보이는 내용. 크기 표기의 `22 KB`와 `22KB`처럼 화면이 달라지는 통일은 따로 커밋해요. |
| 작은 도우미 | 파일 존재 확인, 기록한 파일 지우기, 16진수 표기, 내려받는 중인 `.part`, 폴더 fsync, 파일 이름 다듬기 4벌, 앱 데이터 폴더 이름, 경로 정규화 3벌, `/proc/net/route` 읽기 2벌, 바쁨 응답, worker 깨우기 14곳, 같은 요청 ID 받기 4벌, 실행 task와 종료 유예 4–5벌이에요. | 쓰는 곳들의 가장 아래 크레이트 | 서버 브라우저의 파일 이름 다듬기는 trss-jobs가 다시 다듬는 두 단계 설계예요([0096](0096-file-and-path-helpers.md)에서 확인). 파일 존재 확인, 기록한 파일 지우기, 16진수 표기, `.part`, 앱 데이터 폴더 이름, 경로 정규화는 0096에서 모았어요. 같은 요청 ID 받기는 [0107](0107-split-job-store.md)에서 작업 만들기 3벌을 trss-jobs의 함수 하나로 모았고, 명령 큐의 것은 키와 견주는 것이 달라 따로 둬요(사용자 결정, 2026-10-09). 실행 task와 종료 유예는 [0108](0108-split-job-runner.md)에서 동작이 같은 수집 주기와 자막 작업 실행만 trss-worker의 함수 하나로 모았어요. `/proc/net/route` 읽기, 바쁨 응답, worker 깨우기는 [0112](0112-small-helpers-and-test-helpers.md)에서 trss-core의 `net_route`, trss-web의 `ApiError` 변환, trss-core의 `wake_worker_if`로 모았어요. 표를 읽지 못할 때 웹은 시작하지 않고 송신 프록시는 빈 표로 보는 차이는 그대로예요. |

### 수집 받기 줄기의 재구성

2026-10-07에는 worker의 수집 주기(`cycle.rs`)와 trss-collect의 한 번 받기 명령(`receive_once.rs`)이 토렌트를 더하고, 기록하고, 이름을 바꾸는 일을 각자 했어요.
이름 바꾸기와 그 재시도도 trss-transmission과 한 번 받기에 따로 있었고, 같은 실패를 한쪽은 영어로, 다른 쪽은 한국어로 적었어요.
이력 전체에서 `cycle.rs`를 바꾼 커밋 49개 중 22개, `receive_once.rs`를 바꾼 커밋 52개 중 29개가 수정 커밋이에요. [ADR 0015](../../../adr/0015-test-a-rule-once-in-its-crate.md)에 적은, 다시 받은 토렌트의 이름을 58분 사이에 네 번 고친 일도 여기서 났어요.
수집 판단(`process_job`, `withhold`, `start_replacement`, `make_plans`)이 worker에 남아 있어서, worker에는 실행 루프, 명령 분배, 주기만 남긴다는 [ADR 0011](../../../adr/0011-feature-crate-workspace.md)과 어긋났어요.
[0099](0099-one-receive-path.md)에서 trss-collect의 `receive`(더하기, 기록, 이름 바꾸기)와 `cycle`(수집 판단)로 모았어요.

- 항목 하나를 더하고, 기록하고, 이름을 바꾸는 일은 trss-collect의 한 경로가 맡고, 수집 주기와 한 번 받기 명령이 그 경로를 불러요.
- 이름 바꾸기와 재시도는 한 곳에 있어요. trss-transmission에는 RPC 호출과, 호출하는 쪽이 넘긴 이름 계산만 남아요.
- worker에는 실행 루프, 명령 분배, 주기만 남아요.
- 명령 표의 한 번 받기 전용 칸(`add_unconfirmed`, `original_name`)이 지금 뜻을 지켜요. 수집 주기가 그 칸을 쓰지 않는 지금 동작도 그대로예요.
- 같은 실패를 다르게 적는 두 문구는, [문구와 표기](../../../specs/web-app.md#문구와-표기)를 따르지 않는 쪽을 고치는 결함 수정으로 따로 커밋해요.
- 위의 "릴리스 이름과 회차 읽기"를 이 줄기에서 함께 모아요.

worker 잠금은 멈춘 worker가 마지막 하트비트 뒤에 따로 띄운 task에서 풀어요. 타이밍 때문에 흔들린 테스트를 고친 커밋 네 개(`396145e`, `756ecdc`, `89eeb6c`, `bd9f274`)가 이 비동기 해제에서 비롯됐어요.
따로 띄운 task에서 푸는 것은 패닉이나 중단으로 끝난 경우뿐이라서, 사용자는 지금처럼 두기로 했어요([0100](0100-worker-lock-release.md), 2026-10-08). 해제를 기다리는 테스트 도우미는 worker 테스트의 `wait_lock_free` 하나예요.

테스트는 정리를 마친 뒤 [0101](0101-receive-line-tests.md)에서 ADR 0015대로 나눴어요(2026-10-09).

- Transmission 가짜 서버는 trss-worker의 테스트 폴더에만 있었어요. [0097](0097-shared-transmission-fake.md)에서 trss-transmission의 `fake`로 옮겨, trss-collect의 테스트도 써요. 피드 서버와 nyaa 검색의 가짜는 trss-collect의 `fake`에 있어요.
- 2026-10-07 worker의 해당 테스트는 한 번 받기 78개, 영상 수정본 86개, 수집 주기 41개, 영상 회차 변환 44개, 보관 폴더 이동 30개였어요. 0101에서 worker의 다섯 파일에 있던 297개 중 211개를 trss-collect와 trss-transmission으로 내리고 28개를 지워 60개가 남았어요. worker에는 잠금, 주기와 명령의 순서, 동시 실행, 중단 뒤 이어 하기, 웹에서 worker까지 이어지는 명령만 남아요.
- 조사에서는 그대로 겹치는 테스트를 30–60개, 내릴 규칙 테스트를 약 100개로 추정했어요. 테스트마다 몸통을 견주어, 남는 테스트가 확인하지 않는 입력이나 결과가 있으면 지우지 않고 내렸어요.
- 완료는 위의 요구가 코드에 있고, 기존 worker 테스트가 정리 전까지 고치지 않고 통과했고, 테스트를 나눈 뒤 [RSS 수집 명세](../../../specs/collection.md#검증-표)에 검증 표가 있는 때예요. 2026-10-09에 셋 다 갖춰 이 줄기를 마쳤어요. worker의 수집 영역 중 0101의 다섯 파일 밖에 있는 테스트는 [0113](0113-remaining-area-tests.md)에서 나눴어요.

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
- 완료는 위의 요구가 코드에 있고, 테스트를 나눈 뒤 [작업과 인증 명세](../../../specs/jobs.md#검증-표)와 [자막 명세](../../../specs/subtitles.md#검증-표)에 검증 표가 있는 때예요. 2026-10-09에 다 갖춰 이 줄기를 마쳤어요. 웹 코드에 남은 이 줄기의 규칙 아홉 곳은 [0111](0111-web-and-screen-rules.md)에서 trss-collect와 trss-jobs로 옮겼어요.

### 그 밖의 테스트 정리

- trss-web의 `folders_on_different_filesystems_are_refused`는 `/proc`를 훑어 혼자 4.9초가 걸리고, trss-collect의 같은 이름 테스트와 같은 규칙을 확인해요. 웹 쪽은 ADR 0015대로 정리해요. 0113에서 `/proc`를 수집 폴더로 저장하던 요청을 빼서, 웹 테스트는 거절 응답만 확인하고 `/proc`를 훑지 않아요.
- 웹 API 테스트 461개 중 70–110개가 다른 크레이트의 규칙을 다시 확인한다고 추정했어요. 그 규칙의 영역을 정리할 때 함께 나눠요. 0101과 0110이 나누지 않은 나머지는 0113에서 나눴어요.
- 가짜 AniList와 가짜 Anissia의 같은 뼈대, 웹 테스트 모듈 19곳의 요청 도우미, ZIP 만들기 4벌, 루프백 서버 띄우기 약 14곳은 하나로 모아요. [0112](0112-small-helpers-and-test-helpers.md)에서 trss-core의 `loopback`과 `fake_http`, trss-web의 `testing`, trss-archive의 `testing`(사용자 결정, 2026-10-10)으로 모았어요.
- trss-subtitles의 `testing.rs`에 섞인 테스트 20개는 도우미와 떼어요. 0112에서 `testing/tests.rs`로 옮겼어요.

### 영역별 검증 표

한 영역의 테스트를 정리할 때 그 영역 명세에 요구별 검증 표를 둬요(사용자 결정, 2026-10-07).
표는 요구마다 실제 사용에서 관찰한 것(날짜와 티켓), 테스트로 확인한 것(테스트 이름이나 파일), 확인하지 않은 것을 나눠 적어요.
완료는 여섯 명세([웹 앱 공통](../../../specs/web-app.md), [라이브러리와 작품](../../../specs/library.md), [RSS 수집](../../../specs/collection.md), [자막](../../../specs/subtitles.md), [작업과 인증](../../../specs/jobs.md), [설정과 이전](../../../specs/settings.md))에 모두 표가 있는 때예요. 2026-10-10에 0113이 웹 앱 공통, 라이브러리와 작품, 설정과 이전 명세에 표를 둬서 여섯 명세 모두에 표가 있어요.

### 순서

1. 기준값을 재요.
2. 테스트 DB를 한 번만 만들고, 되풀이해 쓰는 SQL 문을 캐시해요. 뒤의 모든 확인이 빨라져요. 테스트 DB는 [0093](0093-migrated-test-db-once.md)에서 기준값보다 먼저 만들었어요(사용자 요청, 2026-10-07).
3. 위험이 낮은 개념을 먼저 모아요. 같은 파일 알아보기, 회차 텍스트의 키, 작은 도우미의 일부예요.
4. 수집 받기 줄기를 재구성하고 그 테스트를 나눠요. 릴리스 이름과 회차 읽기가 여기 들어가며, [0090](../5-deployed-verification/0090-release-name-corpus.md)과 [0091](../5-deployed-verification/0091-trname-long-and-half-episodes.md)이 끝나 있어야 해요.
5. 요청 간격, 큐 루프, 파일 쓰기를 모아요.
6. 자막 작업 줄기를 재구성하고 그 테스트를 나눠요. 회차 대응, 할 일 집계, 화면이 다시 계산하는 규칙이 여기 들어가요.
7. 남은 웹 규칙과 작은 도우미를 모으고, 라이브러리, 설정, 웹 앱 공통 영역의 테스트를 나눠요.
8. 마지막으로 다시 재고, 정리한 릴리스를 실제 서버에 올려 목표 5의 [0083](../5-deployed-verification/0083-deployed-end-to-end.md) 흐름을 한 번 더 이어 봐요. 실제 서버에 올리는 일은 사용자 승인을 받아요.

### 티켓에서 받을 사용자 결정

| 결정 | 언제 | 사용자 결정 (2026-10-08) |
| --- | --- | --- |
| fsync가 없는 이름 바꾸기와 내려받기(trss-collect의 영상 이름 바꾸기, 서버 브라우저의 내려받기, 표지 폴더 일부)에 fsync를 더할지예요. 미디어 디스크에 비용이 생겨요. | 파일 쓰기를 모을 때 | 더해요. [0104](0104-durable-file-writes-in-core.md)에서 해요. |
| 큐마다 따로 쥐는 파일 잠금을 단일 worker([ADR 0006](../../../adr/0006-separate-web-worker-binaries.md))에서도 남길지예요. | 큐 루프를 모을 때 | 남겨요. 두 번째 worker 프로세스도 뜨면 큐를 돌리기 때문이에요. [0103](0103-queue-loop-in-core.md)에서 해요. |
| worker 잠금을 동기로 풀지예요. 중단 뒤 웹이 바쁨으로 보이는 시간이 달라져요. | 수집 받기 줄기에서 | 지금처럼 둬요([0100](0100-worker-lock-release.md)). |
| 회차 키가 `13.0`을 13으로 볼지예요. 작품 상세와 목록이 지금 달라졌어요. | 회차 텍스트의 키를 모을 때 | 목록도 13으로 봐요([0095](0095-episode-text-key-in-core.md)). |

### 완료 판정

리팩터링은 위의 요구별 완료가 모두 이루어지고, 기준값과 마지막 측정, 변경 범위와 테스트 개수의 앞뒤가 [기록](#기록)에 있을 때 끝나요.
끝나면 결과와 남은 일을 이 문서에 적고, 이 문서와 리팩터링 티켓을 `docs/archive/tickets/` 아래의 한 폴더로 옮겨요.

## 결과

2026-10-10에 [0114](0114-refactor-final-check.md)에서 마지막 측정, 예전 복사본 찾기, 실제 서버 확인을 마치고 이 리팩터링을 닫았어요. 리팩터링의 코드 커밋은 [기록의 커밋](#커밋)에 있어요.

### 요구별 결과

| 요구 | 결과 |
| --- | --- |
| 동작 보존 | 영역마다 기존 테스트를 고치지 않고 통과시킨 뒤 테스트를 나눴어요. 정리하다 찾은 결함과 [0115](0115-erai-magnet-revisions.md)·[0128](0128-keep-unnamed-cycle-torrent.md)·[0130](0130-receive-a-rule-in-turn.md)은 정리 커밋과 따로 `fix` 커밋 22개로 고쳤어요. 같은 파일 식별의 세 저장 형식과 `n:13` 꼴 키는 그대로 읽고, trss-core `file_id`와 `episode`의 테스트가 확인해요. 0.6.2(`bab88cf`) 뒤로 마이그레이션이 바뀌지 않아서 서버 DB 복사본으로 확인할 일은 없었어요. |
| 웹 API의 JSON 모양 | 화면의 계산을 서버로 옮긴 [0106](0106-todo-aggregation-in-jobs.md), [0111](0111-web-and-screen-rules.md), [0114](0114-refactor-final-check.md#예전-복사본-찾기)는 응답에 필드를 더했어요. 0114가 옮긴 회차 순서에서, 숫자가 아닌 회차 텍스트끼리의 순서가 화면의 UTF-16 순서에서 Rust의 바이트 순서로 바뀌었어요. 둘은 U+FFFF보다 큰 글자를 U+E000–U+FFFF의 글자와 견줄 때만 달라요. |
| 독립 리뷰 | [0099](0099-one-receive-path.md), [0102](0102-request-pace-in-core.md), [0104](0104-durable-file-writes-in-core.md), [0109](0109-shared-aside-and-remove.md), [0112](0112-small-helpers-and-test-helpers.md), [0115](0115-erai-magnet-revisions.md)에서 받았고, 지적과 처리는 각 티켓에 있어요. [0100](0100-worker-lock-release.md)은 동시 실행 코드를 바꾸지 않아서, [0103](0103-queue-loop-in-core.md)과 [0105](0105-episode-mapping-in-library.md)는 동작을 바꾸지 않는 이동이라서 받지 않았어요. |
| 테스트 DB를 한 번만 만들기 | [0093](0093-migrated-test-db-once.md)에서 테스트 바이너리 31개의 합이 glibc에서 71.5·68.5초에서 53.0·53.1초로, musl에서 185.9초에서 65.6초로 줄었어요(2026-10-07). 제품 빌드에 테스트 전용 경로가 켜지지 않는 것을 `cargo tree`로 봤어요. |
| SQL 문 캐시 | [0117](0117-sql-statement-cache.md)에서 715곳이 연결의 문 캐시를 쓰게 했어요. 0116과 같은 web 부하에서 trss-web의 CPU 시간이 5.91초에서 3.76초로 줄었고, user 표본 중 SQL 해석 함수의 비중은 8.52%에서 0.43%가 됐어요(2026-10-08). 테스트 시간에는 뚜렷한 효과가 없었어요. |
| 흩어진 개념 모으기 | 아홉 행을 모두 모았어요. 0114에서 행마다 예전 복사본을 이름으로 찾았고, 남아 있던 복사본은 `5eb6d98`, `653f03b`, `6caf49e`에서 모은 곳을 쓰게 했어요. 일부러 남긴 것과 그 까닭(trss-probe의 `dev`·`ino` 쌍, 웹의 바쁨 문구 두 문장, 웹의 `in_place`)은 [0114의 표](0114-refactor-final-check.md#예전-복사본-찾기)에 있어요. |
| 수집 받기 줄기 | 2026-10-09에 마쳤어요. [0099](0099-one-receive-path.md)가 trss-collect의 `receive`와 `cycle`로 모았어요. [0101](0101-receive-line-tests.md)은 worker의 다섯 파일에 있던 테스트 297개 중 211개를 아래 크레이트로 내리고 28개를 지워, 60개가 남았어요. |
| 자막 작업 줄기 | 2026-10-09에 마쳤어요. [0107](0107-split-job-store.md)–[0109](0109-shared-aside-and-remove.md)에서 작업 저장소와 실행기를 나누고 옆으로 옮기기를 한 구현으로 모았어요. [0110](0110-job-line-tests.md)은 tokio 타이머를 기다리던 테스트 15개를 멈춘 tokio 시간으로 기다리게 해, 그 합이 45.8초에서 0.5초가 됐어요. |
| 그 밖의 테스트 정리 | [0112](0112-small-helpers-and-test-helpers.md)와 [0113](0113-remaining-area-tests.md)에서 마쳤어요. |
| 영역별 검증 표 | 2026-10-10에 여섯 명세 모두에 표가 있어요. |
| 잣대 | 테스트 바이너리의 `finished in` 합이 56.45초에서 43.0초로 23.8% 줄었고, 통과한 테스트는 2,818개에서 3,026개가 됐어요. 변경 범위는 `feat`·`fix`만 세면 기준값의 44개가 크레이트 2.70개·파일 13.2개, 리팩터링 기간의 30개가 1.90개·5.7개예요. 리팩터링 기간의 커밋은 대부분 리팩터링 자체라서, 정리한 코드에서 기능 하나가 닿는 범위가 줄었는지는 이 값으로 말할 수 없어요([마지막 측정](#마지막-측정)). |
| 실제 서버 | 2026-10-10에 0.7.0으로 배포하고 [0083](../5-deployed-verification/0083-deployed-end-to-end.md)의 흐름을 이어 봤어요. 고른 후보의 자막, 휴대폰의 직접 찾기(erulabo), 교체 승인, 보관본 `적용`의 화면을 봤어요. 0083과 다른 점은 모두 까닭을 가렸고, 리팩터링이 바꾼 동작은 찾지 못했어요. 0.7.0에서 구독 규칙이 새 회차를 받는 흐름은 보지 못했어요([0114](0114-refactor-final-check.md#실제-서버-2026-10-10-070)). |

### 티켓에서 받은 결정의 기록

[티켓에서 받을 사용자 결정](#티켓에서-받을-사용자-결정)의 네 결정 가운데 `13.0`을 13으로 보는 것은 [라이브러리와 작품 명세](../../../specs/library.md)와 trss-core `episode`의 테스트가 담아요.
나머지 셋은 코드를 고쳐 되돌릴 수 있어서 ADR로 적지 않았어요. 동작은 trss-core의 `files`, `queue`, `lock` 모듈 문서와 테스트가 담아요. 까닭은 [0104](0104-durable-file-writes-in-core.md)(미디어 디스크의 fsync 비용), [0103](0103-queue-loop-in-core.md)(두 번째 worker 프로세스도 큐를 돌려요), [0100](0100-worker-lock-release.md)(중단 뒤 웹이 바쁨으로 보이는 시간)과 이 문서에만 있어요.

### 테스트나 문서가 담지 않는 요구

2026-10-10에 요구마다 테스트, ADR, 명세, 코드 문서를 찾아봤어요. 아래는 이 문서를 옮긴 뒤 어디에도 규칙으로 남지 않는 것이에요.

- 되풀이해 쓰는 SQL 문을 `prepare_cached`로 받는 것은 trss-core `db.rs`의 `STATEMENT_CACHE_CAPACITY` 문서가 전제로만 적어요. 검사하는 테스트나 lint는 없고, 한 번만 쓰는 문은 일부러 `prepare`로 남겼어요.
- 테스트 전용 경로(trss-core의 `test-support`)가 제품 빌드에 켜지지 않는 것은 0093에서 한 번 봤어요. 다시 확인하는 테스트나 배포 단계는 없어요.
- 마이그레이션을 실제 서버에 올리기 전에 서버 DB 복사본에서 확인하는 규칙(사용자 결정, 2026-10-07)은 이 문서와 [0080](../5-deployed-verification/0080-server-db-migration-check.md)의 절차에만 있었어요. 같은 날 사용자가 리팩터링 뒤에도 지키기로 해서, [티켓 목록의 운영](../../../tickets/README.md#운영)에 옮겼어요.
- 한 개념은 한 곳에 둔다는 원칙은 [ADR 0016](../../../adr/0016-shared-parts-in-core.md)에 있고, 모은 곳은 각 모듈의 `//!` 문서가 적어요. 복사본이 다시 생기는 것을 막는 테스트나 lint는 없어요. trss-jobs를 `JobViews`·`JobRequests`·`JobRun`·`place::PlaceStore`로 나눈 선도 코드 문서에만 있어요.
- 웹이 수정본 `다시 받기`를 trss-collect의 판정보다 먼저 거절하는 까닭은 trss-web `in_place.rs`의 문서에만 있어요.
- 한 번 받기 전용 칸(`add_unconfirmed`, `original_name`)의 뜻은 테스트와 [RSS 수집 명세의 검증 표](../../../specs/collection.md#검증-표)가 담고, 그 뜻을 적은 문서는 없어요.
- 화면의 후보 목록이 순위로 정렬하며 `최신 화부터`에서 숫자만 뒤집는 부분과, 분기 이름(`web/src/lib/quarter.ts`)은 테스트가 없어요. `candidates.ts`는 React를 불러서 `node --test`로 돌릴 수 없어요.

### 남은 일

- 리팩터링 중에 찾아 뒤로 미룬 0129, 0131–0135는 티켓 목록의 [리팩터링 뒤의 명세 차이 수정](../../../tickets/README.md#리팩터링-뒤의-명세-차이-수정)에 있어요.
- 규칙의 저장 폴더에서 작품 폴더를 정하는 규칙을 trss-collect의 `plan::work_folder_of`와 `rule_archive::work_folder`가 각자 계산해요. 돌려주는 꼴과 `..`·UTF-8이 아닌 이름의 처리가 달라서, 합치려면 그 차이를 정해야 해요.
- 0.7.0에서 구독 규칙이 새 회차를 받는 것과 구독 제작자의 자막은 `FX Senshi Kurumi-chan` 3화(2026-10-15 12:30 UTC)가 첫 기회예요.
- 정리한 코드에서 기능 하나가 닿는 범위가 줄었는지는 [목표 6](../../../tickets/README.md#6-앱-yaml-내보내기와-가져오기)의 기능 커밋을 `dev/measure/change-spread.sh`로 세어 봐야 알 수 있어요.
- 직접 찾기가 고른 후보와 같은 파일을 다른 근거로 받으면, 같은 파일을 가리키는 보관본이 하나 더 생기고 그 보관본에 `교체 비교`가 남아요([0114](0114-refactor-final-check.md#실제-서버-2026-10-10-070)). 기록된 두 결정이 겹친 결과이고, 고칠지는 정하지 않았어요.

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

2026-10-10에 [0114](0114-refactor-final-check.md)에서 `cc91ce5`를 측정했어요. 테스트를 바꾼 마지막 커밋은 `6e46ef1`이고, 그 뒤의 커밋은 문서만 바꿨어요.
기준값과 같은 개발 PC에서 같은 명령과 같은 빌드 상태로 측정했고, 측정을 시작할 때 다른 cargo 빌드나 테스트는 돌고 있지 않았어요. 원자료는 이 PC의 `dev/local/measure/0114/`에 있어요.

테스트 시간이에요.

- 처음 빌드는 ``Finished `test` profile [unoptimized + debuginfo] target(s) in 0.18s``였어요. `6e46ef1`을 빌드해 둔 `target/`이라 다시 컴파일한 것이 없었어요.

| | 기준값 1회 | 기준값 2회 | 마지막 1회 | 마지막 2회 |
| --- | ---: | ---: | ---: | ---: |
| cargo의 `Finished … in` | 0.12s | 0.13s | 0.11s | 0.11s |
| 테스트 바이너리 31개의 `finished in` 합 | 56.6초 | 56.3초 | 43.1초 | 42.9초 |
| 걸린 시간(doc test 포함) | 59.36초 | 58.71초 | 45.11초 | 44.89초 |
| 통과·실패·무시 | 2,818·0·14 | 2,818·0·14 | 3,026·0·14 | 3,026·0·14 |

- 두 번의 평균으로 견주면 합은 56.45초에서 43.0초로, 걸린 시간은 59.04초에서 45.0초로 둘 다 23.8% 줄었어요. 마지막 두 번의 차이는 합과 걸린 시간 모두 0.5%예요.
- 통과한 테스트는 208개 늘었어요. 위 레이어의 테스트를 주인 크레이트로 내리며 경우를 나눠 더한 것이 많아요.
- 크게 줄어든 것은 worker `tests/it`(11.7초 → 6.6초, [0101](0101-receive-line-tests.md)과 [0113](0113-remaining-area-tests.md)), trss-jobs `tests/it`(10.9초 → 3.5초, [0110](0110-job-line-tests.md)이 제품의 폴링 간격을 실제로 기다리던 원격 화면·찾기 테스트 15개를 멈춘 시간으로 바꿨어요), trss-anissia lib(0.70초 → 0.21초)예요. trss-collect lib는 내려온 테스트를 받아 1.0초에서 2.7초로 늘었어요.

| 바이너리 | 기준값 1회 | 기준값 2회 | 마지막 1회 | 마지막 2회 |
| --- | ---: | ---: | ---: | ---: |
| trss-web lib | 7.58초 | 7.51초 | 6.98초 | 6.96초 |
| trss-worker `tests/it` | 11.73초 | 11.71초 | 6.58초 | 6.59초 |
| trss-jobs `tests/it` | 10.89초 | 10.97초 | 3.46초 | 3.47초 |
| trss-browser `tests/it` | 3.16초 | 3.15초 | 3.16초 | 3.15초 |
| trss-web `tests/subtitle_upload_peak.rs` | 3.12초 | 3.02초 | 2.93초 | 2.93초 |
| trss-jobs `tests/extract_process.rs` | 2.72초 | 2.72초 | 2.74초 | 2.74초 |
| trss-collect lib | 1.04초 | 1.03초 | 2.73초 | 2.70초 |
| trss-library lib | 2.29초 | 2.22초 | 2.07초 | 2.06초 |
| trss-archive `tests/it` | 2.84초 | 2.95초 | 2.05초 | 1.88초 |
| trss-collect `tests/revision_crc_peak.rs` | 2.00초 | 1.99초 | 1.91초 | 1.91초 |
| trss-jobs lib | 2.07초 | 2.05초 | 1.79초 | 1.80초 |
| trss-browser lib | 1.50초 | 1.50초 | 1.50초 | 1.50초 |
| trss-web `tests/artwork_serving_peak.rs` | 1.50초 | 1.46초 | 1.43초 | 1.43초 |
| trss-core lib | 1.04초 | 1.05초 | 1.32초 | 1.34초 |
| trss-subtitles lib | 1.22초 | 1.13초 | 1.07초 | 1.07초 |
| trss-anilist lib | 0.69초 | 0.69초 | 0.69초 | 0.68초 |
| trss-anissia lib | 0.70초 | 0.71초 | 0.21초 | 0.21초 |
| trss-library `tests/artwork_header_check.rs` | 0.18초 | 0.17초 | 0.17초 | 0.17초 |
| trss-web `tests/artwork_upload_peak.rs` | 0.13초 | 0.11초 | 0.11초 | 0.10초 |
| trss-worker lib | 0.09초 | 0.09초 | 0.08초 | 0.08초 |
| trss-probe main | 0.08초 | 0.08초 | 0.08초 | 0.08초 |
| trss-transmission lib | 0.00초 | 0.00초 | 0.01초 | 0.01초 |
| trss-import lib | 0.01초 | 0.01초 | 0.01초 | 0.01초 |
| 나머지 8개 | 각 0.00초 | 각 0.00초 | 각 0.00초 | 각 0.00초 |

musl 대상([0079](../5-deployed-verification/0079-musl-test-run.md)의 `dev/musl-test.sh`)도 같은 커밋에서 돌렸어요. 의존성까지 처음 빌드한 시간은 3분 11초이고, 빌드한 뒤 다시 돈 두 번의 합은 51.9초와 51.3초예요. 두 번 다 3,026개가 통과했고, 실패는 0개, 무시는 14개였어요.

변경 범위예요. 세는 방법은 기준값과 같아요.

| 기간 | 커밋 | 크레이트 | 파일 |
| --- | --- | ---: | ---: |
| 2026-10-05–08, `ff7e6df`까지(기준값) | 49개(`feat` 25, `fix` 19, `perf` 5) | 2.45개 | 12.0개 |
| 2026-10-08–10, `ff7e6df` 뒤 `cc91ce5`까지(리팩터링 기간) | 112개(`feat` 8, `fix` 22, `refactor` 79, `perf` 3) | 2.21개 | 7.5개 |
| 2026-10-07–10, `cc91ce5`까지(기준값과 같은 나흘) | 139개(`feat` 17, `fix` 38, `refactor` 79, `perf` 5) | 2.08개 | 7.2개 |

- `feat`·`fix`만 세면 기준값은 44개가 크레이트 2.70개·파일 13.2개, 리팩터링 기간은 30개가 1.90개·5.7개예요.
- 리팩터링 기간의 커밋은 대부분 리팩터링 자체이고, 그 기간의 `feat` 8개는 화면의 계산을 서버로 옮긴 [0111](0111-web-and-screen-rules.md)의 것이에요. 그래서 이 값으로는 정리한 코드에서 기능 하나가 닿는 범위가 줄었는지 말할 수 없어요. 그것은 리팩터링 뒤의 기능 작업([목표 6](../../../tickets/README.md#6-앱-yaml-내보내기와-가져오기))에서 같은 방법으로 세어 봐야 알 수 있어요.

### 커밋

리팩터링의 코드 커밋 175개예요. 0093의 `533d321`과, 기준값 `ff7e6df` 뒤 `6caf49e`까지의 `docs`가 아닌 커밋을 제목에 적은 티켓으로 묶었어요. 제목에 티켓이 없는 커밋은 그 hash를 적은 티켓 아래에 뒀어요.
종류별로 `refactor` 81개, `test` 59개, `fix` 22개, `feat` 9개, `perf` 4개예요. [0092](0092-refactor-baseline.md)와 [0100](0100-worker-lock-release.md)은 코드 커밋이 없어요. 0092의 측정 도구 `b70ae9f`는 0093보다 먼저 들어갔어요.

- [0093](0093-migrated-test-db-once.md)
  - `533d321` perf(core): copy new test databases from one migrated per process
- [0094](0094-file-identity-in-core.md)
  - `a43ccfe` refactor(library): rename the discovery FileIdentity to SeenFile (0094)
  - `c2944a4` refactor(core): recognize a recorded file by its inode in one place (0094)
  - `a8b218a` test: keep the device-number tests that reach a feature path (0094)
- [0095](0095-episode-text-key-in-core.md)
  - `e03223e` fix(library): list a 13.0 episode as episode 13 like the work detail (0095)
  - `eb58482` refactor(core): read, order and label an episode text in one place (0095)
- [0096](0096-file-and-path-helpers.md)
  - `2adf9e4` refactor(core): keep one copy of the small file helpers (0096)
  - `c8d915d` refactor(core): name the app data folders in trss-core (0096)
  - `c5702f9` refactor(core): keep one lexical path normalization for folder locks and rules (0096)
  - `0b99139` refactor(core): check a folder and a pair of folders in one place (0096)
  - `6e9063e` fix(library): count only Transmission's lower-case .part as a download in progress (0096)
  - `d9cdad7` fix(core): treat /../x and /x as one folder in folder locks and rule paths (0096)
- [0097](0097-shared-transmission-fake.md)
  - `8239ab7` refactor(test): move the fake Transmission, feed and nyaa servers into the crates that own their clients
- [0098](0098-release-name-reading.md)
  - `1fb4ede` test(collect): check a refused torrent-set-location of a work folder move against the fake Transmission
  - `6c82abd` refactor(core): gather the readers of trname-written names and season folders in trss_core::trname_names
  - `d0403c2` refactor(collect): one implementation of the episode conversion's arithmetic
  - `d8b8d9d` refactor(collect): read a release name in one place, trss_collect::release_name
  - `4202f22` fix(collect): read a bare four-digit episode (0098)
  - `9a1b352` fix(collect): read the extension, the revision mark, the season and the dash of a release name one way (0098)
  - `41ca53d` fix(collect): keep the name of a torrent read as no episode or as a batch (0098)
  - `33b5125` test(collect): read the example names of release_name from the corpus (0098)
  - `7a0f0cf` test(collect): check that the predicted episode is the one in the name the worker gives (0098)
- [0099](0099-one-receive-path.md)
  - `000b078` test(worker): pin what the cycle leaves of its own adds whose name stays (0099)
  - `b701931` refactor(collect): move the cycle's item decisions out of the worker (0099)
  - `ffb6ded` fix(collect): write the cycle's add failures in Korean like 다시 받기 (0099)
  - `9086756` refactor(collect): one path adds, records and renames a received torrent (0099)
  - `e984ee8` refactor(collect): one constructor for the row a revision decision writes (0099)
  - `59370f4` fix(collect): ask again for a torrent without a file count in the cycle too (0099)
  - `78936a3` fix(collect): stop the cycle's rename once nothing more can come of it (0099)
  - `d63e767` fix(collect): derive the cycle's rename from the first name it reads (0099)
  - `a6c20cb` perf(collect): let the cycle's rename go on in the add's Transmission session (0099)
- [0101](0101-receive-line-tests.md)
  - `8032954` test(collect): test a revision's decision at the cycle in trss-collect (0101)
  - `aa1819a` test(collect): test advancing a video replacement in trss-collect (0101)
  - `d4ecd9e` test(collect): test 다시 받기 of a video revision in trss-collect (0101)
  - `c027b02` test(collect): test the offset a cycle decides for a new season in trss-collect (0101)
  - `1e55cf6` test(collect): test the episode undo of a new season in trss-collect (0101)
  - `66f60bd` test(collect): test the first past item, the restarted cour and worth_offering in trss-collect (0101)
  - `96182ae` test(collect): test the rules of 다시 받기 in trss-collect, not through the worker (0101)
  - `74181b8` test(collect): test what a cycle decides about an item in trss-collect, not through the worker (0101)
  - `749e275` test(transmission): test the add of a torrent already there and the cleanup in trss-transmission, not through the worker (0101)
  - `2e7c4e3` test(collect): test the archive move of a work folder in trss-collect, not through the worker (0101)
  - `90b98b2` test(web): leave the rules of the collect API tests to trss-collect, keep the web's part (0101)
  - `a4bb97c` test(web): test the in-place rule on Evidence::held, not through the router (0101)
- [0102](0102-request-pace-in-core.md)
  - `4383469` refactor(core): read Retry-After, bounded bodies and the download size limit in trss-core (0102)
  - `5d8edcf` refactor(core): keep the request pace and the wait for a turn in trss-core (0102)
  - `6d05069` test(core): test the request pace rule once in trss-core (0102)
  - `eca71d1` refactor(collect): drop the unused host-keyed pace methods of SearchPace (0102)
- [0103](0103-queue-loop-in-core.md)
  - `d779a94` refactor(core): name the lock files in one place and build the write check's list from it (0103)
  - `5edddd1` refactor(core): run the four background queues in one loop in trss-core (0103)
  - `4796b18` refactor(core): compute when a put-off queue item is tried again in trss-core (0103)
- [0104](0104-durable-file-writes-in-core.md)
  - `a97ea38` refactor(core): sync folders and publish files without replacing in trss-core (0104)
  - `5ec757c` refactor(library): write the staged artwork file with trss-core's create-new write (0104)
  - `680ec22` refactor(probe): sync folders with trss-archive's sync_dir (0104)
  - `65667fd` fix(collect): sync the folders after renaming a video itself (0104)
  - `476fa94` refactor(collect): tell a filesystem without the no-replace rename in one place (0104)
  - `8419452` fix(browser): sync a download's bytes and folders when it is moved out (0104)
  - `7d2fd9d` fix(library): sync the artwork folders and fail a publish whose rename is not durable (0104)
  - `57812fc` refactor(jobs): sync a recovered receive file with trss-core's sync_file (0104)
  - `60966d3` test: test the rename that replaces nothing once, in trss-core (0104)
  - `e52e0f0` perf(core): sync a folder once when a rename stays inside it (0104)
- [0105](0105-episode-mapping-in-library.md)
  - `655e85f` refactor(core): move the offset and episode-range wording to trss-core's episode (0105)
  - `c629986` refactor(library): move the pure episode-mapping computation from trss-jobs (0105)
  - `eec62e2` refactor(library): build the reasons an episode is not placed in one place (0105)
  - `843ed4b` fix(library): leave a whole episode unmapped when the offset overflows it (0105)
  - `4b85d7f` refactor(collect): add the episode offset of an Anissia caption with the library's (0105)
  - `889ccdb` fix(library): word the placement reasons alike and say 1화보다 앞 without an episode count (0105)
- [0106](0106-todo-aggregation-in-jobs.md)
  - `6ee407a` refactor(jobs): gather, group, sum and order the to-dos in trss-jobs (0106)
  - `9d777b8` refactor(web): show the server's to-do badges and plan change totals (0106)
- [0107](0107-split-job-store.md)
  - `0d511eb` refactor(jobs): split store.rs by feature and move the place delegations into place/ (0107)
  - `e7f1542` refactor(jobs): ask one function whether a request ID was used before (0107)
  - `fc2a83d` refactor(jobs): split JobStore into JobViews, JobRequests and JobRun over the same Db (0107)
- [0108](0108-split-job-runner.md)
  - `a98d957` refactor(jobs): move receiving, publishing and recovering a file into Receiver (0108)
  - `bbe1c8f` refactor(jobs): move the remote-screen care and the find job into ScreenTender (0108)
  - `21555b1` refactor(worker): wait for a spawned task within the shutdown grace in one helper (0108)
- [0109](0109-shared-aside-and-remove.md)
  - `16c6887` fix(jobs): sync the folders a set-aside file is made in and removed from (0109)
  - `78ae270` refactor(jobs): share the set-aside, check, put-back and removal of replacement and relocation (0109)
  - `eb5f495` refactor(jobs): make hold_stuck ask each feature to hold its own records (0109)
- [0110](0110-job-line-tests.md)
  - `2d68052` test(jobs): wait the remote-screen and find polls in paused tokio time (0110)
  - `f57860e` test(jobs): share the clocks and the setup base of the integration tests (0110)
  - `6cebe0d` test(jobs): move the to-do cards and the compare cases of the job line's web tests down (0110)
  - `d9a349a` test(jobs): move the replacement to-do cases of the replacement web tests down (0110)
  - `75a0042` test(jobs): move the stored-copy rules of the work page's web tests down (0110)
  - `1383377` test(jobs): move the upload rules of the upload web tests down (0110)
  - `93afdab` test(jobs): move the to-do, mapping and creator rules of their web tests down (0110)
  - `5d1fb3b` test(jobs): trim the screen requests of the screen web tests to the web's part (0110)
- [0111](0111-web-and-screen-rules.md)
  - `3476cb9` refactor(core): judge worker liveness in the heartbeat module (0111)
  - `5919b32` refactor(collect): move the rule preview and the overlap flag out of the web (0111)
  - `6a5e12d` fix(web): tell a rule's last received time from the whole history (0111)
  - `bc24dcf` refactor(collect): compile a rule's regex in one place (0111)
  - `65ac930` refactor(web): use the core calendar for the status board's dates (0111)
  - `1ccb12e` refactor(collect): put the quarter helpers on Quarter (0111)
  - `0f55b1a` refactor(jobs): give the shown subtitle format of a name to SubtitleFormat (0111)
  - `94c9ca6` refactor(library): look up an episode's air time in the library (0111)
  - `c41c31c` refactor(web): build the retry offer of a stopped revision in one place (0111)
  - `62f7aff` test(core): test worker liveness once, in the heartbeat module (0111)
  - `e3db410` test(collect): test the rule preview once, in the plan's preview module (0111)
  - `302ecd2` refactor(collect): decide an Anissia candidate's revision mark beside its primitives (0111)
  - `40aaa4c` refactor(collect): tell the files of a failed replacement from its row in trss-collect (0111)
  - `5369eb1` refactor(jobs): read the receive failures in one place for the list and the to-do (0111)
  - `fcbfc46` refactor: decide a creator's candidates and the jobs made of them outside the web (0111)
  - `9fce281` refactor(jobs): decide whether a remote screen still belongs to its job in Screen (0111)
  - `bbcf1ad` refactor(jobs): group the open jobs and tell what a job waits for in trss-jobs (0111)
  - `45cb8ec` refactor(jobs): tell how a receipt and its unpacking stand in trss-jobs (0111)
  - `4848d11` refactor(jobs): convert a seen file's change time to milliseconds in one place (0111)
  - `5623810` refactor(jobs): group the stored copies by creator and decide what choosing one does in trss-jobs (0111)
  - `50882a0` test(collect): test an Anissia candidate's revision mark once, in trss-collect (0111)
  - `d2e5db0` test(collect): test the files of a failed replacement once, on its row (0111)
  - `891f954` test(collect): test which creators an anime has once, in trss-collect (0111)
  - `005bba7` test(jobs): test the job list's groups and waiting order once, in trss-jobs (0111)
  - `568a1f7` test(jobs): test what came of unpacking an archive once, in trss-jobs (0111)
  - `bfd7dfc` test(jobs): test the stored copies' grouping and choice once, in trss-jobs (0111)
  - `76b0556` refactor(core): compute episode runs once in trss-core episode (0111)
  - `ab046c3` feat(web): send episode runs and shown episode texts in the answers (0111)
  - `eefffc8` feat(web): show the episode runs the server computes (0111)
  - `13142ee` feat(web): send the shown episode of candidates and receive-failed cards (0111)
  - `0ace4ce` refactor(web): show the episode texts the server strips of leading zeros (0111)
  - `22d9780` feat(web): send the server limits the screens copied by hand (0111)
  - `b7ab88e` refactor(web): take the server limits from the answers instead of copies (0111)
  - `84fa020` feat(web): preview the episode mapping dialog's input on the server (0111)
  - `431f652` refactor(web): show the mapping dialog's previews from the server (0111)
  - `a550e3a` feat(web): label the past-search range on the server (0111)
  - `8786067` refactor(web): show the past-search range label from the server (0111)
  - `acec8b9` refactor(web): keep the weekday names in one screen module (0111)
  - `5ec5645` refactor(web): move the size text out of the to-do screen (0111)
  - `ed67573` feat(web): tell sizes with a space and a GB step everywhere (0111)
  - `2bda653` refactor(web): keep the same-day time of received copies in one screen module (0111)
  - `bad6692` feat: name sizes in the server's reasons with a space, as the screen does (0111)
- [0112](0112-small-helpers-and-test-helpers.md)
  - `8613726` refactor: wake the worker through one function in core and one method on the web state (0112)
  - `9d088c2` refactor(web): turn an AniList failure into an API answer in one place (0112)
  - `dc0b5f2` refactor: read the routing table in one core module for the web and the egress proxy (0112)
  - `b5f11b6` refactor(web): build, send and read the API test requests in one test module (0112)
  - `9dbabc9` refactor(subtitles): keep the tests of the test server in their own file (0112)
  - `ae40df8` refactor(test): start the loopback servers of the tests through one trss-core module (0112)
  - `3defc27` refactor(test): share the axum-free parts of the fake AniList and Anissia servers in trss-core (0112)
  - `e957b9e` refactor(test): build the sample ZIPs of the tests in one trss-archive module (0112)
  - `ac2a292` refactor(web): name the route table's path from net_route in the read error (0112)
- [0113](0113-remaining-area-tests.md)
  - `f10e9c9` refactor(collect): decide an episode undo request once for the web and the worker (0113)
  - `0da1a23` refactor(web): ask trss-anissia whether an unlisted anime's run is over in the week (0113)
  - `9429757` refactor(collect): name the season's holder in trss-collect for the web's link views (0113)
  - `85f6dbd` test(web): leave the channel store's rules to the store in the channels_api tests (0113)
  - `a1dbcfe` test(web): leave the retry and adoption reasons to trss-collect in the commands_api tests (0113)
  - `81efa33` test(web): leave the schedule rules to trss-anissia and trss-collect in the schedule_api tests (0113)
  - `7b7dd3a` test(web): leave the stalled and read rules to trss-core and trss-collect in the status_api tests (0113)
  - `95d5f1a` test(web): leave the schedule, creator and store rules to their crates in the subscriptions_api tests (0113)
  - `100016e` test(web): leave the link, search and read rules to their crates in the seasons_anissia_api tests (0113)
  - `7033a11` refactor(import): decide what the checked subscriptions of an import come to in trss-import (0113)
  - `b945db5` test(web): leave the import rules to trss-import and trss-collect in the import_api tests (0113)
  - `743c629` test(web): leave the setup, folder and policy rules to their crates in the setup_api, settings_api and policy_api tests (0113)
  - `4b7b01b` test(web): leave the link, year, airing and search rules to trss-library in the seasons_api tests (0113)
  - `0b7b8a5` test(web): leave the pick refusals and the 429 wait to trss-library in the artwork_api tests (0113)
  - `8aaff30` test(web): leave the paging and the filter-and-search rules to page.rs in the library_api tests (0113)
  - `51c7ad0` test(web): leave the video check card to trss-jobs in the video_check_api test (0113)
  - `2c6bfd0` test(anilist,anissia): leave the Retry-After values to trss-core and keep one 429 wiring test per client (0113)
  - `ea52b35` test(worker): leave the collect folder rules to trss-collect and trss-web in the collect_folder tests (0113)
  - `57ec085` test(worker): move the first read and title-waiting cycle tests to trss-collect (0113)
  - `c0dc929` test(collect): own the past search's receive, revision, paging and Retry-After tests (0113)
  - `da9b731` test(web): test the past search's request checks and the receive_past refusals in trss-web (0113)
  - `3553aa3` test(worker): leave the past search rules to trss-collect and trss-web in the past_search tests (0113)
  - `5e19786` test(library): test the watch folder reads, the rescan outcome and the live watches in trss-library (0113)
  - `59b0179` test(collect): test the season link, the Anissia captions command and the library's follow of a work move in trss-collect (0113)
  - `1eb50b8` test(web): test the watch folder endpoints, the settings registration, the rescan request and the library list shape in trss-web (0113)
  - `23352b7` test(worker): leave the library, season link and Anissia captions rules to the crates that own them in the worker's tests (0113)
  - `5821eba` refactor(collect): sort Transmission's torrents into the status counts in trss-collect, where the worker only calls it (0113)
  - `566989f` refactor(worker): name each queue's lock file with LockFile where the worker starts it, and drop the five per-crate lock_path_for wrappers (0113)
  - `f3fdc33` test(worker): leave the status counts, rule preview, channel edit and secret rules to trss-collect in the worker's tests (0113)
  - `6e46ef1` test(collect): check that a replaced channel's matched rule keeps its ID when it becomes a subscription (0113)
- [0114](0114-refactor-final-check.md)
  - `5eb6d98` refactor: use trss-core's episode key, hex, day length and device number where copies were left (0114)
  - `653f03b` refactor(web): name a quarter in one screen module (0114)
  - `6caf49e` feat(web): send the episode keys, order and forms the screens compared by hand (0114)
- [0115](0115-erai-magnet-revisions.md)
  - `c8dd9c6` fix(collect): read a revision written as (V2) after the episode number (0115)
  - `31202ad` fix(collect): send Erai-raws magnet feed revisions through the revision replacement (0115)
- [0117](0117-sql-statement-cache.md)
  - `fa6a281` perf(db): take repeated SQL statements from the connection's cache (0117)
- [0128](0128-keep-unnamed-cycle-torrent.md)
  - `468e619` fix(collect): keep and note a torrent the cycle cannot rename, as 다시 받기 does (0128)
- [0130](0130-receive-a-rule-in-turn.md)
  - `78011cd` fix(collect): receive the items that may decide a rule's offset one at a time (0130)
- 티켓 없음
  - `47d2b43` fix(collect): say the new video is inside a folder rather than several files
