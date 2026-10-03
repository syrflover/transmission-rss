# 0039 서버 브라우저 컨테이너와 수명을 다뤄요

- 상태: 완료
- 출처: [작업 화면 안의 인증과 브라우저 수명](../specs/jobs.md#작업-화면-안의-인증과-브라우저-수명), [인증 운영 ADR](../adr/0002-server-browser-subtitle-authentication.md), [공통 정책](../specs/settings.md#공통-정책)
- 막는 티켓: [0032](0032-server-browser-spike.md)(원격 화면 방식과 자원 한도), [0036](0036-subtitle-jobs-foundation.md)

## 작업

worker가 작업에 쓸 서버 Chromium을 띄우고 거두는 기반이에요. 원격 화면을 사용자에게 보여주는 일은 [0040](0040-remote-auth-screen.md)이에요.

- Chromium은 앱과 분리한 비공개 컨테이너로 compose에 더해요. 0032에서 정한 대로 가상 디스플레이 위의 창 있는 Chromium(1배)이고, VNC 서버·창 관리자는 두지 않으며, 메모리 한도 시작값은 768MiB예요. CDP 포트는 호스트에 열지 않고, worker와만 나누는 별도 네트워크 안에서만 닿아요.
- 실행마다 광고·추적 주소를 막아요. 인증 서비스 주소는 막지 않아요.
- 0032의 장치는 Selenium 이미지를 썼어요. 제품 이미지는 Chromium과 가상 디스플레이만 담은 것으로 새로 정하고, 바꾼 이미지에서 erulabo 인증이 여전히 되는지 사람이 확인해요.
- worker가 작업마다 브라우저 실행을 시작·종료하고, 실행마다 새 프로필을 쓰며 종료할 때 쿠키·사이트 저장 상태를 버려요. 받은 파일은 공유한 다운로드 경로에서 작업의 수신 영역으로 옮겨요.
- 작업을 실행 중이지 않고 사용자 조작도 없는 유휴 시간(기본 5분)이 지나면 브라우저를 닫고, 인증 필요 작업은 남겨요. 상태 조회는 브라우저를 띄우거나 조작 시각을 갱신하지 않아요.
- 동시 브라우저 작업은 정책의 상한(기본 1개)을 지키고, 상한을 넘는 작업은 기다려요.
- 브라우저 실행은 작업에 묶고, 끝난 실행의 연결로 다음 작업을 조작할 수 없게 해요.

## 완료 기준

| 입력·상태 | 기대 결과 |
| --- | --- |
| 브라우저가 필요한 작업(가짜 출처 페이지) | Chromium 컨테이너가 시작되고, 서버 브라우저가 받은 파일이 작업의 수신 영역에 있어요. |
| 유휴 5분(시험에서는 짧게 설정) | 브라우저가 닫히고 작업과 받은 파일·기록은 남아요. 실행 중 다운로드는 유휴로 끊기지 않아요. |
| 상태만 조회하며 기다림 | 브라우저가 새로 뜨지 않고 유휴 시각이 늘지 않아요. |
| 실행을 닫고 다시 띄움 | 이전 실행의 쿠키·사이트 저장 상태가 남지 않아요. |
| 동시 상한 1에서 브라우저 작업 둘 | 하나만 실행되고 다른 하나는 기다려요. 상한 2로 바꾸면 둘의 화면·프로필·파일이 섞이지 않아요. |
| 호스트에서 CDP 포트 접속 시도 | 닿지 않아요. |
| ~~실제 erulabo 게시물, 768MiB 한도의 제품 이미지~~ | [0040](0040-remote-auth-screen.md)으로 옮겼어요. 사람이 서버 브라우저를 볼 원격 화면이 0040에 있어서예요(사용자 결정, 2026-10-03). |
| worker 재시작 | 남은 브라우저 실행을 정리하고 인증 필요 작업은 그대로 기다려요. |

## 결과

### 만든 것 (2026-10-03)

- **설계**(사용자 결정, 2026-10-03): 컨테이너 안 작은 실행기예요. 항상 떠 있는 `trss-browser` 컨테이너에 Xvfb와 실행기 `trss-browserd`만 두고, worker가 HTTP와 bearer 토큰으로 실행의 시작·종료를 요청해요. Docker 소켓은 쓰지 않아요.
- **크레이트 `trss-browser`**: 실행기 바이너리(실행마다 새 프로필의 창 있는 Chromium, 끝나면 프로필과 다운로드 폴더를 지움, 실행별 CDP WebSocket 중계, 모든 경로에 토큰)와 worker 쪽 `BrowserPool`·`BrowserRun`·`Page`(작업마다 하나인 실행, 정책의 동시 상한과 유휴 시간, 광고·추적 주소 차단, 다운로드를 작업 폴더로 옮기기)예요. 시험용 `trss-fake-chromium`도 있어요.
- **신뢰 경계**: 실행기는 호스트에 포트를 열지 않고, trss-web·Transmission이 있는 `trss_net`이 아닌 worker와만 나누는 `browser_net`에 있어요. 토큰은 상수 시간으로 견주고, 로그·오류·`Debug`·Chromium 환경에 남기지 않아요. 브라우저가 쓸 수 있는 다운로드 폴더는 믿지 않아서, UUID 이름의 한 이름뿐인 일반 파일만 링크를 따르지 않고 옮겨요. 파일 하나 200MiB, 실행 합계 1GiB, 120초 정체에서 다운로드를 취소해요.
- **수명**: 유휴 종료와 작업 단계 시작은 한 잠금에서 정해져서 둘 다 받아들여지는 일이 없어요. 시작 요청이 끊겨도 실행기가 프로세스·폴더를 남기지 않고, worker가 다시 시작하면 남은 실행을 정리해요. 실행기 하나에는 worker 하나만 붙어요(`<db>.browser.lock`).
- **배포**: `Dockerfile.browser`, compose의 `trss-browser` 서비스(768MiB, `cap_drop: ALL`, `no-new-privileges`, tini), `deploy.yml`의 두 번째 이미지 빌드, 개발 환경(`dev/compose.sh`, `dev/dev.env`). **서버 `.env`에 `TRSS_BROWSER_TOKEN`이 있어야 compose가 시작돼요.**
- 어떤 출처도 아직 풀을 쓰지 않아요. erulabo(0041)·직접 찾기(0046)·WinPNG(0043)가 이 위에 붙어요.
- 규칙 전체와 측정값은 [작업 화면 안의 인증과 브라우저 수명](../specs/jobs.md#작업-화면-안의-인증과-브라우저-수명)에 있어요.

### 검증한 것

| 완료 기준 | 근거 |
| --- | --- |
| 브라우저가 필요한 작업 | 풀 수준: `a_run_is_set_up_before_anything_loads`, `a_download_is_moved_into_the_given_folder_by_its_suggested_name`, 실제 이미지의 무시된 시험 `tests/docker.rs`(실제 다운로드를 옮김). 출처가 풀을 쓰지 않아 작업 전체로는 확인하지 않았어요(0041에서). |
| 유휴 종료 | `the_reaper_ends_a_run_that_has_been_idle_past_the_idle_time`, `a_download_under_way_is_not_cut_by_the_idle_time`, `a_running_job_step_keeps_the_run_and_the_idle_time_counts_from_its_end`, `a_step_and_an_idle_end_are_never_both_accepted` |
| 상태만 조회 | `status_queries_start_nothing_and_do_not_move_the_idle_time` |
| 쿠키·저장 상태가 남지 않음 | 실제 이미지 시험에서 실행 중 쿠키가 프로필에 있고 실행 뒤 사라짐 |
| 동시 상한 | `with_a_cap_of_one_the_second_job_waits_for_the_first`, `a_raised_cap_lets_the_waiting_job_in_and_the_two_runs_stay_apart`, `an_ended_run_refuses_every_operation_and_cannot_reach_the_next_one` |
| 호스트에서 CDP 포트 | `the_browser_container_publishes_no_port`, `the_browser_shares_a_network_with_the_worker_alone_and_is_not_on_trss_net`, 실제 이미지 시험(호스트에 열린 포트 없음) |
| worker 재시작 | `a_new_pool_resets_the_launcher_and_empties_the_downloads_folder`, 실제 이미지 시험(재시작한 worker가 남은 실행을 정리). 인증 필요 작업이 그대로인지는 출처가 붙은 뒤(0041) 확인해요. |

작업 공간 시험 1,997개가 통과하고(실제 네트워크·docker 시험 4개는 무시), clippy 경고가 없어요. 실제 이미지 시험은 2026-10-03에 두 번(검토 보강 전후) 통과했어요. 개발 환경(`dev/compose.sh up`)에서 두 이미지가 빌드되고 `trss-browser`가 Xvfb와 실행기만으로 약 14MiB에서 쉬었어요. 브라우저는 `browser_net`에만, worker는 `browser_net`과 `trss_net`에, trss-web은 `trss_net`에만 있었어요. worker는 `The server browser is on`을 남겼고 로그에 토큰이 없었어요.

### 독립 리뷰

신뢰 경계와 동시성을 두고 리뷰를 받았어요. 막는 결함은 없었고 아래를 모두 고쳤어요.

| 지적 | 처리 |
| --- | --- |
| 브라우저가 다운로드 폴더에 심은 symlink·하드 링크·`..` 이름을 따라 옮김 | UUID 이름, `O_NOFOLLOW`로 연 폴더 기준 이동, 한 이름뿐인 일반 파일만 |
| 브라우저가 trss-web·Transmission과 같은 네트워크 | `browser_net`으로 떼어 냄 |
| `Network.enable`이 응답 본문을 버퍼에 쌓음 | 버퍼 크기 0 |
| 유휴 종료와 `mark_busy`가 둘 다 받아들여짐 | 한 잠금에서 정하고, 기존 실행을 돌려받는 것도 사용으로 셈 |
| 끊긴 시작 요청이 프로세스·폴더를 남김 | 시작을 따로 돌리고, 끝내기가 진행 중 시작을 취소하고 기다림, 폴더를 지움 |
| 목록을 읽은 뒤 준비된 실행을 사라진 것으로 봄 | 목록 전에 준비된 실행을 찍어 둠 |
| 멈춘·끝없는 다운로드가 실행을 붙듦, 이벤트 지연(`Lagged`) | 크기·정체 한도와 취소, 지연이면 실행을 끝냄 |
| 두 worker가 서로의 실행을 정리 | worker 하나만 브라우저를 씀(잠금) |
| `PoolConfig`의 `Debug`와 Chromium 환경에 토큰 | 가리고 지움 |
| 다른 파일 시스템 복사 뒤 원본이 없으면 오류 | 성공으로 봄 |
| 문서가 남은 일을 다르게 적음 | 명세·README·ADR 0002를 고침 |

### 한계

- 다운로드 한도(200MiB, 1GiB, 120초)는 측정이 아닌 시작값이에요.
- `browser_net`은 보통 bridge라서 호스트 LAN과 Docker 게이트웨이에는 닿아요. 막으려면 호스트 방화벽이 필요해요.
- 정책의 동시 상한을 3으로 올리면 768MiB에 들지 않아요. 한도와 함께 올려야 한다고 compose와 README에 적었어요.
- 메모리 측정은 두 번이 달랐어요(파일 캐시 포함 286MiB와 768MiB까지, 익명 메모리 175~199MiB). 원인은 확인하지 않았고 CPU는 재지 않았어요. `deploy.yml` 단계는 돌려 보지 않았어요.
- 실행의 다운로드 폴더는 실행이 끝날 때 지워져서, 아직 옮기지 않은 다운로드는 사라져요.
