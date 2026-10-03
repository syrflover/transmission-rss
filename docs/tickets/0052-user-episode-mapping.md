# 0052 자막 후보 구역에서 회차 대응을 정하고 `회차 확인 필요`로 물어요

- 상태: 완료
- 출처: [자막의 회차 대응](../specs/library.md#자막의-회차-대응)의 사용자 설정과 묻기(사용자 결정, 2026-10-03), [할 일](../specs/jobs.md#할-일)의 `회차 확인 필요`
- 막는 티켓: [0051](0051-airtime-episode-mapping.md)(미정·어긋남 판단)

## 작업

자동으로 정하지 못했거나 어긋난 대응을 사용자가 자막 후보 구역의 제작자 묶음에서 정해요.

- 제작자 묶음의 `회차 대응 정하기`: `같은 번호`, `앞 시즌에 이어 셈`(앞 시즌 회차 수 합을 알 때만), `직접 차이` 가운데 고르고, 각 선택이 지금 후보 회차를 어느 시즌 회차로 옮기는지 보여줘요. 자동 근거가 있으면 미리 골라 둬요.
- 회차별 예외: Anissia 회차 표시 하나를 시즌 회차 하나나 `받지 않음`으로 정해요. 비교 키는 명세의 숫자 키 규칙을 따라요(`013`과 `13.0`은 같은 키).
- 저장은 버전을 함께 보내고, 낡은 버전이면 409와 지금 대응을 돌려줘요. `자동으로 되돌리기`를 둬요.
- 사용자가 대응을 저장하거나 `자동으로 되돌리기`를 고르면 0051이 남긴 그 전 자동 차이(`retired_offset`)를 지워요. 지우지 않으면 되돌린 뒤에도 앱이 그 전 차이로만 정해요.
- 미정인 구독 제작자 출처와 어긋난 회차는 작품마다 `회차 확인 필요` 할 일 하나로 모으고, 그 제작자 묶음으로 이어요. 정하면 사라져요.
- 사용자가 정한 대응과 예외는 자동 수신, 수정 후보 분류, 제작자를 붙인 자막의 수정본 판단에 같은 방식으로 쓰여요.

## 완료 기준

| 입력·상태 | 기대 결과 |
| --- | --- |
| 미정인 구독 제작자 출처 | `회차 확인 필요` 할 일이 생기고, 누르면 작품 상세의 그 제작자 묶음이 펼쳐져요. |
| `앞 시즌에 이어 셈`을 고름(앞 시즌 12화) | 묶음에 `직접 정함 · 13화 → 1화`처럼 보이고, 할 일이 사라지고, 다음 살핌에서 자동 수신이 이어져요. |
| 앞 시즌 회차 수를 모름 | `앞 시즌에 이어 셈`은 고를 수 없고 까닭이 보여요. |
| `13.5`를 `받지 않음`, `0`을 기본 대응으로 둠 | `13.5`는 받지 않고, 다른 회차는 기본 대응을 따라요. |
| `013`과 `13`에 서로 다른 예외 | 같은 키라서 거절돼요. |
| 두 화면에서 같은 출처를 정함 | 늦은 저장은 409와 지금 대응을 받아요. |
| 앱이 `auto`를 저장하는 중에 사용자가 정함 | 사용자의 대응이 남아요. |
| `자동으로 되돌리기` | 앱이 다시 정하고, 근거가 없으면 미정과 할 일로 돌아가요. 미정으로 돌아갔던 자동 대응의 그 전 차이는 지워져서, 새 근거가 가리키는 다른 차이로도 정해요. |
| 휴대폰 폭 | 정하기와 예외 편집이 가로 스크롤 없이 쓰여요. |

## 결과

### 만든 것 (2026-10-03)

- **사용자 대응**: 자막 후보 구역의 제작자 묶음에 `회차 대응 정하기`와 `자동으로 되돌리기`가 있어요. 대화상자에서 `같은 번호`·`앞 시즌에 이어 셈`·`직접 차이` 가운데 고르고, 각 선택이 지금 후보 회차를 어디로 옮기는지(`13화 → 1화`) 미리 보여요. 예외는 Anissia 회차 하나를 시즌 회차 하나나 `받지 않음`으로 정해요. 두 회차가 한 시즌 회차로 들어오면 경고만 하고 저장은 막지 않아요. 저장하면 대응과 예외가 한 단위의 `user` 대응이 되고, 묶음 줄은 `직접 정함 · 13화 → 1화`·`직접 정함 · 같은 번호 · 예외 13.5 받지 않음`처럼 적어요.
- **API**: `PUT /api/library/works/{id}/seasons/{n}/anissia/sources/{source}/mapping`과 `POST …/mapping/revert`예요. 읽은 버전을 함께 보내고, 낡으면 409와 지금 대응을 돌려줘요. 같은 번호 키의 예외 둘, 범위 밖 값, 되돌릴 사용자 대응이 없음은 400이에요. 저장·되돌리기 뒤에는 바로 다시 살펴서 새 대응으로 받을 회차를 받아요.
- **예외 적용**: 예외는 번호 키(`013`=`13`=`13.0`)로 찾고 차이보다 먼저 적용해요. 자동 수신, 어긋남 판단, 다른 제작자가 붙든 회차 판단, 제작자를 붙인 자막의 수정본 판단, 후보 목록의 수정본 분류가 같은 함수를 써요. `받지 않음`은 사용자가 직접 정한 것이라, 이미 받은 회차의 수정본과 14일 재확인도 멈춰요. 앱이 찾은 어긋남은 받은 회차의 수정본을 막지 않아요(0051).
- **동시성**: 앱의 자동 결정은 `user` 대응을 덮어쓰지 않아요. 대응 버전은 모든 줄이 함께 쓰는 카운터에서 받아서, 되돌린 뒤 앱이 다시 정해도 화면이 쥔 버전과 겹치지 않아요. 자동 수신 작업은 결정 때의 대응 버전을 작업 저장과 같은 트랜잭션에서 다시 확인해서, 그 사이 사용자가 저장했으면 만들지 않아요.
- **할 일 `회차 확인 필요`**: 작품마다 하나이고 읽을 때마다 계산해요. 구독 제작자 출처가 미정이면서 `0`이 아닌 회차를 올렸거나, 예외가 덮지 않은 어긋난 회차가 있으면 생겨요. 카드는 `/library/<id>?season=N&section=candidates&source=<출처>`로 이어져 그 묶음을 열어요. 이 계산이 실패해도 다른 할 일과 배지는 그대로 보여요.
- **기록(마이그레이션 43)**: `subtitle_episode_mappings.version`, `subtitle_mapping_clock`, `subtitle_episode_exceptions`(대응 줄이 지워지면 함께 지워짐)예요. 사용자 저장과 되돌리기는 `retired_offset`을 지워요. 보관 폴더 병합은 이기는 대응의 예외를 함께 옮겨요.
- 규칙 전체는 [자막의 회차 대응](../specs/library.md#자막의-회차-대응), [할 일](../specs/jobs.md#할-일), [구독 제작자 자동 수신](../specs/subtitles.md#구독-제작자-자동-수신)에 있어요.

### 검증한 것

| 완료 기준 | 근거 |
| --- | --- |
| 미정 출처의 할 일과 이어지기 | `an_undecided_subscribed_creator_is_a_to_do_that_goes_when_the_user_maps_it`, `an_undecided_creator_with_only_the_registration_line_asks_nothing_until_a_real_episode_is_posted`. 개발 환경에서 `?source=`로 그 묶음이 열렸어요. 할 일 카드 자체는 복사본에 미정 출처가 없어 화면으로 보지 못했어요. |
| `앞 시즌에 이어 셈` | `continuing_on_from_the_earlier_season_decides_an_undecided_source_and_receives_on`, node 시험 `the group line after continuing on from a 12-episode season …` |
| 앞 시즌 회차 수를 모름 | `the_sum_of_the_earlier_seasons_is_not_known_without_their_episode_counts`, node 시험 `continuing on is not offered when …` |
| `13.5` 받지 않음 | `an_episode_the_user_does_not_receive_is_left_while_the_others_follow_the_default`, `a_conflicting_episode_is_a_to_do_until_an_exception_covers_it`, `a_revision_of_an_episode_the_user_does_not_receive_is_left_while_other_revisions_are_received`, `an_episode_the_user_does_not_receive_is_not_read_again` |
| `013`과 `13`의 예외 | `two_exceptions_of_one_number_are_refused_whatever_they_say`, `two_exceptions_of_one_number_are_refused_with_a_sentence` |
| 두 화면의 저장 | `a_save_from_an_older_version_is_refused_and_changes_nothing`, `a_save_from_a_screen_that_read_an_older_version_is_refused_with_the_current_mapping`. 개발 환경에서 낡은 버전의 저장이 409와 지금 대응을 받았어요. |
| 앱 저장 중 사용자 저장 | `the_user_saving_while_the_app_is_about_to_store_its_decision_keeps_the_users_mapping`, `a_job_decided_under_a_mapping_the_user_has_since_saved_is_not_made` |
| `자동으로 되돌리기` | `a_revert_lets_the_app_decide_again_to_any_offset_the_grounds_now_say`, `a_revert_with_no_grounds_returns_to_undecided_and_the_ask`, `reverting_gives_the_source_back_to_the_app_which_decides_again`. 개발 환경에서 되돌리자 예외가 지워지고 바로 `auto` 0으로 다시 정해졌어요. |
| 휴대폰 폭 | 개발 환경 375px에서 대화상자와 예외 줄에 가로 넘침이 없었어요(요소 폭 측정). |

작업 공간 시험 2,035개가 통과하고(실제 네트워크·docker 시험 4개는 무시), clippy 경고가 없고, 웹 타입 검사와 node 시험 27개가 통과해요. 개발 환경(서버 데이터 복사본)에서 마이그레이션 43이 적용됐고, 대화상자가 예외 대상이 시즌 회차 수 밖이면 `1화 → 30화 (시즌 회차 수 밖)`으로 보였어요.

### 독립 리뷰

마이그레이션, 버전과 409, 앱과 사용자의 쓰기 순서, 예외 적용의 일관성을 두고 리뷰를 받았어요. 막는 결함은 없었고 아래를 고쳤어요.

| 지적 | 처리 |
| --- | --- |
| 미리 보기가 시즌 회차 수 밖의 예외 대상을 `시즌 밖`으로 보이지만 서버는 받음, 회차 수가 서버와 다름 | `32화 (시즌 회차 수 밖)`으로 받는다고 보이고, 서버의 회차 수(일정 대체 포함)를 써요 |
| `받지 않음`이 받은 회차의 수정본과 재확인을 막지 않음 | 막아요(사용자의 직접 결정이라서) |
| 결정과 작업 저장 사이에 사용자가 저장하면 옛 대응으로 작업이 생김 | 작업 저장 트랜잭션에서 대응 버전을 다시 확인해요 |
| 할 일 계산 실패가 할 일 전체를 막고 읽을 때마다 무거움 | 실패는 로그만 남기고, 정해진 대응은 관찰을 읽지 않아요 |
| 대응이 없는 출처의 되돌리기가 문서와 다른 409 | 400이에요 |
| 묶음 줄의 예가 시즌 밖·예외 회차일 수 있음 | 시즌에 들어오고 예외가 없는 첫 회차를 번호로 적어요 |
| 두 회차가 한 시즌 회차로 들어옴 | 대화상자에서 경고해요 |
| `0` 줄만 있는 미정 출처도 할 일 | 묻지 않아요 |

### 한계

- 대응을 바꿔도 이미 받은 보관본과 미완료 작업을 새 기준으로 다시 평가하지 않아요. [자막의 회차 대응](../specs/library.md#자막의-회차-대응)의 그 요구는 아직 어느 티켓에도 없어요.
- 작품 폴더가 보이지 않는 동안 되돌린 구독 제작자 출처는 대응 줄이 없어서 할 일도 생기지 않아요. 폴더가 돌아와 다시 살피면 생겨요.
- 받은 파일 이름의 회차가 대응과 다른지 견주는 것은 목표 4의 묶음 분석이에요.
- 두 회차가 한 시즌 회차로 들어오는 대응은 경고만 하고 저장돼요. 그러면 두 자막이 같은 회차로 받아져요.
- 할 일 계산 실패 경로에는 시험이 없어요.
