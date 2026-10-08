# 0099 토렌트 하나를 더하고, 기록하고, 이름을 바꾸는 경로를 하나로 만들어요

- 상태: 완료
- 출처: [수집 받기 줄기의 재구성](refactoring.md#수집-받기-줄기의-재구성), [ADR 0011](../adr/0011-feature-crate-workspace.md)
- 막는 티켓: [0098](0098-release-name-reading.md)(이름 읽기)

## 작업

2026-10-07에 worker의 수집 주기(`cycle.rs`의 `process_job`)와 trss-collect의 한 번 받기 명령(`receive_once.rs`의 `execute_with`)이 토렌트를 더하고, 기록하고, 이름을 바꾸는 일을 각자 했어요.

- 이름 바꾸기와 재시도가 trss-transmission의 `rename_torrent`·`rename_with_retries`와 한 번 받기의 `rename`에 따로 있어요. 이름을 정하지 못하면 앞의 것은 새로 더한 토렌트를 지우고, 뒤의 것은 지우지 않아요. 뒤의 것만 원래 이름을 DB에 남겨요.
- 같은 실패를 수집 주기는 영어로, 한 번 받기는 한국어로 적어요.
- 수집 판단(`process_job`, `withhold`, `start_replacement`, `make_plans`)이 worker에 있어요. ADR 0011은 worker에 실행 루프, 명령 분배, 주기만 남기기로 했어요.
- 이 줄기는 수정이 가장 많았던 곳이에요. 이력 전체에서 `cycle.rs`를 바꾼 커밋 49개 중 22개, `receive_once.rs`를 바꾼 커밋 52개 중 29개가 수정 커밋이에요.

한 경로로 모으되 지금 동작은 그대로 둬요.

- 항목 하나를 더하고, 기록하고, 이름을 바꾸는 일은 trss-collect의 한 경로가 맡아요. 수집 주기와 한 번 받기 명령은 그 경로를 부르고, 두 쪽이 지금 다르게 하는 일(새로 더한 토렌트를 지우는지, 원래 이름을 남기는지)은 경로에 넘기는 값으로 지켜요.
  수집 주기가 이름을 정하지 못한 토렌트를 지우는 동작은 [0128](0128-keep-unnamed-cycle-torrent.md)에서 없애요(사용자 결정, 2026-10-08). 0099는 지금 동작을 지켜요.
- 명령 표의 한 번 받기 전용 칸(`add_unconfirmed`, `original_name`)은 지금 뜻을 지키고, 수집 주기는 지금처럼 그 칸을 쓰지 않아요.
- trss-transmission에는 RPC 호출과, 호출하는 쪽이 넘긴 이름 계산만 남아요.
- 수집 판단은 trss-collect로 옮기고, worker에는 실행 루프, 명령 분배, 주기만 남아요.
- 같은 실패를 다르게 적는 문구는 [문구와 표기](../specs/web-app.md#문구와-표기)를 따르지 않는 쪽을 고치는 결함 수정으로 따로 커밋해요.
- 저장 데이터와 동시 실행(폴더 차례, 토렌트 문, 잠금)을 건드리므로 독립 리뷰를 받아요. 리뷰는 명령 표의 칸, 이름 바꾸기와 그 재시도의 순서, 중단 뒤 이어 하기를 봐요.

## 완료 기준

- 수집 주기와 한 번 받기 명령이 같은 경로를 불러요. 토렌트 더하기, 기록, 이름 바꾸기와 그 재시도의 구현이 각각 하나예요.
- trss-worker의 `cycle.rs`에 수집 판단 함수가 없어요. worker에 남은 일의 목록이 결과 절에 있어요.
- worker와 trss-collect의 기존 테스트가 고치지 않고 통과해요.
- 독립 리뷰의 지적과 처리 결과가 결과 절에 있어요.

## 결과

### 결정

- **경로의 모양**: trss-collect의 `receive` 모듈이 `add`, `record`, `rename`을 하나씩 가져요(리드가 정함).
  - 부르는 쪽이 폴더 차례를 먼저, 그다음 토렌트 문을 잡고 세 단계를 불러요. 경로는 차례도 문도 잡지 않아요.
  - 두 쪽이 다르게 하는 일은 넘기는 값으로 지켜요. `Original`은 어느 이름에서 새 이름을 만드는지(수집 주기는 처음 읽은 이름 `Current`, `다시 받기`는 명령 표에 남긴 이름 `Recorded`), `Underivable`은 trname이 이름을 만들지 못한 새 토렌트를 지우는지(`Remove`) 남기는지(`Keep`), `note`는 이름이 그대로인 까닭을 어느 기록 항목에 적는지예요.
  - 답이 없는 추가를 확인하지 못한 추가로 보는 기준(수집 주기는 거절이 아닌 실패 모두, `다시 받기`는 답이 없는 경우)과 `다시 받기`의 다섯 번 시도는 부르는 쪽의 정책으로 남겨요.
- 명령 표의 `add_unconfirmed`와 `original_name`은 `다시 받기`만 써요. 수집 주기는 지금처럼 쓰지 않아요.
- `receive_past`의 `new_revision`은 수집 주기와 같게 동작해서 `Decided::row` 하나로 합쳤어요. `withhold`와 `decide_revision`은 다르게 동작해서 따로 둬요.
  - 수집 주기는 항목 키로 `Observe`를 주기의 시각에 쓰고, 오류가 나면 항목이 다음 주기를 기다려요.
  - `receive_past`는 항목 ID로 `Outcome`을 지금 시각에 쓰고, 오류가 나면 명령을 다시 시도해요.
  - `receive_past`는 사람이 고른 `버전 미상` 결과를 받아들이고, 수집 주기는 그 결과를 보류해요.
- **패닉 주입 hook**: 수집 주기 항목 작업이 패닉하면 worker가 토렌트를 지우지 않는지 보는 worker 테스트는, 파일 개수가 없는 답으로 패닉을 일으켰어요. 아래 (c)로 그 패닉이 사라져서, `test-support` 기능 뒤의 `trss_collect::cycle::fault::panic_after_record`로 기록 직후 패닉을 일으켜요. 기능을 켜지 않은 빌드에는 들어가지 않아요(리드가 정함).
- **동작 변경 (b)–(d)**: 작업 절은 "지금 동작은 그대로 둬요"이고, 동작을 바꾸는 수정으로는 (a)의 문구만 적었어요. 두 경로를 견주다 수집 주기에서만 나는 결함 셋이 드러나서, 정리 커밋과 따로 커밋해 고쳤어요(리드가 정함, 2026-10-08). 셋 다 저장 형식은 그대로예요.
  - (b)는 RPC 수와 차례를 쥔 시간만 줄이고, 저장되는 값과 Transmission의 마지막 상태는 같아요.
  - (c)는 Transmission의 답에서 칸 하나가 빠지면 항목 작업이 패닉하던 것을 다시 묻기로 바꿨어요.
  - (d)는 회차가 두 번 변환된 잘못된 파일 이름을 막아요.
  - 그래서 완료 기준의 "기존 테스트를 고치지 않고 통과"는 정리 커밋에서 지켰고, 동작 변경 커밋에서 기존 테스트 두 곳을 고쳤어요(아래 바꾼 것).
- **패닉 문구**: `토렌트를 추가하다 내부 오류가 났어요.`예요. 다른 내부 오류 문구(`처리하다 내부 오류가 났어요.`, `폴더를 옮기다 내부 오류가 났어요.`)와 꼴을 맞췄어요.

### 바꾼 것

| 커밋 | 내용 |
| --- | --- |
| `000b078` test(worker): pin what the cycle leaves of its own adds whose name stays | 지금 테스트가 지키지 않던 동작, 곧 수집 주기가 이름을 만들지 못한 새 토렌트를 지우는 것 등 4개를 정리 전에 테스트로 지켰어요. |
| `b701931` refactor(collect): move the cycle's item decisions out of the worker | 수집 판단을 `trss_collect::cycle`로 옮겼어요. |
| `ffb6ded` fix(collect): write the cycle's add failures in Korean like 다시 받기 | (a) |
| `9086756` refactor(collect): one path adds, records and renames a received torrent | `receive`의 한 경로예요. 이름 바꾸기와 재시도를 trss-transmission에서 옮겼고, trss-transmission은 RPC 호출만 남아 trss-core·trname·tokio-util 의존이 빠졌어요. 기록 항목 ID를 돌려주는 `HistoryStore::record_one`을 더했어요. |
| `e984ee8` refactor(collect): one constructor for the row a revision decision writes | `Decided::row` |
| `59370f4` fix(collect): ask again for a torrent without a file count in the cycle too | (c) |
| `78936a3` fix(collect): stop the cycle's rename once nothing more can come of it | (b) |
| `d63e767` fix(collect): derive the cycle's rename from the first name it reads | (d) |
| `perf(collect): let the cycle's rename go on in the add's Transmission session`(이 결과를 담은 커밋) | 아래 리뷰 지적 3이에요. |

정리 커밋 셋(`b701931`, `9086756`, `e984ee8`)은 기존 테스트를 고치지 않고 통과했어요. 기존 테스트를 고친 곳은 동작 변경 커밋의 두 곳뿐이에요. (a)는 영어 거절 문구를 확인하던 worker 테스트의 기대 문구 하나를 바꿨고, (c)는 패닉 테스트가 패닉을 일으키는 방법만 바꿨어요.

### worker에 남은 것

worker의 `cycle.rs`는 1,418줄에서 582줄이 됐어요.

- `run_cycle`: 주기의 순서(채널 읽기, 판단, 추가, 떠난 토렌트 지우기)와 `CycleReport` 집계
- `add_jobs`: 항목 작업을 띄우고 기다리고, 패닉을 가둬요
- `apply_session`, `record_reads`, `record_transmission_counts`, 동시 실행 상수, `CycleError`, `CycleReport`
- 지우기의 연결: `Removal`, `TorrentGate`, `InFlight`(`removal.rs`)와 `CommandsAtRemoval` 다시 내보내기
- 실행 루프, `tick`, 명령 분배

`trss_collect::cycle`로 옮긴 것은 `make_plans`, `open_rules`, `judge_feed`(항목 판단), `leave_revisions`(이제 `CycleReport` 대신 개수를 돌려줘요), `settle_offsets`, `process_job`, `record_panic`, `remove_departed`, `new_revision`, `withhold`, `start_replacement`, `rename_mode`예요. `RenameMode`, `has_trname_form`, `looks_renamed`와 이름 바꾸기는 trss-transmission에서 `trss_collect::receive`로 옮겼어요.

### 동작이 바뀐 것

- **(a) 추가 실패 문구**(`ffb6ded`): 수집 주기도 `다시 받기`처럼 연결 실패, 답 없음, 거절을 한국어 한 문장으로 적어요. 항목 작업이 패닉하면 위의 패닉 문구를 적어요.
- **(b) 이름 바꾸기 멈춤**(`78936a3`): 수집 주기의 이름 바꾸기가 `다시 받기`처럼 토렌트가 없거나, 파일이 여러 개거나, 방금 지웠거나, 이름이 이미 맞으면 곧바로 멈춰요. 전에는 남은 시도를 다 쓰고, Transmission이 거절하는 같은 이름 바꾸기도 보냈어요. 저장되는 값과 Transmission의 마지막 상태는 같고, RPC 수와 폴더 차례를 쥔 시간만 줄어요.
- **(c) 파일 개수가 없는 답**(`59370f4`): 두 경로 모두 메타데이터가 아직 없는 것으로 읽고 다시 물어요. 전에는 수집 주기가 `unwrap`에서 패닉했어요.
- **(d) 두 번 바뀌는 회차**(`d63e767`): Transmission이 이름을 바꿨지만 그 답을 잃으면, 수집 주기의 다음 시도가 바뀐 이름에서 다시 회차를 변환했어요. 가짜 Transmission으로 재현했어요. E62가 E38을 거쳐 E14가 됐어요. 이제 수집 주기는 처음 읽은 이름을 기억하고 그 이름에서만 새 이름을 만들어요.

(b)·(c)·(d)의 테스트는 trss-collect의 `receive/tests.rs`에 있고, 각각 고치기 전에 실패하는 것을 확인했어요.

### 독립 리뷰

2026-10-08에 stora:complex-reviewer가 `d63e767`까지의 커밋을 리뷰했어요(패닉 문구를 고치기 전이에요).
리뷰는 명령 표의 칸, 이름 바꾸기와 그 재시도의 순서, 차례와 문의 순서를 끝까지 확인했어요. 중단 뒤 이어 하기는 worker를 단계 사이에서 죽이는 테스트가 없어서 코드를 읽고 추론해서만 확인했어요.
리뷰어는 trss-collect의 `receive` 테스트 44개와 trss-worker 테스트 434개를 돌렸어요. `cargo tree -e normal,build,features -i trss-collect`로 일반 빌드에서는 `test-support`가 켜지지 않는 것도 확인했어요.
막는 결함은 없었어요.

| 지적 | 처리 |
| --- | --- |
| 1. (b)–(d)는 작업 절의 "지금 동작은 그대로"와 기존 테스트를 고치지 않는다는 완료 기준 밖이에요. | 결정 절에 까닭과 고친 테스트를 적었어요. |
| 2. [0128](0128-keep-unnamed-cycle-torrent.md)이 지운 함수 `rename_torrent`를 지금 일로 적어요. | 고쳤어요. |
| 3. `rename`이 Transmission client를 새로 만들어서, 수집 주기가 이름 바꾸기마다 세션을 다시 맞춰요(409 응답 하나). 예전 수집 주기는 add의 client를 그대로 썼어요. | 이 결과를 담은 커밋에서 `rename`이 부르는 쪽의 client를 받게 했어요. 수집 주기는 add에 쓴 client를 넘기고, `다시 받기`는 이름 바꾸기와 라벨 떼기에 client 하나를 써요. 가짜 Transmission이 세션 맞추기를 세지 않아서 테스트는 더하지 않았어요. |
| 4. `file-count`를 보내지 않는 Transmission(RPC 17 전, 3.x)에서는 수집 주기가 패닉 대신 항목마다 이름 바꾸기 시도를 모두 쓰며 폴더 차례를 쥐고, 이름은 바뀌지 않아요. | 배포하는 Transmission은 4.x라서 닿지 않아요. 남은 것에 적었어요. |
| 5. `Existing` 모드도 `Kept`를 돌려줘요. 0128에서 수집 주기가 메모를 적으면, 이미 있던 토렌트에도 메모가 붙을 수 있어요. | 0128에서 다뤄요. `다시 받기`는 `Added` 모드로만 이름을 바꿔요. |

### 검증한 것

2026-10-08에 `cargo test --locked --workspace -j 4`로 확인했어요. 실패는 모두 0개, 무시는 모두 14개예요.

| 뒤 | 통과 |
| --- | ---: |
| 시작(`7a0f0cf`) | 2,852 |
| `000b078`, `b701931` | 2,856 |
| `ffb6ded`, `9086756`, `e984ee8` | 2,858 |
| `59370f4` | 2,859 |
| `78936a3` | 2,863 |
| `d63e767`, 이 결과를 담은 커밋 | 2,864 |

`ffb6ded`부터 `d63e767`까지의 수는 패닉 문구를 고치기 전에 센 값이에요. 문구를 고친 뒤에는 이 결과를 담은 커밋에서 다시 셌어요.
`cargo fmt --all --check`는 깨끗해요. clippy는 구현 때 `cargo clippy --workspace --all-targets -j 4`로, 이 결과를 담은 커밋에서 trss-collect(`test-support` 포함)로 돌렸고 경고가 없었어요.

### 남은 것

- 수집 주기가 이름을 만들지 못한 새 토렌트를 지우는 것과 메모를 적지 않는 것은 [0128](0128-keep-unnamed-cycle-torrent.md)에서 바꿔요.
- `file-count`를 보내지 않는 Transmission 3.x에서는 수집 주기가 이름을 바꾸지 못한 채 항목마다 이름 바꾸기 시도를 모두 써요(리뷰 지적 4). trss는 그런 Transmission을 따로 알리지 않아요.
- 테스트를 나누는 일은 [0101](0101-receive-line-tests.md)이에요.
