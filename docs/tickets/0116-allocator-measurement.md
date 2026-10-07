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
- 개발 환경(`dev/compose.sh`)에서 서버 DB 복사본으로 돌려요. 운영 compose의 메모리 한도를 그대로 쓰고, trss-worker와 trss-web을 서로 다른 물리 core 4개에 묶어요. 서버(J4105)가 4 core이고, tokio가 CPU 수만큼 스레드를 띄우기 때문이에요.
- 이미지마다 같은 데이터 상태에서 시작해요.
- 측정용 이미지와 패치는 저장소에 넣지 않아요. 고른 할당기로 바꾸는 일은 결과를 보고 정해요.
- 이 PC에서는 J4105의 절대 속도와 실제로 새 회차를 받는 부하를 측정하지 못해요.

## 완료 기준

| 관찰 | 기대 결과 |
| --- | --- |
| web 부하 | 브라우저처럼 동시 요청 6개로 작품 목록, 작품 상세, 표지 이미지 같은 읽기 요청을 되풀이한 시간, 지연 시간, CPU 시간(user·system), 최대 메모리가 이미지마다 있어요. |
| worker 부하 | 감시 폴더 다시 확인(`watch_rescan`)과 평소 cycle의 시간, CPU 시간, 최대 메모리가 이미지마다 있어요. |
| 메모리 한도 | 이미지마다 `oom_kill` 수가 있어요. |
| malloc의 비중 | perf로 본 malloc과 그 lock의 비중이 이미지마다 있어요. |
| 결정 | 사용자가 측정 값을 보고 할당기를 골랐고, 그 결정과 날짜가 이 티켓에 있어요. musl malloc이 아닌 것을 고르면 바꾸는 일을 배포(0081) 전에 하는지도 정해요. |
