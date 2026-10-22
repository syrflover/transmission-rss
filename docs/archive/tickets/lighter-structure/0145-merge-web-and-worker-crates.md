# 0145 `trss-web`과 `trss-worker` 크레이트를 `trss` 크레이트 하나로 합쳐요

- 상태: 완료 (2026-10-21, 아래 "결과")
- 출처: [구조 가볍게 하기 명세의 한 프로세스와 이름](lighter-structure.md#한-프로세스와-이름), [한 프로세스 ADR](../../../adr/0019-one-process-for-web-and-worker.md)
- 막는 티켓: [0144](0144-lighter-structure-baseline.md)(기준값)

## 작업

web과 worker를 한 프로세스로 합치는 첫 단계로, 두 크레이트의 코드와 테스트를 `trss` 크레이트 하나로 옮겨요. 이 티켓에서는 바이너리가 아직 `trss-web`과 `trss-worker` 둘이고, 이미지와 Compose의 서비스도 그대로예요. 실행을 하나로 합치는 일은 [0146](0146-one-trss-process-and-container.md)이 해요. 이렇게 나누면 코드를 옮기는 큰 변경이 동작을 바꾸지 않은 채 따로 들어가고, 프로세스를 합치는 변경은 실행 쪽만 다뤄요.

- 크레이트 이름은 `trss`예요(사용자 결정, 2026-10-21). 기능 크레이트는 그대로예요.
- web의 모듈과 worker의 모듈은 이름이 겹치지 않게 한 크레이트 안에 두고, 두 바이너리는 같은 라이브러리를 써요.
- `trss-worker`의 `tests/it`은 `trss`의 `tests/it`이 되고, web의 메모리 최고치 테스트 3개와 `remote_screen_docker`는 자기 바이너리를 그대로 가져요. 테스트를 테스트 나눔 ADR에 맞추는 일은 [0150](0150-trss-crate-tests-by-adr-0015.md)이 해요.
- 지금 문서는 `trss-web`이나 `trss-worker`의 경로로 코드와 테스트를 가리켜요. 2026-10-21에 명세 6개와 readme에서 두 이름이 나온 줄은 228줄이었어요. 완료한 티켓, 보관한 티켓, ADR은 그때의 기록이라 고치지 않아요.

## 완료 기준

- [x] `trss-web`과 `trss-worker` 크레이트가 없고, 두 크레이트의 코드는 `trss` 크레이트에 있어요. `trss` 크레이트가 `trss-web`과 `trss-worker` 바이너리를 만들고, 이미지와 Compose는 지금처럼 그 둘을 써요.
- [x] 통과·건너뛴 테스트 수가 앞뒤로 같고, 결과에 그 수를 적어요.
- [x] `docker compose`로 띄운 개발 환경이 지금처럼 돌아요. 구독 추가의 받기 하나를 실행해 확인하고 결과에 적어요.
- [x] 같은 커밋에서, 지금 코드를 적는 문서가 옮긴 파일과 테스트를 새 경로로 가리켜요. 명세, 검증 표, readme, 개요, 용어집, 열린 티켓이 대상이에요. 결과에 찾은 방법과 고친 문서를 적어요.
- [x] `dev/compose.sh`, `dev/measure/`의 스크립트, `deploy/probe.sh`, `Dockerfile`이 크레이트 경로를 쓰면 새 경로로 고쳐요.
- [x] [명세의 동작 보존 조건](lighter-structure.md#동작-보존)을 지켜요.

## 결과

2026-10-21에 두 크레이트의 파일을 `git mv`로 `crates/trss`에 옮겼어요. 두 라이브러리에 같은 이름의 `env` 모듈이 있어서, 각 라이브러리를 `web`과 `worker` 모듈로 그대로 뒀어요.

```
crates/trss/
├── src/
│   ├── lib.rs          web과 worker 모듈만 선언해요
│   ├── web.rs, web/    예전 trss-web의 라이브러리
│   ├── worker.rs, worker/   예전 trss-worker의 라이브러리
│   └── bin/
│       ├── trss-web.rs
│       └── trss-worker.rs
└── tests/
    ├── it/, fixtures/  예전 trss-worker의 tests/
    └── artwork_serving_peak.rs, artwork_upload_peak.rs,
        subtitle_upload_peak.rs, remote_screen_docker.rs
```

코드는 경로만 바꿨어요. 크레이트 안의 `crate::…`는 `crate::web::…`나 `crate::worker::…`로, 바이너리와 테스트의 `trss_web::…`·`trss_worker::…`는 `trss::web::…`·`trss::worker::…`로 바꿨어요. 파일이 한 단계 깊어져서, 다른 크레이트의 fixture를 읽는 `include_str!` 하나에 `../`를 더했어요. `Cargo.toml`의 의존성은 두 크레이트의 것을 합쳤고, `Cargo.lock`에서는 워크스페이스 패키지만 바뀌었어요.

### 테스트

`cargo test --locked --workspace -j 4`를 빌드한 뒤 실행하니 3,086개가 통과하고, 실패 0개, 건너뛴 것 14개였어요. [0144](0144-lighter-structure-baseline.md)의 `d8432c8a`와 같은 수예요. 테스트 바이너리는 31개에서 30개가 됐어요. `trss-web`과 `trss-worker`의 단위 테스트(468개와 12개)가 `trss`의 단위 테스트 바이너리 하나(480개)로 합쳐졌기 때문이에요. 바이너리 시간의 합은 둘 다 43.0초였어요. `cargo clippy --workspace --all-targets`에 경고가 없고, `cargo fmt`는 경로가 길어진 줄을 다시 나누고 `use` 줄의 순서를 맞췄어요. 원본 로그는 이 PC의 `dev/local/measure/0145/`에 있어요.

### 개발 환경

`dev/compose.sh up`이 이 작업 트리로 이미지를 빌드해 `trss-web`, `trss-worker`, `trss-browser`, Transmission을 띄웠어요. worker는 수집 주기, 감시 폴더 읽기, Anissia 관찰을 돌렸고, web은 8080에서 응답했어요.

내장 브라우저에서 `편성표에서 추가`로 `명탐정 코난`을 subsplease.org 채널, 릴리스 제목 `Detective Conan`으로 구독하며, 이미 기록된 항목 `[SubsPlease] Detective Conan - 1217 (1080p) [845D5162].mkv` 하나를 체크했어요. worker는 `receive_once` 명령을 한 번 시도해 17초 뒤 `received`로 마쳤고, 화면에는 `추가함`이 보였어요. 확인한 뒤 개발 환경의 `dev/local/data`와 `dev/local/transmission-config`를 확인 전에 떠 둔 사본으로 되돌렸어요.

### 문서와 스크립트

두 크레이트의 코드와 테스트를 가리키는 곳은 `trss-web`, `trss-worker`, `trss_web`, `trss_worker`로 찾았어요. 찾은 범위는 명세 6개, readme, `web/README.md`, 개요, 용어집, 티켓 목록, 열린 티켓이에요.

- 검증 표의 `` `크레이트` `경로` `` 인용 251개(`trss-web` 221개, `trss-worker` 30개)는 스크립트로 바꿨어요. 경로 225개가 `web/…`, `src/web/…`, `worker/…`, `src/worker/…`로 바뀌었고, `tests/…`는 그대로예요. 바꾼 뒤 명세가 인용한 Rust 테스트가 모두 인용한 파일에 있고, 화면 테스트 인용 48개도 그대로 찾아지는 것을 확인했어요.
- 손으로 고친 곳은 `jobs.md`, `library.md`, `subtitles.md`의 코드 경로, `web-app.md`의 공통 라이브러리 모듈 구성 문장, 티켓 0129·0132·0134·0135예요.
- 다른 크레이트와 화면 코드의 주석이 두 크레이트를 가리키던 곳도 `trss::web`·`trss::worker`나 새 파일 경로로 고쳤어요. `remote_screen_docker`의 실행 명령은 `cargo test -p trss`가 됐어요.

바이너리, 컨테이너, Compose 서비스의 이름으로 쓴 `trss-web`·`trss-worker`는 그대로 뒀어요. readme, 개요, `web-app.md`의 실행 구성, 로그 문구가 여기에 들어가고, [0146](0146-one-trss-process-and-container.md)이 바꿔요. 날짜를 붙인 관찰(0157, 0160, 명세의 기준값)과 완료한 티켓, 보관한 티켓, ADR도 그대로예요.

`dev/compose.sh`, `dev/measure/`의 스크립트, `deploy/probe.sh`, `Dockerfile`은 바이너리와 서비스의 이름만 쓰고 크레이트 경로는 쓰지 않아서 고치지 않았어요. `Dockerfile`은 `cargo build --release --locked`로 워크스페이스의 바이너리를 모두 빌드해요.

확인하지 않은 것은 다음과 같아요.

- `remote_screen_docker`의 무시된 테스트 4개는 돌리지 않았어요. 서버 브라우저 이미지가 필요한 테스트라 기본 실행에서 빠져요.
- musl 컨테이너의 테스트(`dev/musl-test.sh`)는 돌리지 않았어요. 이미지 빌드가 musl로 두 바이너리를 컴파일한 것까지만 확인했어요.
- 개발 환경에서 자막 작업과 보관 파일 정리는 실행하지 않았어요. 이 티켓의 확인 대상이 아니어서예요.
