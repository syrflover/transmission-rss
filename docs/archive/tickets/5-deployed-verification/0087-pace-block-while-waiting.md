# 0087 AniList와 Anissia 요청이 차례를 기다리는 사이에 생긴 막힘을 지켜요

- 상태: 완료 (2026-10-07)
- 출처: [작품 표지](../../../specs/library.md#작품-표지), [시즌 정보](../../../specs/library.md#시즌-정보), [작품 연결과 제외](../../../specs/library.md#작품-연결과-제외)
- 막는 티켓: 없음

## 작업

2026-10-07 구조 조사에서 찾았어요.
AniList와 Anissia 요청은 web과 worker가 함께 쓰는 요청 간격을 DB에 두고, 차례가 올 때까지 기다린 뒤 보내요. 상대가 `429`로 기다리라고 하면 그 시각까지 막아요.
지난 회차 검색은 차례를 기다린 뒤 막힘을 다시 읽어서, 기다리는 사이에 다른 요청이 받은 `429`를 지켜요(`a35963c`).
AniList(`trss-anilist`의 `turn`)와 Anissia(`trss-anissia`의 `turn`)는 다시 읽지 않아요. 그래서 기다리는 사이에 막힘이 생겨도 요청을 보내요.

리팩터링에서 세 곳의 요청 간격을 trss-core 하나로 모으지만, 동작을 바꾸는 이 수정은 배포하기 전에 지금 코드에서 따로 해요(사용자 결정, 2026-10-07).

## 완료 기준

- AniList와 Anissia 각각에, 차례를 기다리는 요청이 그 사이에 생긴 막힘을 만나면 보내지 않고 남은 시간과 함께 `Busy`로 끝나는 테스트가 있어요. 가짜 서버는 그 요청을 받지 않아요.
- 막힘이 없으면 기다린 뒤 지금처럼 보내요.

## 결과

### 만든 것 (2026-10-07)

- trss-anilist와 trss-anissia의 `RequestPace`에 `blocked_until`을 더했어요. 두 `turn`은 차례를 기다린 뒤 막힘을 다시 읽고, 막힘이 지금보다 뒤면 요청을 보내지 않고 남은 시간과 함께 `Busy`로 끝나요. 지난 회차 검색(`SearchClient::page`)과 같은 방식이에요.
- 기다리지 않은 요청(차례가 지금인 요청)은 다시 읽지 않아요. 차례를 받을 때 이미 막힘을 반영했기 때문이에요.
- 두 크레이트의 모듈 설명에 기다리던 요청도 막힌다는 것을 적었어요.

### 검증한 것

| 완료 기준 | 근거 |
| --- | --- |
| 기다리는 사이에 생긴 막힘을 만나면 보내지 않고 남은 시간과 함께 `Busy` | AniList는 `trss-anilist`의 `tests::a_block_that_comes_while_a_request_waits_for_its_turn_stops_the_request`, Anissia는 `trss-anissia`의 같은 이름 테스트가 확인해요. 다른 요청이 차례를 먼저 잡고(간격 600 ms), 기다리는 사이 150 ms에 10초 막힘을 걸어요. 시계를 고정해 남은 시간이 정확히 10초인 `Busy`여야 하고, 가짜 서버가 받은 요청이 없어야 통과해요. 고치기 전에는 둘 다 실패했어요. AniList는 `Ok(Some(…))`, Anissia는 `Ok([])`로 요청을 보냈어요. |
| 막힘이 없으면 기다린 뒤 보냄 | AniList는 새 테스트 `a_request_waits_for_its_turn_and_is_then_sent`(300 ms를 기다린 뒤 요청 1개), Anissia는 기존 테스트 `a_caller_that_may_wait_waits_for_its_turn_in_real_time`(두 요청 사이 250 ms 이상)이 확인해요. |

2026-10-07(`8b60b11` 위의 변경)에 `cargo test -j 4`로 trss-anilist 13개와 trss-anissia 49개가 모두 통과했어요.
두 클라이언트를 쓰는 trss-library, trss-collect, trss-web, trss-worker도 함께 돌려 1,585개가 통과했고 4개는 원래대로 무시됐어요.
