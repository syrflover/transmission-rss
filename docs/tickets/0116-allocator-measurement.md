# 0116 운영 앱의 메모리 할당기를 측정해서 골라요

- 상태: 진행 중
- 출처: [0079](0079-musl-test-run.md)의 musl에서 느린 테스트
- 막는 티켓: 없음

## 작업

2026-10-07에 musl 테스트가 느린 원인을 따로 분석했어요(`06e63df`).

- 많은 테스트 스레드가 함께 SQLite를 쓰면, SQLite의 C 코드가 하는 할당이 musl malloc의 lock 하나에서 줄을 서요. Rust의 `#[global_allocator]`만 바꾸면 달라지지 않았고, C의 `malloc`까지 바꾸는 mimalloc `override`로 trss-core의 lib 테스트가 8.50초에서 1.42초가 됐어요(glibc 1.24초).
- mimalloc을 쓰면 `artwork_upload_peak` 테스트가 메모리 한도를 넘었고, `MIMALLOC_PURGE_DELAY=0`을 주면 통과했어요.
- 컨테이너의 `/tmp`가 디스크에 있던 것은 `f62c3e7`에서 tmpfs로 바꿨어요.

운영 앱도 musl malloc을 써요. 운영에서는 프로세스마다 SQLite 연결이 하나이고 `Mutex` 뒤에 있어서 테스트만큼 경쟁하지 않을 것으로 보지만, 측정한 값은 없어요.
서버의 컨테이너는 메모리 한도가 작아서(worker 256M, web 128M) 속도와 함께 최대 메모리도 기준이에요.
사용자는 배포하기 전에 이 PC에서 측정해서 고르기로 했어요(사용자 결정, 2026-10-07).

- 세 이미지를 견줘요. 지금의 musl, musl에 mimalloc `override`를 넣고 `MIMALLOC_PURGE_DELAY=0`을 준 것, glibc로 빌드해 glibc 이미지에 넣은 것이에요.
- 개발 환경(`dev/compose.sh`)에서 서버 DB 복사본으로 돌려요. 운영 compose의 한도(컨테이너마다 CPU 0.25개, 메모리 worker 256M·web 128M)를 그대로 써요. CPU 한도 때문에 tokio의 async worker thread는 하나이고, 동시성은 blocking thread에서 나와요.
- 이미지마다 같은 데이터 상태에서 시작해요.
- 측정용 이미지와 패치는 저장소에 넣지 않아요. 고른 할당기로 바꾸는 일은 결과를 보고 정해요.
- 이 PC에서는 J4105의 절대 속도와 실제로 새 회차를 받는 부하를 측정하지 못해요.

## 측정 결과

2026-10-07, `259c19e`에서 빌드한 세 이미지를 개발 PC(Ryzen 5 5600X, RAM 32 GB)의 개발 환경에서 견줬어요.

- 이미지
  - musl: 지금의 `Dockerfile` 그대로예요. 126 MB예요.
  - mimalloc: trss-core가 `libmimalloc-sys`의 `override`를 링크하고, 이미지에 `MIMALLOC_PURGE_DELAY=0`을 줬어요. 바이너리의 `malloc`이 `mi_malloc`과 같은 주소인 것을 `nm`으로 확인했어요. 127 MB예요.
  - glibc: `rust:1.99.0-bookworm`(musl 이미지와 같은 rustc 1.99.0)에서 빌드해 `debian:bookworm-slim`에 넣었어요(glibc 2.36). 235 MB예요.
- 데이터: 개발 환경의 데이터 폴더(서버 DB 복사본, 작품 88개, 감시 폴더 2개, 표지 61개)를 떠 두고, 측정마다 그 상태로 되돌렸어요.
- 한도: 운영 compose의 한도 그대로예요. trss-web과 trss-worker 모두 async worker thread가 하나였어요.
- 부하
  - web: 요청 113개(목록 13개, 작품 상세 88개, 표지 12개)를 2번 돌려 데운 뒤, 30번(3,390개)을 동시 6개로 보냈어요.
  - worker: 감시 폴더 2개를 차례로 다시 확인하는 일을 30번 했어요.
- 이미지마다 3번씩 순서를 섞어 측정했어요. 표는 평균이고 괄호는 최소–최대예요. page fault와 메모리 한도 이벤트는 그 뒤 이미지마다 한 번 더 돌려서 기록했어요.
- CPU 시간은 컨테이너 cgroup의 `cpu.stat`, 메모리는 `memory.peak`예요. 할당기의 비중은 `perf record -e cpu-clock:u`로 본 user 표본 중 할당기 함수의 비율이에요. 이 PC의 `perf_event_paranoid`가 2라서 kernel 표본은 보지 못했어요.

| 값 | musl | mimalloc | glibc |
| --- | --- | --- | --- |
| web 부하의 CPU 시간 | 5.56초 (5.46–5.70) | 4.96초 (4.86–5.07) | 4.17초 (4.15–4.19) |
| └ user / kernel | 3.79 / 1.77초 | 3.44 / 1.52초 | 3.21 / 0.96초 |
| web 부하의 걸린 시간 | 22.12초 (21.74–22.65) | 19.75초 (19.36–20.24) | 16.59초 (16.55–16.65) |
| web 요청 지연 p50 / p95 | 8.2 / 96.4 ms | 6.7 / 95.8 ms | 6.0 / 93.0 ms |
| 다시 확인 30번의 worker CPU 시간 | 1.49초 (1.49–1.50) | 1.01초 (1.00–1.02) | 0.89초 (0.88–0.90) |
| └ user / kernel | 0.85 / 0.64초 | 0.67 / 0.34초 | 0.62 / 0.27초 |
| web 최대 메모리 (한도 128M) | 11.1 MiB (10.9–11.2) | 128.0 MiB (128.0–128.0) | 29.9 MiB (27.9–31.4) |
| worker 최대 메모리 (한도 256M) | 14.9 MiB (14.3–16.2) | 68.3 MiB (62.9–71.7) | 28.6 MiB (27.7–29.9) |
| `oom_kill` | 0 | 0 | 0 |
| user 표본 중 할당기 (web / worker) | 16.5% / 21.9% | 10.4% / 12.2% | 14.4% / 22.2% |
| └ 그중 musl의 lock 함수 | 3.6% / 2.1% | – | – |
| page fault (web 부하 / 다시 확인) | 133,825 / 125,819 | 86,471 / 24,109 | 3,188 / 2,637 |
| web이 한도에 닿은 횟수(`memory.events`의 `max`) | 0 | 41 (major fault 600) | 0 |

- 오류는 없었어요. 요청 3,390개가 모두 성공했고, 다시 확인은 모두 `done`이었어요. 첫 cycle이 끝나기까지의 시간은 세 이미지 모두 약 20초였어요. 이 시간은 피드를 받는 네트워크가 정해요.
- musl은 user 시간의 할당기 비중이 glibc와 비슷했어요. 차이는 kernel 시간이었어요. page fault가 glibc의 40배 이상이었어요. musl의 할당기가 해제한 메모리를 곧바로 kernel에 돌려주고 다시 받는 것으로 보여요. 그래서 메모리는 가장 적고 kernel 시간은 가장 길어요. lock 함수의 비중은 user 표본의 2–4%였어요.
- glibc는 CPU 시간이 가장 적었어요. musl보다 web 부하에서 25%, 다시 확인에서 40% 적었어요. 대신 최대 메모리가 web 2.7배, worker 1.9배였어요. 한도와는 아직 거리가 있어요.
- mimalloc은 web이 128M 한도에 41번 닿았어요. major fault도 600번 났어요. 한도에 닿아 회수된 파일 페이지(실행 파일 같은)를 디스크에서 다시 읽은 것으로 보여요. 지금 설정으로는 쓸 수 없어요.
- p95가 세 이미지 모두 약 95 ms인 것은 CPU 0.25개 한도의 throttling 주기(100 ms) 때문이에요.
- J4105는 이 PC보다 core가 느려서 같은 CPU 시간이 더 긴 걸린 시간이 돼요. 서버의 절대값은 측정하지 않았어요.
- 할당기와 별개로, web의 user 표본에서 SQL을 해석하는 함수(`sqlite3RunParser`, `yy_reduce`, `sqlite3GetToken`, `keywordCode`)가 glibc에서 10.3%, musl에서 8.2%였어요. 코드에 `prepare`가 175곳 있고 `prepare_cached`는 없어서, 요청마다 SQL을 다시 해석해요.

## 완료 기준

| 관찰 | 기대 결과 |
| --- | --- |
| web 부하 | 브라우저처럼 동시 요청 6개로 작품 목록, 작품 상세, 표지 이미지 같은 읽기 요청을 되풀이한 시간, 지연 시간, CPU 시간(user·system), 최대 메모리가 이미지마다 있어요. |
| worker 부하 | 감시 폴더 다시 확인(`watch_rescan`)과 평소 cycle의 시간, CPU 시간, 최대 메모리가 이미지마다 있어요. |
| 메모리 한도 | 이미지마다 `oom_kill` 수가 있어요. |
| malloc의 비중 | perf로 본 malloc과 그 lock의 비중이 이미지마다 있어요. |
| 결정 | 사용자가 측정 값을 보고 할당기를 골랐고, 그 결정과 날짜가 이 티켓에 있어요. musl malloc이 아닌 것을 고르면 바꾸는 일을 배포(0081) 전에 하는지도 정해요. |
