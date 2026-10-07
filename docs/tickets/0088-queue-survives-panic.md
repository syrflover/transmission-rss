# 0088 worker의 백그라운드 큐가 항목 하나의 패닉으로 멈추지 않아요

- 상태: 완료 (2026-10-07)
- 출처: [작품 표지](../specs/library.md#작품-표지), [시즌 정보](../specs/library.md#시즌-정보), [방영작 구독](../specs/collection.md#방영작-구독), [자막 후보 조회](../specs/subtitles.md#자막-후보-조회)
- 막는 티켓: 없음

## 작업

2026-10-07 구조 조사에서 찾았어요.
worker는 백그라운드 큐 네 개를 각자의 task로 띄워요. 표지, 시즌 정보, Anissia 편성 갱신, Anissia 자막 목록이에요.
각 큐는 항목을 루프 안에서 바로 처리해요. 처리 중에 패닉이 나면 그 큐의 task가 끝나요.
worker는 큐의 handle을 종료할 때만 기다려요. 그래서 패닉 메시지가 로그에 한 번 찍힌 뒤로는 worker가 다시 뜰 때까지 그 큐가 돌지 않아요.
수집 주기, 명령, 자막 작업, 수정본 재확인은 실행마다 task를 따로 띄워서 패닉이 그 실행만 끝내요.
지금 패닉을 일으키는 입력은 알려진 것이 없어요. 가상 실패도 고치기로 한 결정(2026-10-07)에 따라 배포하기 전에 고쳐요.

- 패닉한 항목은 그 큐가 실패를 다루는 방식대로 미뤄요. 같은 항목이 곧바로 다시 패닉하는 고리를 만들지 않아요.

## 완료 기준

- 네 큐 각각에, 처리 중 패닉한 항목 뒤에 다음 항목을 처리하는 테스트가 있어요. 패닉한 항목은 실패처럼 미뤄지고, 로그에 큐 이름과 함께 남아요.
- 패닉이 없을 때 큐의 순서, 간격, 잠금은 지금과 같아요.

## 결과

### 만든 것 (2026-10-07)

- trss-core에 `queue::run_item`을 더했어요([ADR 0016](../adr/0016-shared-parts-in-core.md)의 큐 부품 가운데 첫 조각이에요). 큐 항목 하나를 돌리다 패닉이 나면 잡아서 `큐 이름: 항목 panicked: 메시지`를 로그에 남기고, 패닉 메시지를 `Err`로 돌려줘요. 기본 panic hook이 찍는 줄(파일과 줄 번호)도 그대로 남아요.
- 네 큐가 항목을 `run_item` 안에서 처리하고, 패닉한 항목은 각 큐의 실패처럼 미뤄요.

  | 큐 | 로그의 큐 이름 | 패닉한 항목의 처리 |
  | --- | --- | --- |
  | 표지 | `Artwork queue` | 검색·받기 작업을 실패로 미뤄요. 시도 횟수가 늘고 `RETRY_DELAYS`(1분, 10분, 1시간)를 따라요. |
  | 시즌 정보 | `Season queue` | 검색은 표지와 같은 규칙으로 미루고, 정보 다시 받기는 1시간 미뤄요. |
  | Anissia 편성 갱신 | `Anissia queue` | 그 갱신에 든 작품을 모두 1시간 미뤄요. 패닉 전에 이미 받은 작품도 함께 미뤄서, 그 작품은 하루 대신 1시간 뒤에 한 번 더 물어요. |
  | Anissia 자막 목록 | `Anissia caption observation` | 그 읽기를 실패한 읽기로 끝내요. 다음 읽기는 읽기 일정대로 시작 30분 뒤예요. 일정은 첫 요청 전에 이미 써 두므로 바로 다시 읽지 않아요. |

- 테스트에서 패닉을 일으킬 입력이 없어서, trss-core의 `test-support` feature에서만 켜지는 한 번짜리 패닉 지점(`queue::testing::panic_next`)을 `run_item` 안에 뒀어요. 큐의 처리 코드에는 테스트용 호출이 없어요. 배포 빌드는 이 feature를 켜지 않아요.
- 네 큐의 로그 줄은 큐 이름 상수(`QUEUE`)를 써요. 글자는 전과 같아요.

### 검증한 것

| 완료 기준 | 근거 |
| --- | --- |
| 표지 | `artwork::tests::a_job_that_panics_is_put_off_like_a_failure_and_the_next_job_runs`에서 먼저 꺼내는 검색이 패닉하면 `Later`가 되고(시도 1회, 60–65초 뒤), 다른 작품의 검색과 받기가 이어서 돌아요. |
| 시즌 정보 | `seasons::tests::a_search_or_refresh_that_panics_is_put_off_like_a_failure_and_the_next_one_runs`에서 먼저 꺼내는 검색이 패닉하면 `Later`가 되고 다른 작품이 이어서 연결돼요. 65초 뒤에는 패닉한 검색도 연결돼요. 정보 다시 받기가 패닉하면 요청 없이 `RefreshLater`가 되고, 1시간이 지나야 다시 받아요. |
| Anissia 편성 갱신 | `anissia::tests::a_refresh_that_panics_puts_the_anime_off_like_a_failure_and_the_next_one_runs`에서 패닉한 갱신은 요청 없이 `failed: 1`이고, 1시간이 지나야 다음 갱신이 돌아 작품을 받아요. |
| Anissia 자막 목록 | `anissia::captions_tests::a_reading_that_panics_ends_as_a_failed_one_and_the_next_reading_runs`에서 패닉한 읽기는 요청 없이 `panicked: `로 시작하는 실패로 끝나고, 30분이 지나야 다음 읽기가 5줄을 더해요. |
| 로그에 큐 이름과 함께 남음 | `queue::tests::an_item_that_panics_is_told_with_its_queue_and_the_next_one_runs`가 로그 줄의 모양과 `&str`·`String`·그 밖의 패닉 값의 메시지를 확인해요. 위 네 테스트의 큐 이름은 각 큐의 `QUEUE`예요. |
| 패닉이 없을 때 순서, 간격, 잠금이 같음 | 루프(`run_queue`)는 고치지 않았고, 기존 큐 테스트가 고치지 않고 통과해요. |

- `run_item`이 패닉을 잡지 않게 잠깐 바꾸면 네 큐 테스트가 모두 실패하고, 되돌리면 통과했어요. 테스트가 패닉 격리를 실제로 확인한다는 근거예요.
- 2026-10-07(`507da22` 위의 변경)에 `cargo test -j 4 --workspace`가 2,754개 통과, 실패 없이, 13개 무시로 끝났어요(2분 39초). `cargo clippy --workspace --all-targets`도 경고가 없었어요.

### 검증하지 못한 것

- 실제 worker에서 패닉으로 큐가 멈춘 적은 없고, 지금 패닉을 일으키는 입력도 알려진 것이 없어요. 테스트용 패닉 지점으로만 확인했어요.
- 테스트의 패닉 지점은 항목을 시작하기 전에 패닉해요. 처리 도중의 패닉(일부를 DB에 쓴 뒤)도 같은 `catch_unwind`로 잡지만, 그 상태로 미룬 결과는 따로 돌려 보지 않았어요.
