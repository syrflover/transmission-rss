# 0160 테스트 바이너리가 테스트마다 들이는 준비 비용을 측정하고 줄여요

- 상태: 완료
- 출처: [구조 가볍게 하기 명세의 테스트 정리](lighter-structure.md#테스트-정리)
- 막는 티켓: [0150](0150-trss-crate-tests-by-adr-0015.md)(`trss` 크레이트의 테스트), [0153](0153-no-effect-records.md)(효과별 기록), [0159](0159-generated-types-rest-and-events.md)(타입 생성의 마지막), [0163](0163-batch-archive-on-server.md)(서버의 일괄 보관)

## 작업

2026-10-21에 실행에서 가장 큰 테스트 바이너리는 `trss-web` 단위 테스트(7.0초, 468개)와 `trss-worker`의 `it`(6.6초, 91개)였고, 빌드에서 가장 큰 테스트 단위는 `trss-jobs`의 `it`(37.8초), `trss-web` 단위 테스트(33.8초), `trss-collect` 단위 테스트(30.5초)였어요. 합친 뒤에는 `trss-web`과 `trss-worker`의 것이 `trss` 크레이트의 바이너리예요.

이 티켓은 그런 바이너리가 테스트 하나를 준비하는 데 드는 비용을 측정해요. DB, 앱 상태, HTTP 서버, 임시 폴더, 가짜 Transmission을 테스트마다 만드는 비용이 그 예예요. 찾은 원인마다 고치거나, 고치지 않은 까닭을 적어요. 2026-10-21에 테스트가 없는 bin 하네스도 하나에 0.1–0.4초씩 들었어요.

테스트의 모양이 0150–0159로 정해진 뒤에 해야 같은 것을 두 번 고치지 않아요. 테스트 양을 줄이는 일은 이 티켓의 범위가 아니에요(사용자 결정, 2026-10-21).

## 완료 기준

- [x] 이 티켓을 시작하는 커밋에서 [0144](0144-lighter-structure-baseline.md)와 같은 명령으로 측정해, 실행 시간 상위 다섯 테스트 바이너리와 빌드 시간 상위 세 테스트 단위를 대상으로 정해요.
- [x] 대상마다 테스트 하나를 준비하는 비용을 측정하고, 측정한 방법과 값을 결과에 적어요.
- [x] 찾은 원인마다 고치거나, 고치지 않은 까닭을 결과에 적어요. 고친 것은 고치기 전과 뒤의 값을 같은 명령과 빌드 상태로 측정해 함께 적어요.
- [x] 통과·건너뛴 테스트 수가 앞뒤로 같아요.
- [x] 메모리 최고치를 측정하는 테스트 3개는 기본 실행에 그대로 있어요.
- [x] 측정 스크립트는 `dev/measure/`에, 원본 로그는 `dev/local/measure/0160/`에 있어요.
- [x] [명세의 동작 보존 조건](lighter-structure.md#동작-보존)을 지켜요.

## 결과

2026-10-22에 `a19eb94b`에서 측정해 대상을 정하고, 찾은 원인 다섯 가지를 고쳤어요. 워크스페이스 테스트 바이너리의 `finished in` 합은 43.6·43.7초에서 36.6·36.7초가 됐어요. `trss-core`를 고친 뒤의 cold 빌드는 82.6초에서 51.9초가 됐어요. 테스트 수는 그대로예요. 측정과 수정은 `stora:complex-task` 에이전트가 했어요. 원본 로그와 `--timings` 결과는 이 PC의 `dev/local/measure/0160/`에 있어요.

### 시작 측정과 대상

[0144](0144-lighter-structure-baseline.md)와 같은 PC와 명령으로 측정했어요. `cargo test --locked --workspace --no-run -j 4`로 빌드한 뒤 `cargo test --locked --workspace -j 4`를 바꾸지 않고 두 번 돌렸어요. 새 `dev/measure/test-runs.sh`가 이 순서를 돌리고, 돌리기 전마다 다른 cargo·rustc·테스트 바이너리가 없는지 확인해요. rust-analyzer의 flycheck가 끼어든 측정은 버리고 다시 했어요.

| 값 | 1회 | 2회 |
| --- | ---: | ---: |
| 테스트 바이너리 27개의 `finished in` 합 | 43.6초 | 43.7초 |
| 통과·실패·무시 | 3,201·0·12 | 3,201·0·12 |

실행 시간 상위 다섯 바이너리를 대상으로 정했어요.

| 바이너리 | 1회 | 2회 |
| --- | ---: | ---: |
| `trss` `tests/it` | 7.32초 | 7.33초 |
| `trss` lib | 7.10초 | 7.10초 |
| `trss-jobs` `tests/it` | 3.42초 | 3.42초 |
| `trss-browser` `tests/it` | 3.15초 | 3.15초 |
| `trss` `tests/subtitle_upload_peak.rs` | 2.93초 | 2.93초 |

빌드 시간은 `dev/measure/core-edit-build.sh`로 측정하고 단위별 시간으로 순위를 매겼어요. 이 티켓의 작업에 적은 37.8초, 33.8초, 30.5초는 0144가 견주지 않기로 한 예전 94초 빌드의 값이어서, `trss-core`를 고친 뒤의 cold와 warm 빌드로 매겼어요. 둘 다 같은 세 단위가 위였어요.

| 테스트 단위 | cold | warm |
| --- | ---: | ---: |
| `trss` lib 테스트 | 65.4초 | 11.1초 |
| `trss-jobs` `it` | 36.6초 | 4.3초 |
| `trss-collect` lib 테스트 | 31.2초 | 4.2초 |

이때 빌드 전체는 cold 82.6초, warm 16.9초였어요.

### 측정한 방법

- `dev/measure/test-each.py`는 바이너리 하나를 cargo를 거쳐(cargo가 주는 환경 변수 그대로) 스레드 하나로 한 번, libtest의 기본 스레드로 한 번 돌려요. 테스트마다 걸린 시간과 CPU 시간, 마지막에 끝난 테스트가 언제 시작했는지를 적어요.
- 준비 함수마다 걸리는 시간은 반복해 재는 임시 테스트로 측정하고, 측정한 뒤 지웠어요.
- CPU를 어디에 쓰는지는 `perf record -e cpu-clock:u`로 봤어요.
- 빌드 단위는 별도 target 폴더에서 `-Ztime-passes`와 `-Zdump-mono-stats`로 봤고, 그 폴더는 지웠어요.

### 대상마다 테스트 하나의 준비 비용

| 대상 | 측정한 준비 비용 | 시간을 정하는 것 |
| --- | --- | --- |
| `trss` lib | `AppState::new` 16 ms(HTTP 클라이언트 셋이 하나에 5.2 ms), 라우터 2.5 ms, 메모리 DB 0.27 ms. 테스트 중앙값 24.7 ms | 준비 비용. 바이너리의 끝 시각은 2.9초에 시작하는 4.2초짜리 업로드 멈춤 테스트가 정해요. |
| `trss` `tests/it` | `Harness::new` 2.0 ms, worker가 있으면 3.3 ms, `trss` 프로세스를 띄워 듣기까지 23 ms | 기다림. 스레드 하나로 40초를 돌 때 CPU는 6초였어요. 끝 시각은 2.0초에 시작하는 5.1초짜리 SIGTERM 유예 테스트가 정해요. |
| `trss-jobs` `tests/it` | 라이브러리와 실행기를 갖춘 `Base` 2.2 ms. 테스트 중앙값 16 ms | 테스트 자신의 일. 디버그 빌드의 zlib-rs로 zip fixture를 만드는 데 1.57초짜리 `unpack::bombs…` 테스트의 1,031 ms가 들었어요. |
| `trss-browser` `tests/it` | 테스트 70개의 CPU가 모두 0.6초, 하나에 8.6 ms 아래예요. 준비 함수를 하나씩 재지는 않았어요. | 0초에 시작하는 테스트 하나의 2.8초 대기 |
| `subtitle_upload_peak` | 테스트 하나이고 준비는 가벼워요. | CPU의 약 85%가 128 MiB 본문의 디버그 빌드 SHA-256이에요. |

함께 돌 때 느려지는 정도도 봤어요. `trss-jobs` `tests/it`의 테스트는 따로 돌 때보다 1.65배 오래 걸렸고, CPU 시간도 10.3초에서 15.3초로 늘었어요. 공유 잠금이 아니라 기계(SMT)가 꽉 찬 것으로 봐요. 나머지 대상은 1.0–1.2배였어요.

빌드 단위에서는 코드 생성이 컴파일 시간의 약 60%였고, link는 1초 아래였어요. 생성한 코드에서 가장 큰 몫은 `Db::run`에 넘기는 closure마다 따로 만들어지는 tokio의 blocking task였어요. `trss-jobs` `it`의 45%, `trss-collect` lib 테스트의 31%, `trss` lib 테스트의 23%였어요. 워크스페이스 크레이트의 전체 디버그 정보가 단위마다 17–21%를 더했고, `trss` lib 테스트에서는 ts-rs가 약 26%였어요.

### 고친 원인

같은 명령과 빌드 상태로 고치기 전과 뒤를 측정했어요.

| 원인 | 고친 것 | 고치기 전 | 고친 뒤 |
| --- | --- | --- | --- |
| cargo가 자기가 띄우는 프로세스에 `SSL_CERT_DIR=/etc/ssl/certs`를 넘겨서, HTTP 클라이언트마다 인증서 묶음 파일과 그 폴더의 파일을 모두 읽었어요 | `.cargo/config.toml`에서 `SSL_CERT_DIR`을 비워요 | 클라이언트 하나 5.25 ms, `trss` lib 7.10초, 워크스페이스 합 43.6초 | 0.80 ms, 6.07초, 41.6초 |
| 디버그 빌드의 zlib-rs와 crc32fast가 archive 테스트의 zip을 느리게 만들었어요 | 두 크레이트를 `opt-level = 3`으로 빌드해요 | zip fixture 1,031 ms, `unpack::bombs…` 1.57초, `revision_crc_peak` 1.9초 | 36 ms, 0.29초, 0.32초 |
| 디버그 빌드의 SHA-256 | `sha2`를 `opt-level = 3`으로 빌드해요 | `subtitle_upload_peak` 2.92초, 워크스페이스 합 40.1·40.0초 | 0.55초, 36.6·36.4초 |
| `Db::run`의 closure마다 blocking task가 새로 컴파일됐어요 | closure를 상자에 담아 task를 한 번만 컴파일해요 | cold 빌드 82.6초(단위 합 310.8초) | 64.2초(239.7초) |
| 문서 테스트가 0개인데도 rustdoc이 크레이트마다 약 0.25초를 썼어요 | 라이브러리 크레이트 12개에 `doctest = false` | 실행마다 2.95초 | 0초 |
| 워크스페이스 크레이트의 전체 디버그 정보 | `dev` 프로필에서 줄 정보만 남겨요(사용자 결정, 2026-10-22) | cold 65.6초, warm 16.1초, 다섯 번 빌드의 디스크 쓰기 19.0 GB | 51.9초, 14.4초, 10.3 GB |

- 인증서: 이 PC에서 묶음 파일과 폴더는 같은 루트 119개를 담아요. `cargo run`으로 띄운 앱도 묶음 파일만 읽어요. 이미지로 빌드한 앱은 cargo 없이 돌아서 영향이 없어요.
- `sha2`: 이 최적화를 켜자 `trss-library`의 `artwork::tests::a_request_that_stops_waiting_for_room_or_for_its_read_leaves_the_room_alone`이 8번 중 4번 실패했어요. 시간 0의 timeout이 첫 poll에서 파일 읽기가 아직 진행 중이라고 가정했기 때문이에요. 테스트 전용 장치(`cfg(test)`의 `ReadHold`)로 읽기를 붙잡아 두고 timeout을 걸게 고쳤어요. 단언은 그대로예요. 고친 뒤 그 바이너리를 50번, 그 테스트만 50번 돌려 실패가 없었어요.
- `Db::run`: 시그니처, 패닉 처리, 바뀜 알림, `DbError::Task`로 바꾸는 것은 그대로예요. 호출마다 상자 둘과 downcast 하나가 더해져요. 단위별로 cold 빌드는 `trss` lib 테스트 65.4초에서 48.2초, `trss-jobs` `it` 36.6초에서 21.6초, `trss-collect` lib 테스트 31.2초에서 23.4초가 됐어요.
- 디버그 정보: kache가 분리된 디버그 파일의 경로를 바꿔서, gdb는 전에도 이 빌드의 변수를 읽지 못했어요. 패닉 백트레이스는 줄 정보만 있으면 돼요. `cargo test --profile debugger`로 빌드한 `trss-core` 테스트에는 `migrate_until`의 매개변수와 지역 변수가 있고, 같은 함수가 `dev` 빌드에는 없는 것을 `llvm-dwarfdump`로 확인했어요. 단위별 cold 빌드는 `trss` lib 테스트 49.6초에서 32.2초, `trss-collect` lib 테스트 23.7초에서 19.3초, `trss-jobs` `it` 21.7초에서 17.7초가 됐어요.

### 고치지 않은 원인

- 테스트가 하는 일인 긴 기다림: SIGTERM 유예 5초, 업로드 멈춤 4.2초, 서버 브라우저 launcher의 2.8초 대기, 실행기 종료 1.6초예요. 그 테스트가 확인하는 시간이라서 줄이면 테스트의 뜻이 바뀌어요.
- `trss` lib 테스트 빌드의 ts-rs(생성한 코드의 약 26%): `TS` derive를 기능 뒤로 옮기면, 커밋한 화면 타입이 Rust 타입과 같은지 보는 `committed_files_are_current`가 기본 `cargo test`에서 돌지 않아요. 화면 타입이 낡는 것을 기본 실행이 잡는 쪽을 골랐어요.
- `trss-jobs` `tests/it`이 함께 돌 때 1.65배 느려지는 것: 기계가 꽉 찬 것이어서 테스트 쪽에서 고칠 원인이 아니에요.

### 마친 뒤의 측정

시작 측정과 같은 명령과 빌드 상태예요.

| 값 | 시작(`a19eb94b`) | 마친 뒤 |
| --- | ---: | ---: |
| 테스트 바이너리 27개의 `finished in` 합 | 43.6·43.7초 | 36.6·36.7초 |
| cargo의 전체 실행 시간 | 47.2·47.0초 | 36.9·37.0초 |
| `trss` `tests/it` | 7.32·7.33초 | 7.02·7.03초 |
| `trss` lib | 7.10·7.10초 | 6.00·5.98초 |
| `trss-jobs` `tests/it` | 3.42·3.42초 | 2.92·2.93초 |
| `trss-browser` `tests/it` | 3.15·3.15초 | 3.14·3.13초 |
| `subtitle_upload_peak` | 2.93·2.93초 | 0.54·0.55초 |
| `trss-core`를 고친 뒤의 빌드, warm | 16.9초 | 14.4초 |
| `trss-core`를 고친 뒤의 빌드, cold | 82.6초 | 51.9초 |
| 통과·실패·무시 | 3,201·0·12 | 3,201·0·12 |

- 시작의 cargo 전체 실행 시간은 시작 측정이 적지 않아서, 고친 것을 잠시 치운 같은 커밋에서 다시 측정한 값이에요.
- 빌드 시간은 cold와 warm을 한 번씩 측정했어요. warm은 `Db::run`을 고친 뒤 15.5초, 디버그 정보를 바꾸기 전 16.1초였어요.

### 확인

- 메모리 최고치를 측정하는 테스트 셋(`subtitle_upload_peak`, `artwork_serving_peak`, `revision_crc_peak`)과 `artwork_upload_peak`는 기본 실행에 그대로 있고, 파일도 바꾸지 않았어요.
- 2026-10-22에 플래그 없는 `cargo test --locked --workspace`도 3,201·0·12였어요. `cargo clippy --locked --workspace --all-targets --all-features`에 경고가 없고, `cargo fmt --all --check`가 통과했어요. 웹 파일은 바꾸지 않았어요.
- 측정 스크립트 `dev/measure/test-runs.sh`와 `dev/measure/test-each.py`를 더했어요.

### 동작 보존

앱의 동작은 그대로예요. 바뀐 앱 코드는 `Db::run`의 내부와 `trss-library`의 테스트 전용 장치뿐이에요. 프로필과 `doctest`, `.cargo/config.toml`의 환경 변수는 개발 빌드와 cargo가 띄우는 프로세스에만 닿아요. 테스트의 단언은 바꾸지 않았고, 지운 테스트와 무시하게 바꾼 테스트도 없어요.

### 확인하지 않은 것

- musl 대상과 빈 `target/`에서 처음부터 하는 빌드: 0144처럼 견주는 값이 아니어서 측정하지 않았어요.
- `trss-browser` `tests/it`의 준비 함수 하나하나: CPU의 상한만 봤어요.
- `Db::run`의 상자가 실행 시간에 주는 영향만 따로: 워크스페이스 측정에 묻혀 따로 재지 않았어요.
- cold 빌드의 실행마다의 차이: 한 번씩만 측정했어요.
- `doctest = false`라서 나중에 문서에 Rust 예제를 더하면 실행되지 않아요. 예제를 더할 때 그 줄을 지워야 해요.
