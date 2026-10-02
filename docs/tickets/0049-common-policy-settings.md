# 0049 설정에서 자막 형식과 브라우저 정책을 정해요

- 상태: 완료
- 출처: [공통 정책](../specs/settings.md#공통-정책), [설정 화면](../specs/settings.md#설정-화면), [앱 YAML의 구성](../specs/settings.md#앱-yaml의-구성)의 `subtitle_policy`·`browser`
- 막는 티켓: [0030](0030-feature-crates.md)

## 작업

설정의 `공통 정책` 항목에서 자막 형식 우선순위(ASS·SRT·SMI 각각 한 번, 위아래 이동)와 브라우저 유휴 시간(기본 5분)·동시 작업 수(기본 1개)를 한 정책 버전으로 저장해요.
목록에는 `ASS → SRT → SMI`처럼 현재 값을 보여주고, 작품별로 형식 순서를 재정의한 작품 목록과 그 작품으로 가는 링크를 둬요(재정의의 편집 자리는 결과 목표 4의 작품 상세 자막).
브라우저 값은 [0039](0039-browser-container-lifecycle.md)가 읽어요. 이 티켓이 먼저 끝나지 않아도 0039는 기본값으로 동작해요.
지원 상한은 구현 때 정해 결과에 남겨요.

## 완료 기준

| 입력·상태 | 기대 결과 |
| --- | --- |
| 형식 순서에 SMI를 두 번 넣어 저장하려 함 | 화면에서 만들 수 없고, API로 보내도 변경 없이 거부돼요. |
| 두 화면에서 같은 정책 버전을 열고 한쪽이 먼저 저장 | 다른 쪽 저장은 거부되고, 입력값과 서버 값이 나란히 남으며 `서버 값 사용`·`내 값으로 다시 저장`이 있어요. 자동 재시도는 없어요. |
| 동시 작업 수를 0이나 상한 초과로 입력 | 저장하지 않고 허용 범위를 문장으로 알려요. |
| 값을 바꿈 | `저장`·`되돌리기`가 있는 저장 줄과 마지막 저장 시각이 보여요. |
| 유휴 시간 안내 | 원격 브라우저가 닫혀도 인증 필요 작업은 남고 다시 열면 준비된다는 문장이 있어요. |
| 휴대폰 | 항목이 자기 화면으로 열리고 뒤로 가기로 목록에 돌아와요. |

## 결과

### 만든 것 (2026-10-03)

- **저장(마이그레이션 35)**: `policy_settings`(한 줄: 형식 순서·유휴 시간(초)·동시 작업 수·버전·저장 시각)와 `work_subtitle_policy`(작품별 형식 순서, 작품을 지우면 함께 지워짐)예요. 형식 순서는 표에서도 ASS·SRT·SMI의 여섯 순열만 받아요. 저장한 적이 없으면 줄이 없고, 기본값(`ass,srt,smi`·300초·1개)을 버전 0으로 읽어요.
- **지원 상한**: 유휴 시간 60–3600초(1–60분), 동시 작업 수 1–3개예요. 상한은 `trss-core`의 `IDLE_TIMEOUT_SECONDS`·`CONCURRENT_JOBS`에 있고 API가 `limits`로 화면에 알려요.
- **API**: `GET /api/settings/policy`와 `PUT /api/settings/policy`(본 버전을 함께 보내요)예요. 지난 버전의 저장은 409와 지금 값(`current`)으로 거절하고 아무것도 바꾸지 않아요. 형식이 겹치거나 빠진 순서, 범위 밖의 수(음수 포함)는 400과 허용 범위를 말하는 문장으로 거절해요. 저장 응답은 쓴 그 줄이에요. `overrides`는 작품별 순서를 둔 작품(등록 해제한 감시 폴더의 작품은 빼요)과 그 이름이에요.
- **화면**: 설정의 `공통 정책` 묶음에 `자막 형식·브라우저` 항목 하나가 있고, 목록에 `ASS → SRT → SMI · 5분 · 1개`처럼 지금 값을 보여요. 형식 순서는 위아래 이동, 유휴 시간은 분, 동시 작업 수는 개수로 정해요. 저장 줄에 `저장`·`되돌리기`와 마지막 저장 시각(저장한 적이 없으면 기본값을 쓴다는 문장)이 있어요. 유휴 시간 아래에 원격 브라우저가 닫혀도 인증이 필요한 작업은 남고 작업을 열면 다시 준비된다는 문장이 있어요. 작품별 재정의는 작품으로 가는 링크와 순서로 보여주고, 편집은 그 작품의 상세에서 한다고 적었어요.

### 검증한 것

| 완료 기준 | 근거 |
| --- | --- |
| SMI를 두 번 넣어 저장 | 화면은 이동만 있어서 겹친 순서를 만들 수 없어요. API는 `an_order_with_a_format_twice_or_a_number_out_of_range_is_refused_and_nothing_changes`(겹침·빠짐·모르는 형식 거절 뒤 버전 0 그대로), 표는 `the_table_refuses_an_order_that_is_not_one_and_numbers_that_are_not_positive`예요. |
| 두 화면에서 같은 버전을 열고 한쪽이 먼저 저장 | `a_save_from_a_stale_version_or_out_of_range_changes_nothing`, `a_save_from_a_version_another_screen_saved_over_is_refused_with_the_stored_policy`. 브라우저에서 curl로 새 버전을 먼저 저장한 뒤 화면에서 저장했어요: 409, 입력값이 그대로이고 배너에 내 값과 서버 값이 나란히 있었어요. `내 값으로 다시 저장`은 새 버전으로 한 번 보내 200이었고, `서버 값 사용`은 서버 값을 입력에 넣었어요. 자동으로 다시 보내는 요청은 없었어요. |
| 동시 작업 수 0이나 상한 초과 | 화면에서 0·4·빈 값, 유휴 시간 0·61·1.5에 허용 범위 문장이 나오고 `저장`이 꺼졌어요. 폼을 강제로 제출해도 요청이 없었어요. API는 0·4·-1·2³²+1과 59초·3601초·-300초를 같은 문장으로 거절해요. |
| 값을 바꿈 | 순서를 바꾸면 저장 줄이 켜지고, `되돌리기`가 저장된 순서로 돌렸어요. 저장 뒤 `마지막 저장 10월 3일 00:51`과 목록 값이 바로 바뀌었어요. |
| 유휴 시간 안내 | 화면 문장을 확인했어요. |
| 휴대폰 | 320px·390px에서 항목이 자기 화면으로 열리고 `< 설정`과 브라우저 뒤로 가기로 목록에 돌아왔어요. 목록·패널·배너에 가로 스크롤이 없어요(`scrollWidth`가 `clientWidth`와 같았어요). 320px은 화면으로도 봤어요. |

그 밖에 `the_policy_is_the_defaults_until_saved_and_versioned_after`, `a_format_order_names_each_format_once`, `the_works_with_their_own_order_are_listed_newest_first_and_go_with_their_work`, `a_work_of_an_unregistered_folder_is_not_listed_with_its_own_order`, `a_stored_count_beyond_what_the_app_writes_still_reads_and_can_be_saved_over`, API의 `the_policy_starts_at_the_defaults_and_a_save_answers_it_as_saved`, `the_works_with_their_own_order_are_listed_with_their_names`, 마이그레이션 `a_database_from_before_the_common_policy_keeps_its_settings_and_has_no_policy_row`가 있어요. 화면 확인은 vite 개발 서버로 개발 환경의 API에 붙여 했어요.

### 독립 리뷰

마이그레이션 35와 정책 저장을 두고 독립 리뷰를 받았어요. 막는 결함은 없었어요. 마이그레이션 적용, 거절 때 바뀌는 것이 없음은 시험으로, 버전 계약(첫 저장 둘이 겹쳐도 하나만 성공)은 코드를 따라가 확인했어요. 동시에 저장하는 시험은 돌리지 않았어요.

| 지적 | 처리 |
| --- | --- |
| 작품별 재정의 목록에 등록 해제한 감시 폴더의 작품이 남아, 링크가 없는 작품으로 이어짐 | 등록된 폴더의 작품만 보여요 |
| 표는 양수만 막아서 `u32`보다 큰 값이 들어오면 모든 읽기와 저장이 실패함 | 읽을 때 가장 큰 값으로 읽고, 다음 저장이 범위를 확인해요 |
| 저장 응답이 다시 읽은 값이라 그 사이 다른 저장이 섞일 수 있음 | 쓴 줄로 답해요 |
| 음수나 소수는 범위 문장이 아니라 "요청 내용을 읽지 못했어요"로 거절됨 | 정수는 모두 받아 범위 문장으로 거절해요. 소수는 화면이 보내지 않아요 |

### 남은 한계

- 작품별 형식 순서를 정하는 자리는 아직 없어요(결과 목표 4의 작품 상세 자막). 지금 `work_subtitle_policy`에 쓰는 코드는 없어요.
- 형식 순서와 브라우저 값을 읽어 쓰는 쪽은 아직 없어요. 브라우저 값은 [0039](0039-browser-container-lifecycle.md)가, 형식 순서는 결과 목표 4의 적용이 읽어요.
- 앱 YAML의 `settings.subtitle_policy`·`settings.browser` 내보내기와 가져오기는 결과 목표 5예요.
- 어두운 테마와 개발 환경이 직접 낸 빌드(8080)에서는 화면을 보지 않았어요.

### 커밋

- `1b9d6d1` feat(settings): store the common policy and serve it at /api/settings/policy
- `10a72e0` fix(settings): list only registered works' format orders and read any stored count
- `96b9b10` feat(web): set the subtitle format order and browser policy in settings
