# 0108 작업 실행기에서 원격 화면 돌보기와 받기·게시·복구를 떼요

- 상태: 완료
- 출처: [자막 작업 줄기의 재구성](refactoring.md#자막-작업-줄기의-재구성), [체크포인트와 중단 복구](../specs/jobs.md#체크포인트와-중단-복구)
- 막는 티켓: [0092](0092-refactor-baseline.md)(기준값)

## 작업

trss-jobs의 `runner.rs`(2,607줄)는 2026-10-07에 작업 루프 옆에 원격 화면 돌보기 약 400줄과 받기·게시·복구 약 800줄을 함께 가졌어요.
크레이트 분리 뒤 `runner.rs`는 `store.rs`와 18번, 마이그레이션 목록과 18번 함께 바뀌었어요.

- 원격 화면 돌보기와 받기·게시·복구를 작업 루프에서 떼어, 각자의 인터페이스를 가진 모듈로 둬요. 작업 루프는 차례만 정해요.
- 중단 뒤 복구의 순서와 체크포인트는 그대로예요. 떼는 동안 복구 테스트를 고치지 않아요.
- 실행 task를 띄우고 종료 때 유예를 주는 일이 worker의 주기, 자막 작업, 명령, 수정본 재확인에 4–5벌 있어요. 이 티켓에서 작업 쪽을 정리할 때 하나로 모을 수 있으면 모아요.

## 완료 기준

- 원격 화면 돌보기와 받기·게시·복구가 작업 루프 밖의 모듈에 있고, 작업 루프는 그 인터페이스만 불러요.
- 기존 테스트가 고치지 않고 통과해요.

## 결과

받기·게시·복구와 원격 화면 돌보기를 각자 상태를 가진 타입으로 뗐어요. `Runner`는 차례를 정하고 두 타입의 인터페이스를 불러요.
`Runner`의 공개 함수는 이름과 모양이 그대로라서, trss-worker, trss-web, 테스트는 한 줄도 바뀌지 않았어요.

### 나눈 것

| 타입 | 모듈 | 가진 상태 | 작업 루프가 부르는 인터페이스 |
| --- | --- | --- | --- |
| `Receiver` | `runner/receive.rs`(1,173줄) | `JobRun`, `Placer`, 받는 곳(`ReceiveArea`), 시계, 재시도 간격 | `receive`(파일 하나를 받아 `Receipt`를 돌려줘요), `recover`(끝나지 않은 받은 기록을 디스크와 견줘요), `retry_waits`(게시물 열기의 재시도도 같은 간격을 써요) |
| `ScreenTender` | `runner/tender.rs`(554줄), `runner/tender/find.rs`(460줄), `runner/tender/pages.rs`(289줄) | `JobRun`, `JobViews`, 받는 곳, 시계, 화면 기록(`ScreenStore`), 서버 브라우저(`AuthBrowser`), 지켜보는 run, 확인을 다시 띄우는 작업, 찾기 작업의 가져가기 잠금 | `through_check`(사이트 확인으로 데려가요), `check_staging`(확인으로 받은 파일의 폴더), `release`(끝난 작업의 브라우저 run과 화면을 정리해요), `tend`(`Runner::tend_screens`가 넘겨요), `run_find`(찾기 작업의 run 하나) |

- 나머지 함수는 각 모듈 안에서만 보여요. 받기의 `attempt`, `publish`, `reuse`, 실패와 보류 기록, 원격 화면의 `wait_check`, `watch`, 페이지 따라가기, 찾기 작업의 지켜보기와 가져가기가 그래요.
- 이름은 이미 있는 `Placer`를 따랐어요. 모듈을 `screens`가 아니라 `tender`로 한 것은 `crate::screen` 옆에서 헷갈리지 않게 하려는 것이에요.
- 모듈 문서도 나눴어요. "한 파일의 받기" 단계와 "다시 시작" 표는 `receive.rs`로, 사이트 확인 부분은 `tender.rs`로 옮겼어요. `runner.rs`에는 작업의 차례가 남았어요.

### 작업 루프에 남은 것

`runner.rs`는 2,595줄에서 1,070줄이 됐어요.
남은 것은 작업을 집고 차례대로 돌리는 일(`run_ready`, `run_job`, `run_item`), 작업의 상태를 정하는 일(`received`, `settle`), 다시 줄 세우기, 정리 작업, WinPNG 읽기(`read_winpng`)예요. WinPNG 읽기는 게시물을 여는 단계라서 루프에 남겼어요.

### 찾기 작업

찾기 작업은 원격 화면이 이끄는 작업이라, `run_find`까지 모두 `ScreenTender`의 메서드로 `tender/find.rs`에 뒀어요. 그래서 지켜보는 run과 가져가기 잠금은 `ScreenTender` 하나만 가져요.
`run_find`를 루프에 두고 `ScreenTender`의 인터페이스를 부르게 하는 안도 있었어요. 그러면 잠금을 잡고 가져가기, 끝내기 요청으로 끝내기, run 묶기 같은 인터페이스가 다섯 개쯤 더 필요해서 버렸어요.
`tests/it/find.rs`가 `runner::find::FIND_NOTE`를 쓰므로, `runner.rs`가 `pub use tender::find`로 같은 경로를 남겨요.

### 복제본이 함께 쓰는 상태

worker는 `Runner`를 복제해 task로 띄우고, `ScreenTender::tend`도 자신을 복제해 지켜보기 task를 띄워요.
지켜보는 run, 확인을 다시 띄우는 작업, 가져가기 잠금은 `Arc<Mutex<..>>`로 `ScreenTender`에만 있어서, 복제본이 모두 같은 것을 써요. 영상이 왔는지 본 세대(`video_seen`)는 `Runner`에 `Arc`로 남았어요.
재시도 간격은 바뀌지 않는 `Arc<[Duration]>`로 `Receiver`에만 있고, 루프는 `retry_waits()`로 읽어요.

### 실행 task와 종료 유예

worker가 task를 띄우고 종료 요청 뒤 유예를 주는 곳 가운데, 동작이 같은 두 곳을 `Worker::join_within_grace` 하나로 모았어요. 수집 주기와 자막 작업 실행이에요. 두 곳은 각자의 메시지와 그 뒤의 흐름(수집 주기는 `continue`, 작업 실행은 `break`)을 그대로 가져요.
나머지는 그대로 뒀어요.

- 수정본 재확인은 유예 없이 끝날 때까지 기다려요. 유예를 주면 동작이 바뀌어요.
- 명령은 여러 task를 `JoinSet`으로 모아 한꺼번에 기다리는 다른 모양이에요.
- 서버 브라우저 풀의 종료는 띄운 task가 아니라 future 하나에 시간 제한을 둬요.

### 동작 보존

- 옮긴 코드는 git의 이동 감지로 옮긴 줄과 새로 쓴 줄을 나눠 보았어요. 받기 커밋은 2,126줄이 옮겨졌고, 원격 화면 커밋은 877줄이 옮겨졌어요. `find.rs`와 `pages.rs`는 파일째 옮겨졌어요.
- 새로 쓴 줄은 `use`, 가시성, 타입 정의와 생성자, 문서 링크, 그리고 `self.x`를 `self.receiver.x`나 `self.tender.x`로 바꾼 부르는 줄이에요.
- 함수 몸통이 바뀐 곳은 `settle`의 정리 하나예요. 서버 브라우저 run을 놓고 화면을 지우는 두 줄이 `ScreenTender::release`로 옮겨졌고, WinPNG 읽기의 run을 놓는 줄은 그 앞에 남아 순서가 같아요.
- 테스트 파일은 하나도 바뀌지 않았어요. `pages.rs`의 단위 테스트는 파일과 함께 옮겨졌고 테스트 코드는 그대로예요.

### 검증한 것

2026-10-09에 확인했어요.

| 커밋 | cargo 통과 | 실패 | 무시 |
| --- | ---: | ---: | ---: |
| 시작(`refactor(jobs): split JobStore into JobViews, JobRequests and JobRun over the same Db`) | 2,898 | 0 | 14 |
| `refactor(jobs): move receiving, publishing and recovering a file into Receiver` | 2,898 | 0 | 14 |
| `refactor(jobs): move the remote-screen care and the find job into ScreenTender` | 2,898 | 0 | 14 |
| `refactor(worker): wait for a spawned task within the shutdown grace in one helper`(이 결과를 담은 커밋) | 2,898 | 0 | 14 |

- cargo는 `cargo test --locked --workspace -j 4`예요. 가운데 두 행은 에이전트가 커밋마다 돌린 값이고, 마지막 행은 제가 다시 돌려 확인했어요.
- `cargo fmt --all --check`는 깨끗하고, clippy에 경고가 없어요.
- 에이전트가 센 `cargo doc -p trss-jobs --no-deps`의 경고는 31개에서 30개가 됐어요. 비공개 항목 `Runner::recover`로 가던 링크가 "다시 시작" 표와 함께 옮겨지며 없어졌고, 새로 생긴 경고는 없어요.
- 실제 서버 브라우저나 worker를 띄워 보지는 않았어요. 테스트로만 확인했어요.

### 남은 것

- `receive.rs`는 1,173줄이에요. 옮기기만 했고, 더 나눈다면 받기(`attempt`, `publish`)와 복구(`recover`) 사이가 자연스러운 선이에요.
- `Receiver`는 `Runner::new`가 만든 `Placer`의 복제본을 가져요. 그래서 그 뒤 `with_unpacker`로 붙인 압축 풀기 프로그램은 `Receiver`의 복제본에 없어요. 지금 `Receiver`가 부르는 `unchanged_font`와 `standing`은 그것을 쓰지 않아 동작은 같아요. 다만 나중에 `Receiver`에서 압축을 푸는 함수를 부르면 프로그램이 없는 것으로 보여요.
