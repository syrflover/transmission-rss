# 0092 리팩터링을 시작하며 기준값을 재요

- 상태: 완료 (2026-10-08)
- 출처: [잣대와 기록](refactoring.md#잣대와-기록)
- 막는 티켓: 목표 5의 모든 티켓([0079](../archive/tickets/5-deployed-verification/0079-musl-test-run.md)–[0091](../archive/tickets/5-deployed-verification/0091-trname-long-and-half-episodes.md), 0.6.1의 [0118](../archive/tickets/5-deployed-verification/0118-applied-copy-creator.md)–[0123](../archive/tickets/5-deployed-verification/0123-collect-moves-archived-work-folder.md)), 0.6.1 뒤에 찾아 리팩터링 전에 고치는 [0124](../archive/tickets/5-deployed-verification/0124-kept-copy-off-episode-row.md)–[0126](../archive/tickets/5-deployed-verification/0126-offset-suggestion-before-first-receive.md)

## 작업

리팩터링의 효과는 workspace 테스트 시간과 "한 개념은 한 곳에"로 판단해요. 변경 범위와 테스트 개수는 앞뒤를 기록만 해요(사용자 결정, 2026-10-07).
다른 리팩터링 티켓보다 먼저, 코드를 바꾸지 않은 상태에서 기준값을 재요. 마지막 측정([0114](0114-refactor-final-check.md))은 같은 명령과 같은 빌드 상태로 해서 이 값과 견줘요.
[0093](0093-migrated-test-db-once.md)(테스트 DB를 한 번만 만들기)은 musl 테스트 시간을 줄이려고 이 기준값보다 먼저 했어요(사용자 요청, 2026-10-07). 기준값에는 그 효과가 이미 들어가고, 그 앞뒤는 0093의 결과 절에 있어요.

- 명령은 `cargo test --workspace -j 4`예요. 개발 PC에서 기본 병렬도로 link하면 swap이 차요.
- 빌드 상태는 "한 번 빌드한 뒤 바꾸지 않고 다시 돌린 것"으로 정해요. 처음 빌드는 캐시 상태에 따라 흔들려서 견주기 어려워요. 처음 빌드 시간도 따로 적어 둬요.
- 잴 때 다른 빌드나 테스트가 같은 PC에서 돌고 있지 않은지 확인해요.
- 변경 범위는 기능·수정 커밋 하나가 건드린 크레이트와 파일 수의 평균이에요. 구조 조사는 2026-10-04–07의 커밋으로 크레이트 3.05개, 파일 15.6개를 냈어요. 같은 방법으로, 리팩터링 직전의 같은 길이 기간을 재요.

## 완료 기준

- [리팩터링 명세의 기록](refactoring.md#기준값과-마지막-측정)에 다음이 있어요. 커밋, 명령, 빌드 상태, cargo의 `Finished … in` 줄, 테스트 바이너리마다의 `finished in`과 그 합, 통과·실패·건너뛴 테스트 수, 변경 범위와 그 기간과 세는 방법.
- 같은 상태로 두 번 재서 차이를 적어요. 차이가 10%를 넘으면 까닭을 찾아 적어요.

## 결과

2026-10-08에 `ff7e6df`에서 측정했어요. 값과 세는 방법은 [리팩터링 명세의 기준값](refactoring.md#기준값)에 적었고, 원자료는 개발 PC의 `dev/local/measure/0092/`에 있어요.

- `cargo test --locked --workspace -j 4`를 한 번 빌드한 뒤 바꾸지 않고 두 번 돌렸어요. 테스트 바이너리 31개의 합은 56.6초와 56.3초여서 차이가 0.5%예요. 두 번 다 2,818개가 통과했고, 실패는 0개, 무시는 14개였어요.
- 변경 범위는 2026-10-05–08의 커밋 49개가 평균 크레이트 2.45개, 파일 12.0개를 건드렸어요.
- 작업에는 "기능·수정 커밋"이라고 적었지만, 구조 조사의 3.05개와 15.6개는 `refactor`와 `perf` 커밋도 센 값이었어요. 조사에 쓴 스크립트를 `dev/measure/change-spread.sh`로 옮겼고, 조사 기간에 돌려 같은 값이 나오는 것을 확인했어요. 견줄 수 있게 기준값도 그 방법으로 셌고, 명세의 구조 조사 기록에 그 커밋 종류를 더했어요.

측정하지 않은 것이에요.

- 처음부터 빌드한 시간은 측정하지 않았어요. 적은 처음 빌드(7.98s)는 이미 있던 `target/`에서 trss-worker 하나만 다시 컴파일한 값이에요.
- musl 대상은 측정하지 않았어요. 작업이 정한 명령은 개발 PC의 평소 실행(glibc)이에요.
- 기준값의 기간이 구조 조사의 기간과 사흘 겹쳐서, 두 값은 서로 독립된 표본이 아니에요.
