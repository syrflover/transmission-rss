# 0103 worker 백그라운드 큐의 루프를 trss-core로 모아요

- 상태: 완료
- 출처: [흩어진 개념 모으기](refactoring.md#흩어진-개념-모으기), [ADR 0016](../adr/0016-shared-parts-in-core.md), [ADR 0006](../adr/0006-separate-web-worker-binaries.md)
- 막는 티켓: [0092](0092-refactor-baseline.md)(기준값)

## 작업

worker의 백그라운드 큐 네 개(표지, 시즌 정보, Anissia 편성, Anissia 자막 목록)는 같은 루프를 각자 가져요. 큐마다 파일 잠금을 쥐고, 못 쥐면 60초 뒤 다시 하고, 할 일이 없으면 5초마다 봐요.
잠금 파일의 경로를 만드는 함수가 네 큐와 서버 브라우저에 5개 있고, trss-core의 쓰기 검사가 그 이름을 손으로 나열해요.
표지와 시즌 정보의 다시 하기 규칙은 거의 같고, Anissia의 두 큐는 `429` 뒤 미루는 규칙을 각자 가져요. 패닉 격리는 [0088](../archive/tickets/5-deployed-verification/0088-queue-survives-panic.md)에서 trss-core의 `queue::run_item`으로 들어갔고, 네 큐가 각자 그것을 불러요.

- 루프를 trss-core의 한 구현으로 모으고, 큐마다의 간격, 다시 하기, 미루기는 큐가 넘기는 값으로 지켜요.
- 잠금 파일의 이름은 한 곳에서 정하고, 쓰기 검사가 그 목록을 써요.
- 큐마다 파일 잠금을 쥐는 까닭은 두 번째 worker를 막는 것이에요. worker는 하나만 돌고([ADR 0006](../adr/0006-separate-web-worker-binaries.md)) worker 잠금이 이미 그것을 막아요. 큐마다의 잠금을 남길지 이 티켓에서 사용자 결정을 받아요.

## 완료 기준

- 큐 루프의 구현이 trss-core에 하나 있고, 네 큐가 그것을 써요.
- 잠금 파일 이름이 한 곳에 있고, 쓰기 검사의 목록이 그것에서 나와요.
- 큐마다의 잠금에 대한 사용자 결정과 까닭이 결과 절에 있어요.
- 기존 테스트가 고치지 않고 통과해요.

## 결과

### 모은 곳

| 부품 | 지금 | 지운 복사본 |
| --- | --- | --- |
| 큐 루프 | trss-core `queue::Queue`예요. 큐마다 이름, 잠금 파일 경로, 할 일이 없을 때 다시 보는 간격(`POLL`, 5초), 잠금을 못 쥐었을 때 다시 하는 간격(`LOCK_RETRY`, 60초)을 넘겨요. 표지 큐는 `run_with_upkeep`으로 잠금을 쥐자마자, 그 뒤 10분마다 정리(`maintain`)를 해요. | 표지, 시즌 정보, Anissia 편성, Anissia 자막 목록의 `run_queue` 안 루프 4벌 |
| 잠금 파일 이름 | trss-core `LockFile`(worker, 서버 브라우저, 큐 넷)이에요. 쓰기 검사의 `DATABASE_FILE_SUFFIXES`는 `LockFile::ALL`로 만들어요. | 경로 함수 5개에 적힌 이름과 쓰기 검사에 손으로 적은 목록. 경로 함수 5개는 `LockFile`을 부르는 한 줄로 남았어요. |
| 다시 할 시각 | trss-core `queue::retry`와 `queue::after`예요. 서비스가 기다리라고 하면(`429`) 그만큼 미루고 실패로 세지 않아요. 실패면 큐가 넘긴 지연 목록의 다음 값만큼 미루고, 목록이 끝나면 포기해요. | 표지 큐의 `later`와 시즌 정보 큐의 `put_off`에 있던 같은 계산, 기다림을 밀리초로 더하던 다섯 곳 |

- 패닉 격리는 지금처럼 큐마다 `run_item`으로 해요.
- `POLL`과 `LOCK_RETRY`는 trss-library 표지 큐에서 trss-core로 옮겼어요. 다른 세 큐가 표지 모듈에서 가져다 쓰던 값이에요.
- trss-archive와 trss-probe는 이 부품을 쓰지 않아 복사본이 필요 없었어요.

### 큐마다의 잠금

큐마다 쥐는 파일 잠금은 남겼어요(사용자 결정, 2026-10-08). worker 잠금은 두 번째 worker 프로세스의 수집 주기만 막고, 그 프로세스도 큐는 돌리기 때문이에요. 같은 큐가 두 프로세스에서 함께 돌지 않게 막는 것은 큐마다의 잠금이에요. 이 까닭을 `Queue`가 잠금을 쥐는 자리의 주석에 적었어요.

서버 브라우저의 잠금은 큐의 잠금이 아니에요. worker가 사는 동안 쥐고, 두 번째 worker가 브라우저 컨테이너를 쓰지 못하게 해요. 다만 앱 데이터 폴더에 파일을 만드는 것은 같아서 `LockFile`과 쓰기 검사 목록에 함께 뒀어요.

### 다른 점마다 정한 것

- **루프**: 시즌 정보, Anissia 편성, Anissia 자막 목록의 루프는 같았어요. 표지 큐만 정리를 더 했고, 그 정리는 종료 신호가 끊지 않는 자리에서 돌아요. `run_with_upkeep`이 이 동작을 그대로 지켜요.
- **간격과 잠금을 못 쥘 때**: 네 큐 모두 5초와 60초였고, 잠금 파일을 열지 못하면 같은 문구로 로그를 남기고 60초 뒤 다시 했어요. 동작은 그대로예요.
- **종료**: 하던 항목은 종료 신호에서 그 자리에 버리고, 다음 시작에서 다시 찾는 것이 그대로예요.
- **`429`**: AniList와 Anissia의 오류 형식은 trss-core가 모르므로, `Busy`를 기다림으로 바꾸는 일은 각 크레이트에 남겼어요. Anissia 자막 목록은 지금처럼 다음 읽기 시각과 기다린 뒤의 시각 중 늦은 쪽을 써요.
- **잠금 파일 목록의 순서**: `LockFile::ALL`은 예전 쓰기 검사 목록의 순서(worker, 서버 브라우저, Anissia 편성, Anissia 자막 목록, 표지, 시즌 정보)를 따라요. 쓰기 검사가 문제를 알리는 순서가 이 순서이기 때문이에요.
- 로그 문구, 웹 API의 JSON 모양, 저장 형식은 바뀌지 않았어요.

### 검증한 것

2026-10-09에 `cargo test --locked --workspace -j 4`로 확인했어요. 기존 테스트는 고치지도, 옮기지도, 지우지도 않았어요. 그래서 [RSS 수집 명세의 검증 표](../specs/collection.md#검증-표)는 고칠 것이 없어요.

| 커밋 | 통과 | 실패 | 무시 |
| --- | ---: | ---: | ---: |
| 시작(`fix(collect): say the new video is inside a folder rather than several files`) | 2,811 | 0 | 14 |
| `refactor(core): name the lock files in one place and build the write check's list from it` | 2,813 | 0 | 14 |
| `refactor(core): run the four background queues in one loop in trss-core` | 2,822 | 0 | 14 |
| `refactor(core): compute when a put-off queue item is tried again in trss-core`(이 결과를 담은 커밋) | 2,824 | 0 | 14 |

- trss-core에 테스트 13개를 더했어요. 잠금 파일 이름 1개, 쓰기 검사 목록 1개, 큐 루프 9개, 다시 할 시각 2개예요.
- 큐 루프 테스트는 20–450 ms의 실제 sleep을 써요. 따로 15번 돌려 모두 통과했어요.
- 마지막 행은 세 커밋을 모두 합친 트리에서 다시 돌려 확인했어요.
- `cargo fmt --all --check`는 깨끗하고, clippy에 새 경고가 없어요. trss-core `queue.rs`의 `run_item`에 있는 `redundant_async_block` 경고는 이 티켓 전부터 있었어요.
- 세 커밋의 diff를 예전 루프와 계산에 한 줄씩 대 보았어요. 지운 테스트가 없고 동작을 바꾸지 않는 이동이라 독립 리뷰는 하지 않았어요.

### 남은 것

- 이제 trss-core의 테스트와 겹치는 큐·잠금 테스트 7개를 [0113](0113-remaining-area-tests.md)에서 ADR 0015대로 나눠요. 이 티켓은 기존 테스트를 고치지 않는다는 기준이라 남겼어요.
- 그 정리 뒤에는 크레이트마다 남은 `lock_path_for` 감싸개 5개를 지우고, worker의 `main.rs`가 `LockFile`을 바로 부르게 할 수 있어요.
- 깨우기 소켓의 접미사 `.wake`는 trss-core `access.rs`와 `wake.rs`에 따로 적혀 있어요. 잠금 파일이 아니라 이 티켓에서 다루지 않았어요.
- musl 빌드와 docker, `dev/measure`는 돌리지 않았어요.
