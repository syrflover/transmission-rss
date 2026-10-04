# 0054 서버 브라우저의 새 창을 바로 설정하고 풀어줘요

- 상태: 완료 (2026-10-04, 새 창이 약 0.1초에 실행의 페이지가 되고 문서의 요청부터 차단 목록이 걸려요. 새 창의 최초 이동은 막지 못해요. 아래 "결과")
- 출처: [작업 화면 안의 인증과 브라우저 수명](../../../specs/jobs.md#작업-화면-안의-인증과-브라우저-수명), [0046의 한계](0046-find-in-browser.md#남은-한계)
- 막는 티켓: 없음

## 배경

2026-10-04에 0046의 실제 브라우저 이미지 시험(`find_sample`)에서 봤어요. 게시물의 `target="_blank"` 링크를 신뢰 클릭으로 열자, 새 창이 약 30초 뒤에야 실행의 페이지가 됐어요.

브라우저 풀(`crates/trss-browser/src/pool/driver.rs`)은 자동 연결(`waitForDebuggerOnStart`)로 붙은 target마다 `attach`에서 `configure`를 끝까지 기다린 뒤 `Runtime.runIfWaitingForDebugger`를 보내요. `configure`의 첫 명령 `Network.enable`이 디버거를 기다리는 새 창에서는 답하지 않아요. 그래서 명령 시간 한도(30초)가 지난 뒤 설정 없이 창을 풀어줘요.

그 결과는 두 가지예요.

- 그동안 직접 찾기의 원격 화면이 새 창을 따라가지 않아요.
- 새 창에는 광고·추적 차단 목록(`Network.setBlockedURLs`)이 걸리지 않아요.

처음 창과 다른 사이트의 프레임은 같은 길로 설정돼요. 이 둘은 지금 시험에서 문제가 없었어요.

## 작업

- 새 target의 설정 명령과 `Runtime.runIfWaitingForDebugger`를 답을 기다리지 않고 이 순서로 보낸 뒤 답을 함께 기다려요. Puppeteer가 쓰는 순서예요.
  - 브라우저는 한 세션의 명령을 받은 순서대로 처리해요. 그래서 차단 목록은 창이 첫 요청을 보내기 전에 걸려요.
- 설정 명령이 실패하거나 답하지 않아도 창은 바로 풀려야 해요. 실패는 지금처럼 로그에 남겨요.
- 처음 창·프레임·팝업 모두 같은 길을 써요.

## 완료 기준

| 입력·상태 | 기대 결과 |
| --- | --- |
| 실제 브라우저 이미지에서 게시물의 `target="_blank"` 링크를 신뢰 클릭으로 엶 | 새 창이 2초 안에 실행의 페이지가 되고, 풀의 설정 실패 로그가 없어요. |
| 새 창의 첫 문서가 차단 목록의 주소를 요청함 | 그 요청이 막혀요. |
| 처음 창과 다른 사이트의 프레임 | 지금처럼 차단 목록이 걸려요. |
| 설정 명령이 실패하는 target(가짜 연결) | 창은 바로 풀리고 실패가 로그에 남아요. |
| 0040·0041·0043·0046의 실제 브라우저 이미지 시험 | 모두 통과해요. |

## 결과

### 만든 것 (2026-10-04)

- **명령을 먼저 쓰는 길**: `Connection::send`(`crates/trss-browser/src/cdp.rs`)는 명령을 소켓으로 가는 큐에 호출한 그 자리에서 넣고 답을 기다리는 `Sent`를 돌려줘요. 기존 `command`는 `async fn`이라 처음 poll될 때에야 큐에 넣으므로, 순서를 보장하려면 poll 순서가 아니라 호출 순서로 큐에 넣는 이 길이 필요했어요. `command`도 이 길 위에 있어요.
- **같은 길을 쓰는 attach**: `attach`(`crates/trss-browser/src/pool/driver.rs`)는 page·iframe에 `Network.enable`(버퍼 한도 0) → `Network.setBlockedURLs` → `Target.setAutoAttach`를, 디버거를 기다리는 target에는 이어서 `Runtime.runIfWaitingForDebugger`를 답을 기다리지 않고 이 순서로 보내요. `runIfWaitingForDebugger`의 답이 오면 target을 준비된 것으로 알리고, 그 뒤 설정의 답 셋을 함께 기다려요. 그래서 답하지 않는 명령이 있어도 기다림은 명령 시간 한도 한 번이에요. 설정 명령이 실패하면 target마다 첫 실패 하나를 로그에 남기고(`Network.enable`·`Network.setBlockedURLs`만, 주소는 남기지 않아요), 답하지 않아도 target은 풀려 있어요. 실행이 도중에 끝나면 두 명령이 함께 실패하므로, 한 번만 남겨요. `setAutoAttach`의 거절은 전처럼 로그에 남기지 않아요. 처음 창·프레임·팝업이 모두 이 길 하나를 써요.
- **시험용 가짜 페이지**: 가짜 블로그 게시물(`crates/trss-subtitles/src/fake.rs`)에 `#probe-popup` 링크를 더했어요. 이 링크가 여는 새 창(`blob:` 주소라 불러올 것이 없어요)은 문서가 열리자마자 차단 목록의 주소와 목록에 없는 주소(`https://example.com/`)를 `fetch`(`no-cors`)로 요청하고, 결과를 제목에 써요(`PROBE_POPUP_DOC`).

### 검증한 것

| 완료 기준 | 근거 |
| --- | --- |
| 실제 브라우저 이미지에서 `target="_blank"` 링크의 신뢰 클릭 | 무시된 테스트 `find_sample`(2026-10-04, `ghcr.io/syrflover/trss-browser:local`)에서 새 창이 클릭 뒤 114 ms, 차단 확인용 새 창이 65 ms 만에 실행의 페이지가 됐어요(시험은 2초 안을 요구해요). 이전에는 약 30초였어요. 그 실행의 출력에 풀의 설정 실패 로그(`cannot set up`)가 없었어요. |
| 새 창의 첫 문서가 차단 목록의 주소를 요청함 | 같은 시험에서 `#probe-popup`이 연 새 창의 제목이 `ad blocked, other reached`였어요. 차단 목록을 비운 채 돌리면 `ad reached, other reached`로 시험이 실패해서, 시험이 차단 여부를 가려내는 것도 확인했어요. 차단 목록 주소를 새 창의 주소 자체로 연 시험도 해봤는데 막히지 않았어요(404 페이지가 떴어요). 새 창의 최초 이동 요청은 디버거가 붙기 전에 이미 브라우저가 시작해서인 것으로 보이지만, 원인은 따로 확인하지 않았어요. 아래 한계에 적었어요. |
| 처음 창과 다른 사이트의 프레임 | 같은 시험에서 `https://example.com/`을 넣은 iframe(다른 사이트라 target이에요)이 `ad blocked, other reached`였고, 차단 목록을 비우면 `ad reached, other reached`로 실패했어요. 처음 창은 기존 풀 시험(`a_run_is_set_up_before_anything_loads`, `every_page_is_set_up_new_tabs_and_popups_included`)이 계속 통과해요. |
| 설정 명령이 실패하는 target(가짜 연결) | `trss-browser`의 단위 시험 4개(`pool::driver::tests`)가 가짜 DevTools 소켓으로 확인해요. `Network.enable`의 답을 `Runtime.runIfWaitingForDebugger`가 온 뒤에야 보내는 연결에서 target이 5초 안에 풀리고 소켓에 `Network.enable`, `Network.setBlockedURLs`, `Target.setAutoAttach`, `Runtime.runIfWaitingForDebugger` 순서로, 모두 그 target의 세션으로 도착해요. `Network.enable`에 끝내 답하지 않는 연결에서도 2초 안에 풀려요. `Network.setBlockedURLs`를 거절하는 연결에서는 풀리고, 뒤 명령도 가고, 실패가 한 번 알려져요(`failed` 콜백이 로그를 남기는 자리예요). page가 아닌 target은 풀기만 하고, 이미 실행 중인 iframe은 설정만 해요. |
| 0040·0041·0043·0046의 실제 브라우저 이미지 시험 | 2026-10-04에 `remote_screen_docker`(1 통과), `auth_sample`(1 통과), `erulabo_sample`(2 통과, 실제 erulabo 시험은 확인을 누르지 않고 첫 화면에서 지켜보기만 해요), `winpng_sample`(`TRSS_WINPNG_POSTS`에 harne1.tistory.com 762·763을 주고 2 통과), `find_sample`(1 통과)이 모두 통과했어요. `trss-browser`의 실제 이미지 시험 `docker`(0039·0053)도 2개 통과했어요. |

최종 코드(2026-10-04, 독립 검토 뒤 고친 것까지 `c061bdd` 위의 작업 트리)에서 `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`와 `-p trss-subtitles --features test-hooks --all-targets`(경고 0), `cargo test --workspace --no-fail-fast`(2,191개 통과, 실패 0, 무시 10)가 통과했어요. 고친 뒤 `find_sample`을 다시 돌려 새 창 114 ms, 차단 확인용 새 창 63 ms로 통과했어요. 다른 실제 이미지 시험은 고치기 전 코드로 돌린 결과예요. 고친 곳은 풀어준 뒤 답을 기다리는 방식과 로그 횟수뿐이에요.

### 한계

- 풀은 새 창의 **최초 이동 요청**에는 차단 목록을 걸지 못해요. `window.open`이나 링크가 여는 새 창의 첫 주소는 디버거가 붙기 전에 브라우저가 이미 요청해서인 것으로 보여요(차단 목록 주소를 새 창의 주소로 직접 열면 막히지 않았어요). 차단은 그 문서가 불러오는 요청부터 걸려요. 이전(30초 동안 설정 없이 풀림)보다 나아졌지만, 광고 주소가 새 창의 주소 자체인 경우는 막지 못해요. 원인은 확인하지 않았고, 다룰 일이면 별도 티켓이에요.
- 시험은 새 창과 프레임에서 `fetch`로 차단을 봤어요. 광고 스크립트·이미지 같은 하위 자원과 실제 광고 사이트의 새 창은 보지 않았어요.
- `find_sample`의 차단 확인은 인터넷의 `example.com`에 닿는 것을 대조로 써요. 인터넷이 없으면 대조가 실패해서 시험이 실패해요.
