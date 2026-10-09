# 0102 요청 간격과 외부 응답 다루기를 trss-core로 모아요

- 상태: 완료
- 출처: [흩어진 개념 모으기](refactoring.md#흩어진-개념-모으기), [ADR 0016](../adr/0016-shared-parts-in-core.md)
- 막는 티켓: [0092](0092-refactor-baseline.md)(기준값)

## 작업

2026-10-07에 web과 worker가 함께 쓰는 요청 간격이 DB에 세 벌 있었어요.
trss-anilist와 trss-anissia의 `pace`는 표 이름(`anilist_pace`, `anissia_pace`)만 다르고, trss-collect의 `search_pace`는 호스트마다 간격을 둬요.
차례를 기다리는 코드도 AniList, Anissia, 지난 회차 검색에 따로 있어요. 기다린 뒤 막힘을 다시 읽는 일은 [0087](../archive/tickets/5-deployed-verification/0087-pace-block-while-waiting.md)에서 세 곳 모두에 들어갔어요.
응답을 다루는 일도 흩어져 있어요.

- `Retry-After`를 초로 읽고 기본값 60초, 상한 1시간을 두는 코드가 trss-anilist, trss-anissia, trss-collect의 피드에 3벌 있어요.
- 응답 크기를 제한해 읽는 코드가 5벌 있고, 2 MiB 상한이 3곳에 따로 적혀 있어요. 자막 출처의 것만 넘치면 자르고, 나머지는 실패로 끝내요.
- 서버 브라우저의 내려받기 상한과 자막 출처의 파일 상한(200 MiB)이 같아야 하는데, 두 곳에 따로 적혀 있어요.

ADR 0016대로 trss-core의 모듈에 둬요.

- 간격 표 세 개는 그대로 두고 표 이름이나 호스트를 받는 하나의 구현을 써요. 마이그레이션은 없어요.
- 자막 출처의 메모리 안 간격(`subtitles/http.rs`의 `Pace`)은 worker 한 프로세스 안의 다른 개념이라 남겨요.
- 응답이 넘칠 때 자를지 실패할지처럼 곳마다 다른 동작은 지금대로 지켜요.

## 완료 기준

- 요청 간격의 저장과 차례 기다리기, `Retry-After` 읽기, 크기를 제한한 읽기가 trss-core에 하나씩 있고, 예전 복사본은 없어요.
- 200 MiB 상한이 한 곳에 있어요.
- 기존 테스트가 고치지 않고 통과해요. 그 뒤 세 벌의 간격 테스트(2026-10-07에 8개)를 ADR 0015대로 정리하고, 앞뒤 개수를 결과 절에 적어요.

## 결과

### 모은 곳

| 부품 | 지금 | 지운 복사본 |
| --- | --- | --- |
| 요청 간격 저장 | trss-core `pace::RequestPace`. `anilist(db)`, `anissia(db)`, `host(db, 호스트)`로 열고, 호스트는 안에서 소문자로 바꿔요. 표 이름과 열 이름은 정해진 세 가지에서만 골라 SQL을 만들어요. | trss-anilist와 trss-anissia의 `pace.rs`, trss-collect `SearchPace`의 SQL |
| 차례 기다리기 | `RequestPace::wait_for_turn`. 자리를 잡고 기다린 뒤 막힘을 다시 읽어요([0087](../archive/tickets/5-deployed-verification/0087-pace-block-while-waiting.md)). 부르는 곳은 `TurnError`를 자기 오류(`Busy`, `SearchError::Wait`)로 바꿔요. | trss-anilist와 trss-anissia의 `turn()`, trss-collect `SearchClient::page` 안의 기다리기 |
| `Retry-After` 읽기 | trss-core `response::retry_after`(`DEFAULT_RETRY_AFTER` 60초, `MAX_RETRY_AFTER` 1시간). 헤더를 찾는 네 줄은 부르는 곳에 남아요. | trss-anilist, trss-anissia, trss-collect `feed`의 3벌 |
| 크기를 제한한 읽기 | trss-core `response::read_or_refuse`(넘으면 실패)와 `read_cut`(상한에서 자름). 응답 대신 조각을 읽는 함수(`reqwest::Response::chunk`)를 받아서 trss-core는 reqwest에 의존하지 않아요. | trss-anilist와 trss-anissia의 `read_answer`, AniList 표지 읽기, trss-collect `feed`의 `read_body`, trss-subtitles의 `read_capped` |
| 2 MiB와 200 MiB 상한 | trss-core `MAX_ANSWER_BYTES`, `MAX_DOWNLOAD_FILE_BYTES`. 예전 이름(`trss_anilist::MAX_ANSWER_BYTES`, `trss_anissia::MAX_ANSWER_BYTES`, `trss_collect::feed::MAX_FEED_BYTES`, `trss_browser::MAX_DOWNLOAD_BYTES`, `trss_subtitles::MAX_FILE_BYTES`)은 이 값을 가리켜요. 테스트, trss-library, trss-jobs와 [작업 명세](../specs/jobs.md)가 그 이름을 써요. | 각 크레이트의 숫자 |

- 자막 출처의 메모리 안 간격(`trss-subtitles` `http.rs`의 `Pace`)은 결정대로 남겼어요.
- trss-archive와 trss-probe는 이 부품을 쓰지 않아 복사본이 필요 없었어요.
- 200 MiB 상한은 서버 브라우저와 자막 출처가 함께 쓰는 가장 아래 크레이트인 trss-core에 뒀어요.
- [ADR 0016](../adr/0016-shared-parts-in-core.md)은 응답 부품 때문에 trss-core가 reqwest에 의존하게 된다고 봤지만, 그 의존은 생기지 않았어요. ADR에 적었어요.

### 다른 점마다 정한 것

- **`Retry-After`**: 세 벌이 같았어요. 정수 초만 읽고, 날짜·소수·음수·빈 값은 60초, 1시간에서 잘라요. 동작은 그대로예요.
- **기다린 뒤의 순서**: trss-anilist와 trss-anissia는 기다린 뒤 시계를 먼저 읽고 막힘을 읽었고, trss-collect는 막힘을 먼저 읽었어요. 시계를 먼저 읽는 쪽으로 맞췄어요. 지난 회차 검색에서는 두 읽기 사이가 DB를 한 번 읽는 만큼 짧아질 뿐이에요. 그 사이에 끝나는 막힘도 지켜지고, 알리는 남은 기다림이 몇 밀리초 길어질 수 있어요. 실제 시계로 재현해 보지는 않았어요.
- **크기를 제한한 읽기**: 논리는 같았어요. AniList 표지 읽기는 버퍼를 한 번 미리 잡는 동작(`reserve`)과 "알린 길이보다 짧음" 검사를 그대로 가져요. 자르는 읽기는 지금처럼 상한 바이트에서 정확히 멈춰요.
- **오류 문구**: `PaceError`의 문구가 `DbError`와 같아서 `SearchError::Pace`의 문구도 그대로예요. 웹의 JSON 모양, 문구, 저장 형식은 바뀌지 않았어요.

### 간격 테스트 정리

2026-10-07의 간격 규칙 테스트 8개(trss-anilist 1, trss-anissia 1, trss-collect `search_pace/tests.rs` 6)는 trss-core `pace.rs`의 규칙 테스트 5개가 됐어요. 클라이언트마다 있던 기다리기 테스트 7개는 연결 테스트 3개만 남았고, 지운 4개는 trss-core의 `wait_for_turn` 테스트 2개가 확인해요. 리뷰 뒤에는 세 서비스가 각자의 표에 간격을 적는지 확인하는 테스트(`each_service_keeps_its_pace_in_its_own_table`)를 trss-core에 더했어요. 생성자 둘이 표를 바꿔 써도 다른 테스트는 모두 통과했기 때문이에요.

| 파일 | 앞 | 뒤 | 한 일 |
| --- | ---: | ---: | --- |
| trss-anilist `pace.rs` | 1 | 0 | trss-core `requests_take_turns_and_a_block_holds_them_all`로 그대로 옮겼어요. |
| trss-anissia `pace.rs` | 1 | 0 | 표만 다른 같은 테스트라 옮긴 뒤 지웠어요. |
| trss-collect `store/search_pace/tests.rs` | 6 | 0(파일 지움) | trss-core의 테스트가 같은 경우를 확인해요. 지난 회차 검색만 확인하던 경우(다음 자리보다 늦은 요청은 기다리지 않음, 가장 긴 기다림 안의 자리는 잡음, 물은 적 없는 호스트는 막힘이 없음)도 trss-core로 옮겼어요. |
| trss-core `pace.rs` | 0 | 8 | 규칙 테스트 5개(옮긴 것 1, 새로 쓴 것 4), `wait_for_turn` 테스트 2개, 리뷰 뒤 더한 표 이름 테스트 1개예요. |
| trss-anilist `lib.rs`의 기다리기 테스트 | 2 | 1 | 기다리는 사이 막히면 `Busy`로 아무것도 보내지 않는 연결 테스트를 남기고, `a_request_waits_for_its_turn_and_is_then_sent`를 지웠어요. |
| trss-anissia `tests.rs`의 기다리기 테스트 | 3 | 1 | 클라이언트 여럿이 DB 하나로 간격을 지키는 연결 테스트를 남기고, `a_caller_that_may_wait_waits_for_its_turn_in_real_time`과 `a_block_that_comes_while_a_request_waits_for_its_turn_stops_the_request`를 지웠어요. |
| trss-collect `past_search/client/tests.rs`의 기다리기 테스트 | 2 | 1 | 한 시간 막힌 호스트에서 검색이 곧바로 실패하는 연결 테스트를 남기고, `a_block_that_comes_while_a_request_waits_for_its_slot_stops_the_request`를 지웠어요. |

간격에 관한 테스트는 이 파일들과 trss-core를 합쳐 15개에서 11개가 됐어요. 지운 테스트는 모두 trss-core의 같은 이름이나 같은 경우의 테스트가 확인해요.
[RSS 수집 명세의 검증 표](../specs/collection.md#검증-표)가 인용하던 `requests_are_given_slots_a_spacing_apart`는 trss-core의 `requests_take_turns_and_a_block_holds_them_all`로 바꿔 적었어요.

### 검증한 것

2026-10-09에 `cargo test --locked --workspace -j 4`로 확인했어요. 정리 커밋까지 기존 테스트는 고치지 않았어요.

| 커밋 | 통과 | 실패 | 무시 |
| --- | ---: | ---: | ---: |
| 시작 | 2,807 | 0 | 14 |
| `refactor(core): read Retry-After, bounded bodies and the download size limit in trss-core` | 2,813 | 0 | 14 |
| `refactor(core): keep the request pace and the wait for a turn in trss-core` | 2,818 | 0 | 14 |
| `test(core): test the request pace rule once in trss-core` | 2,808 | 0 | 14 |
| `refactor(collect): drop the unused host-keyed pace methods of SearchPace`(이 결과를 담은 커밋) | 2,808 | 0 | 14 |
| 리뷰 뒤 표 이름 테스트를 `test(core)` 커밋에 합친 뒤 | 2,809 | 0 | 14 |

- `cargo fmt --all --check`는 깨끗하고, `cargo clippy --workspace --all-targets -j 4`에 새 경고가 없어요. trss-core `queue.rs`의 `redundant_async_block` 경고는 그 전부터 있었어요.
- 한 번은 trss-jobs `tests/extract_process.rs`의 `a_failure_the_child_reports_is_a_failure_and_not_a_refusal`가 `Text file busy`로 실패했다가 다시 돌리면 통과했어요. 스크립트를 쓰는 테스트 여러 개가 한 바이너리에서 함께 도는 탓이고, 이 티켓은 trss-jobs를 바꾸지 않았어요.
- 2026-10-09에 stora:reviewer가 네 커밋의 동작 보존을 검토했어요. 간격 계산, 기다린 뒤 막힘을 다시 읽는 순서, 오류 대응과 문구, 읽기 상한과 `reserve`, 표와 열 이름을 정해진 셋에서만 고르는 SQL과 statement cache, 지운 테스트마다 남은 짝을 봤고, 부르는 쪽의 동작이 바뀐 곳은 찾지 못했어요. 지적 셋은 고쳤어요. 검증 표에 남았던 지운 테스트의 인용, 비어 있던 이 결과 절, 표 이름을 확인하는 테스트가 없던 것이에요.
- 공개 API에서 `trss_anissia::pace` 모듈과 trss-collect의 `SearchPace::take_slot`, `SearchPace::blocked_until`, `PaceError`가 빠졌어요. 워크스페이스 안에서는 쓰는 곳이 없어요.

### 남은 것

- trss-anilist와 trss-anissia에는 `Retry-After` 값을 확인하는 클라이언트 테스트(`a_429_without_a_retry_after_waits_a_minute_and_a_huge_one_is_cut_to_an_hour` 등)가 남아, trss-core의 `retry_after` 테스트와 겹쳐요. 이 티켓의 정리 대상인 간격 테스트가 아니라 남겼고, [0113](0113-remaining-area-tests.md)에서 ADR 0015대로 나눌 수 있어요.
- musl 빌드와 docker, `dev/measure`는 돌리지 않았어요.
