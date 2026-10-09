# 0112 남은 작은 도우미와 테스트 도우미를 모아요

- 상태: 완료
- 출처: [흩어진 개념 모으기](refactoring.md#흩어진-개념-모으기), [그 밖의 테스트 정리](refactoring.md#그-밖의-테스트-정리)
- 막는 티켓: [0092](0092-refactor-baseline.md)(기준값)

## 작업

2026-10-07에 본, 제품 코드의 작은 복사본이에요.

- trss-web이 명령을 맡긴 뒤 worker를 깨우는 같은 세 줄이 14곳에 있어요.
- 바쁨을 응답으로 바꾸는 같은 함수가 trss-web의 표지 API와 시즌 API에 있어요.
- `/proc/net/route`를 읽는 코드가 trss-browser의 나가는 연결 검사와 trss-web의 브라우저 네트워크에 있어요.

테스트 도우미의 복사본도 모아요.

- 가짜 AniList와 가짜 Anissia는 같은 뼈대(시작, 설정, 요청 기록, `429`와 `Retry-After`, 실패, 덧붙임)를 각자 가져요.
- trss-web 테스트 모듈 19곳이 요청 도우미를 각자 가져요.
- ZIP을 만드는 도우미가 4벌, 루프백 주소에 서버를 띄우는 코드가 약 14곳에 있어요.
- trss-subtitles의 `testing.rs`(약 2,000줄)는 앞부분이 도우미이고 뒷부분에 테스트 20개가 섞여 있어요. 테스트를 도우미와 떼요.

## 완료 기준

- 위의 복사본이 각각 한 곳에 있어요.
- 통과·건너뛴 테스트 수가 앞뒤로 같아요.

## 결과

### 결정

- ZIP을 만드는 테스트 도우미는 ZIP 형식의 주인인 trss-archive의 `testing` 모듈에 둬요(사용자 결정, 2026-10-10). 사용자가 코드베이스 설계로 어느 쪽이 이상적인지 물었고, 시험용 ZIP의 꼴도 형식의 지식이라 주인 크레이트에 두는 쪽을 골랐어요. 그래서 테스트에서만 생기는 trss-subtitles → trss-archive 의존이 생겼고, trss-archive는 자기 `tests/it`이 feature를 보도록 자기 자신을 dev-dependency로 적어요.
- 아래는 리드가 정했어요.
  - 완료 기준의 "통과·건너뛴 테스트 수가 앞뒤로 같아요"는 "잃은 테스트가 없어요"로 읽었어요. [흩어진 개념 모으기](refactoring.md#흩어진-개념-모으기)가 모은 부품에는 그 크레이트의 테스트를 두라고 하므로, 새 공유 부품 여섯에 테스트를 더했어요. 기존 테스트는 지우거나 이름을 바꾸거나 확인하는 내용을 바꾸지 않았어요.
  - trss-core에는 feature 뒤로도 axum을 두지 않아요. `cargo test --workspace`는 크레이트마다 feature를 합치므로, axum이 모든 크레이트의 뿌리인 trss-core보다 먼저 빌드되어야 하게 되기 때문이에요. 그래서 trss-core의 공유 부품은 axum 없이 짜고, 가짜 서버마다 그것을 axum 응답으로 바꿔요.
  - 가짜 Transmission의 주소 잡기는 그대로 둬요. `restart`가 앞서 쓴 주소에 다시 붙고, trss-transmission은 trss-core를 의존하지 않아요.
  - 웹 테스트의 요청 도우미는 모듈마다의 `App::call` 같은 이름과 서명을 그대로 둔 얇은 감싸개로 남겨, 약 500곳의 호출과 URI를 바꾸지 않았어요. `/api` 없이 라우터를 만드는 여섯 모듈도 그대로예요.

### 바꾼 것

| 커밋 | 내용 |
| --- | --- |
| `refactor: wake the worker through one function in core and one method on the web state (0112)` | 웹 16곳과 worker 2곳의 깨우기를 trss-core `wake::wake_worker_if`와 trss-web `AppState::wake_worker`로 모았어요. 부르는 조건은 곳마다 그대로예요. |
| `refactor(web): turn an AniList failure into an API answer in one place (0112)` | 표지 API와 시즌 API의 `busy`와 그 옆 두 갈래를 trss-web `error.rs`의 `From<AnilistError> for ApiError` 하나로 모았어요. 문구와 상태 코드는 그대로예요. |
| `refactor: read the routing table in one core module for the web and the egress proxy (0112)` | 웹의 브라우저 네트워크와 trss-browser의 송신 프록시가 trss-core `net_route`(`PATH`, `Route`, `parse`)로 `/proc/net/route`를 읽어요. |
| `refactor(web): build, send and read the API test requests in one test module (0112)` | 웹 테스트 25개 파일의 요청 도우미를 `src/testing.rs`(`api`, `bare_api`, `request`, `send`, `send_text`, `call`, `call_text`)로 모았어요. |
| `refactor(subtitles): keep the tests of the test server in their own file (0112)` | `testing.rs`의 테스트 20개를 글자 그대로 `testing/tests.rs`로 옮겼어요. |
| `refactor(test): start the loopback servers of the tests through one trss-core module (0112)` | 테스트와 가짜 서버가 루프백 주소에 서버를 띄우는 22곳과 빈 포트를 잡는 5곳이 trss-core `loopback`(`bind`, `bind_on`, `serve`, `serve_on`, `unused_addr`)을 써요. |
| `refactor(test): share the axum-free parts of the fake AniList and Anissia servers in trss-core (0112)` | 가짜 AniList와 Anissia의 429·500 결정, JSON 덧붙임, 64 KiB 조각 나누기를 trss-core `fake_http`로 모았어요. 가짜 피드와 피드 테스트의 조각 나누기도 그것을 써요. |
| `refactor(test): build the sample ZIPs of the tests in one trss-archive module (0112)` | trss-subtitles `verify::zip_of`, trss-jobs의 두 복사본, 테스트 안에서 바로 만들던 보통 ZIP 네 곳을 trss-archive `testing`(`zip_of`, `deflated_zip`, `zip_with_dirs`)으로 모았어요. |
| `refactor(web): name the route table's path from net_route in the read error (0112)` | 독립 검토가 찾은 것으로, 웹의 읽기 오류 문구가 경로를 다시 적지 않고 `net_route::PATH`를 써요. 문구는 같아요. |
| `docs: record the result of gathering the small and test helpers (0112)`(이 결과를 담은 커밋) | 이 결과, 검증 표가 인용하는 테스트 파일 경로, 계획 문서예요. |

- 새 테스트 도우미 모듈(trss-core `loopback`, `fake_http`, trss-archive `testing`)은 `#[cfg(any(test, feature = "test-support"))]` 뒤에 있고, 쓰는 크레이트는 `[dev-dependencies]`에서만 feature를 켜요. trss-core `net_route`는 제품 코드라 feature 없이 있어요. trss-core의 `test-support`는 `tokio/net`을 더 켜요.
- 제품 빌드에 테스트 feature가 들어가지 않는 것은 [0097](0097-shared-transmission-fake.md)과 같은 방법으로 확인했어요. `cargo tree -p <크레이트> -e normal,build -e features`에서 `feature "test-support"`가 trss-worker, trss-web, trss-probe, trss-browser, trss-jobs 모두 0번이었어요(2026-10-10, `e957b9e`). `dev`를 더하면 trss-web에서 11번 나와서, 이 검사가 feature를 찾아낼 수 있다는 것도 확인했어요.

### 남긴 차이

- `/proc/net/route`를 읽지 못하면 웹은 시작하지 않고, 송신 프록시는 빈 표로 봐요. 웹은 metric이 숫자가 아닌 줄을 기본 경로 찾기에서 버리고, 프록시는 `lo`와 접두 8 미만을 걸러요. 이 규칙과 IPv6 표 읽기는 부르는 쪽에 그대로 있어요.
- Anissia의 바쁨 문구(`요청이 몰려`)는 AniList와 문구·상태 코드가 달라서 따로 둬요. `seasons_anissia_api.rs`가 그 문구로 안내를 고르기도 해요.
- 가짜 AniList는 `Retry-After`를 늘 보내고 `extensions.padding` 아래에 덧붙이며 404 답에도 덧붙여요. 가짜 Anissia는 초가 없으면 머리글을 보내지 않고 `padding` 아래에 덧붙이며 200 답에만 덧붙여요. 공유 함수가 이 차이를 인자로 받아요.
- 두 가짜가 axum 응답으로 바꾸는 약 12줄(거절을 상태 코드와 `Retry-After`로, 조각을 `Body::from_stream`으로)은 아직 두 벌이에요. trss-core에 axum을 두지 않는 결정 때문이에요.
- nyaa 가짜의 거절은 정해 둔 상태 코드와 빈 본문을 세지 않고 돌려줘서 공유 결정과 모양이 달라요. 테스트가 그 필드를 바로 쓰므로 그대로 뒀어요.
- trss-archive 테스트가 바이트를 손으로 짜는 ZIP(CP949 이름, Unix 권한, 장치), 흘려 쓰는 압축 폭탄, 50만 멤버로 ZIP64를 일으키는 경우, 개발 환경의 제품 코드인 trss-subtitles `fake::zip_of`는 그대로 둬요.
- 파일 이름 다듬기와 실행 task의 종료 유예는 0096과 0108의 결론대로 남은 복사본이 없어서 다루지 않았어요.
- 티켓에 없던 것으로, trss-subtitles `verify::claimed_members`와 trss-archive `zip::read_end`가 둘 다 ZIP의 끝 기록을 읽어요. 앞의 것은 꼬리의 모든 후보를 한도와 견주고(주석 속 가짜 기록 방어), 뒤의 것은 마지막으로 맞는 기록 하나와 여러 디스크 여부를 읽어서 일부러 달라요. 옮기면 압축 폭탄 방어가 바뀔 수 있어 이 티켓에서는 기록만 해요.
- 웹 테스트의 `AppState::new(Db::open_blocking(":memory:"))` 22벌은 요청 도우미가 아니라서 그대로예요.

### 테스트

| 파일 | 앞 | 뒤 |
| --- | ---: | ---: |
| trss-core `wake.rs` | 2 | 3 |
| trss-core `net_route.rs`(새 파일) | 0 | 5 |
| trss-core `loopback.rs`(새 파일) | 0 | 6 |
| trss-core `fake_http.rs`(새 파일) | 0 | 7 |
| trss-web `error.rs` | 3 | 4 |
| trss-archive `testing.rs`(새 파일) | 0 | 6 |

- 다른 파일의 테스트 수는 바뀌지 않았어요. trss-subtitles `testing/tests.rs`의 20개는 경로(`testing::tests::*`)도 같아요.
- 새 ZIP 도우미의 테스트는 옮긴 테스트가 기대는 꼴을 확인해요. 크기와 CRC가 로컬 헤더에 있고 data descriptor가 없으며, 끝 기록이 `len - 22`에 주석 없이 있고, ZIP64 locator가 없어요.
- `/proc/net/route` 옮기기는 신뢰 경계라 독립 검토를 받았어요. 머리 줄, 짧은 줄, 잘못된 16진수, 숫자가 아닌 metric, `lo`, CRLF와 탭에서 옛 결과와 같다는 결론이었고, 막는 문제는 없었어요. 검토는 코드만 읽었고 빌드는 하지 않았어요.

### 확인

- workspace 테스트(`cargo test --locked --workspace -j 4`)는 `dacb400`의 3,011개 통과에서 3,037개 통과가 됐어요(2026-10-10, `e957b9e`). 실패는 0개, 무시는 14개예요. 늘어난 26개는 위 표의 새 테스트예요.
- 마지막 커밋(`ac2a292`)은 오류 문구 하나를 바꿔서 trss-web의 `browser_net` 테스트 6개만 다시 돌렸고, 모두 통과했어요.
- 커밋마다 `cargo fmt --all --check`와 `cargo clippy --locked --workspace --all-targets -j 4`에 경고가 없었어요. 무시되는 Docker 테스트는 빌드(`--no-run`)만 확인했고 Docker로 돌리지 않았어요.
- 코드 줄 수는 `dacb400`에서 90개 파일, 2,360줄을 더하고 2,152줄을 지워 208줄 늘었어요. 이 가운데 테스트 20개를 다른 파일로 옮긴 커밋이 1,108줄을 더하고 1,113줄을 지웠어요. 새 공유 모듈 넷(`net_route` 170줄, `loopback` 166줄, `fake_http` 149줄, archive `testing` 175줄)은 테스트를 포함해 660줄이에요. 웹 테스트 도우미를 모은 커밋은 400줄을 줄였어요.
