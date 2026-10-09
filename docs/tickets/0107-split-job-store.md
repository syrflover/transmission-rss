# 0107 작업 저장소를 쓰임에 따라 나눠요

- 상태: 완료
- 출처: [자막 작업 줄기의 재구성](refactoring.md#자막-작업-줄기의-재구성)
- 막는 티켓: [0092](0092-refactor-baseline.md)(기준값)

## 작업

trss-jobs의 `JobStore`(`store.rs`, 2,872줄)는 2026-10-07에 공개 함수가 70개였어요. 웹이 읽는 모양, 세 종류의 작업 만들기(자막 작업, 올리기, 찾기), 집기와 상태 전이, 찾기 작업, 받은 기록과 사건을 한 타입이 다 가져요.
크레이트 분리 뒤 코드가 바뀐 커밋 109개 중 `store.rs`는 마이그레이션 목록과 21번, trss-web의 `jobs_api.rs`와 19번 함께 바뀌었어요. 수정 커밋은 0개라서, 목표는 회귀를 막는 것보다 기능 하나를 더할 때 건드리는 범위를 줄이는 것이에요.
작업 만들기 세 곳은 같은 요청 ID를 다시 받으면 처음 결과를 돌려주는 약 15줄을 각자 가져요. 명령 큐에도 같은 일이 있어요.

- 쓰임에 따라 나눠, 기능 하나가 필요한 부분만 건드리게 해요. 나누는 선은 이 티켓에서 함수마다 부르는 쪽을 세어 정해요.
- 나누는 선이 책임이나 의존 방향이 다른 여러 안으로 갈리면, 안마다 견준 것과 함께 사용자 결정을 받아요.
- 같은 요청 ID 받기는 하나로 모아요.

## 완료 기준

- 나눈 선과 그 근거(부르는 쪽의 수)가 결과 절에 있어요.
- `JobStore`의 함수들이 나눈 부분으로 옮겨지고, 같은 요청 ID 받기가 하나예요.
- 기존 테스트가 고치지 않고 통과해요.

## 결과

`JobStore`는 없어졌어요. 같은 DB 위의 핸들 네 개로 나눴고, 파일은 기능마다 묶었어요.

### 나눈 선과 근거

2026-10-09에 `JobStore`의 함수 70개(생성자와 `db` 빼고)를 부르는 곳마다 셌어요. 테스트 밖에서 부르는 곳은 181곳이었어요.
웹이 부르는 함수와 worker의 `Runner`가 부르는 함수는 거의 겹치지 않았어요. 겹친 것은 `items`(할 일 집계 1곳, `Runner`와 `Placer` 8곳)와 `event`(`Recheck` 1곳)뿐이었어요.

| 타입 | 함수 | 맡는 일 | 테스트 밖에서 부르는 곳 |
| --- | ---: | --- | --- |
| `JobViews` | 8 | 작업 목록, 작업 상세, 기다림 목록 셋, `items`, `picks_of_anime` | 20곳. 웹의 작업·원격 화면·배치 확인 API 6곳, 할 일 집계 4곳, 구독과 시즌 화면 2곳, `Runner`와 `Placer`의 `items` 8곳 |
| `JobRequests` | 7 | 작업 만들기 넷(`create`, `create_under_mapping`, `create_upload`, `create_find`), 올리기의 확인 둘, 찾기 작업 끝내기 요청 | 8곳. 웹의 작업 API 3곳, `Uploads` 3곳, `Follow` 1곳, `Recheck` 1곳 |
| `JobRun` | 36 | 집기, 다시 줄 세우기, 상태 전이, 찾기 작업의 받기 쪽, 단계와 사건, 받은 파일 기록 | 132곳. 모두 `Runner`, 그것이 돌리는 `Placer`와 찾기 작업, worker의 `Recheck`(`event` 1곳)예요. |
| `place::PlaceStore` | 19 | 배치, 교체, 재배치, 정리의 읽기와 사람의 결정 | 21곳. 웹의 작업·라이브러리 API 18곳, 할 일 집계 2곳, `Runner`의 `relocation_note` 1곳 |

- `PlaceStore`의 19개는 `place/` 모듈의 SQL을 부르기만 하는 한 줄짜리였어요. 조사 때 최근 기능 커밋 12개 중 9개가 `store.rs`에 이런 함수를 더했어요. 이제는 `place/api.rs`에 더해요.
- 함수 하나가 두 타입에 있지 않아요. 에이전트가 스크립트로 확인했어요.
- 두 쓰임이 필요한 곳은 핸들 둘을 가져요.

| 가진 쪽 | 핸들 |
| --- | --- |
| 웹 `AppState` | `JobViews`(`jobs`), `JobRequests`(`job_requests`), `PlaceStore`(`place`) |
| `Uploads` | `JobRequests` |
| `Follow` | `JobRequests`, `JobViews` |
| `Recheck`(worker에서만 만들어요) | `JobRequests`, `JobRun` |
| `todo::Sources` | `JobViews`, `PlaceStore` |
| `Runner` | `JobRun`, `JobViews`, `PlaceStore` |
| `Placer` | `JobRun`, `JobViews` |

`Runner::new`와 `Placer::new`는 받는 값의 수가 그대로예요. `JobRun`을 받아 같은 DB로 나머지 핸들을 만들어요.

### 파일

`store.rs`(2,868줄)를 `store/` 아래 일곱 파일로 나눴어요. 파일은 기능마다, 타입은 부르는 쪽마다예요.

| 파일 | 줄 | 담은 것 |
| --- | ---: | --- |
| `store/mod.rs` | 109 | 세 타입의 정의, 오류, 작업의 출처 상수 |
| `store/create.rs` | 384 | 작업 만들기(`JobRequests`), 같은 요청 ID 확인 `earlier` |
| `store/find.rs` | 441 | 찾기 작업 전체. 만들기와 끝내기 요청은 `JobRequests`, 받기 쪽 함수는 `JobRun`의 `impl`이에요. |
| `store/run.rs` | 595 | 집기, 상태 전이, 단계, 사건(`JobRun`) |
| `store/receipts.rs` | 422 | 받은 파일 기록(`JobRun`) |
| `store/views.rs` | 180 | 목록과 상세(`JobViews`) |
| `store/rows.rs` | 550 | 여러 기능이 함께 쓰는 행 타입과 읽기 함수 |
| `place/api.rs` | 334 | `PlaceStore`의 19개 |

### 사용자 결정

2026-10-09에 정했어요.

- **쓰임별 타입으로 나눠요.** 다른 안은 타입 하나를 파일로만 나누는 것과, 감싸는 타입 없이 `Connection` 위의 함수로 바꾸는 것이었어요. 파일로만 나누면 부르는 쪽을 하나도 고치지 않지만, 웹이 worker의 상태 전이를 부르는 것을 막지 못해요. 쓰임별 타입은 웹과 worker를 나눈 [ADR 0006](../adr/0006-separate-web-worker-binaries.md)의 경계를 타입이 지키게 해서, 앞으로 기능을 더할 때를 보고 골랐어요.
- **찾기 작업은 한 파일에 두고, 타입은 부르는 쪽대로 나눠요.** 찾기 작업을 고칠 때 한 파일만 보면 돼요.
- **같은 요청 ID 확인은 trss-jobs 안에서만 모아요.** 명령 큐는 키(기본 키), 견주는 것(kind와 payload), 돌려주는 것(행 전체)이 달라 그대로 둬요.
- **완료 기준 "기존 테스트가 고치지 않고 통과해요"를 바꿨어요.** 쓰임별 타입은 테스트가 저장소를 만들고 부르는 곳을 고쳐야 해서, 고르는 안에 그 비용(약 416곳)을 적어 드렸어요. 그래서 타입을 나누는 커밋에서는 테스트를 기계적으로만 고쳐요. 저장소를 만드는 줄과 함수를 부르는 대상만 바꾸고, 단언, 입력, 기대값, 테스트 이름은 바꾸지 않아요. 앞의 두 커밋은 테스트를 고치지 않았어요.

### 같은 요청 ID 확인

작업 만들기 세 곳(`create`, `create_upload`, `create_find`)이 글자까지 같은 10줄을 각자 가졌어요. 이제 `store/create.rs`의 `earlier`를 불러요. `earlier`는 같은 브라우저 명령 ID로 만든 작업이 있으면 요청이 같을 때 `Existing`, 다를 때 `Mismatch`를 돌려줘요.
`create`는 회차 대응 버전을 먼저 확인하고 `earlier`를 불러요. 그래서 대응이 바뀐 뒤 다시 온 `auto:` 요청은 예전처럼 `Existing`이 아니라 만들지 않음(`None`)을 받아요. 웹의 200·409·202 응답과 명령 큐는 바뀌지 않았어요.

### 테스트를 고친 방식

- trss-jobs 테스트는 `tests/it/main.rs`의 `Handles`(`views`, `requests`, `run`, `place`)로 핸들을 만들어요.
- 테스트 파일 36개를 고쳤어요. 핸들 이름과 타입 이름을 지우고 공백을 합쳐 앞뒤를 견주는 스크립트로 확인하니, 27개는 그 밖에 바뀐 것이 없었어요.
- 남은 9개는 핸들을 만드는 줄과 `Runner::new`에 넘기는 값만 달랐어요. 제가 줄마다 확인했어요. `tests/it/main.rs`의 `Handles`, trss-web 테스트 네 파일의 `JobRun::new(state.db().clone())`, trss-web `remote_screen_docker.rs`, trss-worker의 `src/jobs.rs` 테스트와 `tests/it/anissia_captions.rs`, `tests/it/season_link.rs`예요. trss-web `jobs_api/tests.rs`의 도우미 `job_in`은 `&JobStore` 대신 `&AppState`를 받아요.
- 옮기거나, 이름을 바꾸거나, 지운 테스트는 없어요. 테스트 수는 그대로예요.

### 검증한 것

2026-10-09에 확인했어요.

| 커밋 | cargo 통과 | 실패 | 무시 |
| --- | ---: | ---: | ---: |
| 시작(`fix(library): word the placement reasons alike and say 1화보다 앞 without an episode count`) | 2,898 | 0 | 14 |
| `refactor(jobs): split store.rs by feature and move the place delegations into place/` | 2,898 | 0 | 14 |
| `refactor(jobs): ask one function whether a request ID was used before` | 2,898 | 0 | 14 |
| `refactor(jobs): split JobStore into JobViews, JobRequests and JobRun over the same Db`(이 결과를 담은 커밋) | 2,898 | 0 | 14 |

- cargo는 `cargo test --locked --workspace -j 4`예요. 가운데 두 행은 에이전트가 커밋마다 돌린 값이고, 마지막 행은 제가 다시 돌려 확인했어요.
- `cargo fmt --all --check`는 깨끗하고, `cargo clippy --workspace --all-targets -j 4`에 경고가 없어요.
- 파일을 나눈 커밋은 git의 이동 감지로 옮긴 줄(5,384줄)과 새로 쓴 줄을 나눠 보았어요. 새로 쓴 줄은 모듈 문서, `use`, `self.db`를 `self.db()`로 바꾼 것, 파일 사이에서 쓰도록 넓힌 가시성뿐이에요.
- 웹 코드는 테스트 밖에서 `JobRun`과 `Runner`를 쓰지 않아요. 에이전트가 `JobRun`을 trss-jobs 밖에 드러내지 않게 잠시 바꿔 보았을 때, trss-web은 컴파일됐고 trss-worker만 실패했어요.
- web/은 바뀌지 않아 `npm test`는 돌리지 않았어요. 테스트 실행 시간은 견주지 않았어요.

### 정리한 것

- 부르는 곳이 없던 `Runner::store()`를 지웠어요.
- `UploadSummary::counts`는 `store` 밖에서 쓰지 않아 `pub(super)`로 좁혔어요.
- `file_expect`는 테스트만 부르지만 그대로 뒀어요.

### 남은 것

- 웹이 `JobRun`을 쓰지 않는 것은 타입이 끝까지 막지는 못해요. trss-worker가 다른 크레이트라서 `JobRun::new`는 공개여야 하고, 웹도 `Db`로 하나를 만들 수는 있어요. 지금은 웹 코드에 `JobRun`이 없는 것을 검색으로 확인해요.
- trss-web 테스트는 worker가 쓰는 행을 준비하려고 `JobRun`을 만들어요. 이를 위해 `AppState`에 테스트에서만 쓰는 `db()`를 더했어요.
