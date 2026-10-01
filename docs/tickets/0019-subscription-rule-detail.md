# 0019 구독 규칙을 작품·시즌에 잇고 규칙 상세 위 네 줄과 받기 스위치를 보여줘요

- 상태: 완료 (실제 Anissia·실제 Transmission으로 받은 영상의 시즌 연결과 409 화면 확인은 남았어요)
- 출처: [방영작 구독](../specs/collection.md#방영작-구독), [수집 화면](../specs/collection.md#수집-화면)(규칙 상세), [머리와 시즌](../specs/library.md#머리와-시즌), [규칙 보관](../specs/collection.md#규칙-보관), [구독 제작자 자동 수신](../specs/subtitles.md#구독-제작자-자동-수신), [채널과 규칙 필드](../specs/settings.md#채널과-규칙-필드)(`season_id`)
- 막는 티켓: [0018](0018-subscribe-from-schedule.md)

## 작업

구독 규칙이 받은 영상이 라이브러리에 나타나면 그 작품·시즌에 구독을 잇고, 규칙 상세 맨 위와 작품 상세 머리에 구독 정보를 보여줘요.

- 시즌 연결: 구독 규칙의 수집 이력 항목으로 받은 영상이 감시 폴더의 어느 시즌 폴더에 나타났는지로 `season_id`를 정해요. 저장 폴더 경로나 이름의 비슷함만으로 잇지 않아요. 한 번 이은 시즌은 사용자가 바꾸기 전까지 그대로이고, 그 시즌이 다른 Anissia 작품에 이미 이어져 있으면 잇지 않고 까닭을 규칙 상세에 남겨요.
- 작품 상세 머리의 한국어 작품명은 이어진 시즌이 있는 작품에서 Anissia `subject`예요([0017](0017-season-info.md)이 폴더 이름으로 비워 둔 자리). 선택 시즌의 자막 제작자도 머리에 보여요.
- 규칙 상세 맨 위: 큰 포스터 옆에 방영 요일·분기, 자막 제작자와 `변경`, 받은 회차 진행, 마지막 수집의 네 줄을 모든 규칙에 똑같이 둬요. 값이 없으면 `미정`·`아직 없음`으로 채워 줄 수를 유지해요. 포스터는 이어진 작품의 표지([0015](0015-work-artwork.md))이고 없으면 같은 크기의 빈 자리예요. 휴대폰에서는 상태·제목·원제가 전체 너비로 올라가고 포스터와 네 줄만 나란히 있어요.
- 받은 회차 진행: 이어진 시즌의 영상 보유 회차 수 / 그 시즌 AniList 회차 수예요. 총 회차를 모르면 남은 부분을 점선으로 두고 `9 / ?화 받음`이에요.
- 자막 제작자 `변경`: Anissia 자막 목록의 제작자 또는 `제작자 미정`으로 바꿔요. 작품 상세 머리의 `제작자 변경`도 같은 값을 바꿔요.
- `영상 받기`·`자막 받기` 스위치: 네 줄 아래에 두고, 구독이 아닌 규칙에는 `영상 받기`만 있어요. `영상 받기`를 끄면 규칙 상태가 새 `paused`(`멈춤`)가 되어 수집하지 않고 폴더는 그대로예요. 규칙 상태에 `paused`를 더하는 마이그레이션과 RSS 평가·미리보기·목록 배지(`수집 중`·`멈춤`·`보관됨`·`제목 대기`)·멈춤 배너를 함께 바꿔요. `자막 받기`는 구독의 자막 방식(`follow`·`undecided` ↔ `none`)을 바꾸며, 끌 때 정해 둔 제작자를 기억했다가 켜면 되돌려요. 영상 받기가 꺼졌거나 보관된 규칙에서는 비활성이고 까닭 문장이 있어요. 스위치는 바로 적용되고 저장 줄을 거치지 않으며, 버전이 어긋나면 `409`예요. 보관·복원(0011)은 그대로이고, 복원하면 `영상 받기`가 켜져요.
- 편성표와 연결되지 않은 규칙은 네 줄 자리에 `편성표와 연결`을 두고, 편성표에서 작품과 제작자를 골라 기존 규칙을 구독으로 만들어요. 일치 문구·저장 폴더는 바꾸지 않아요.

## 완료 기준

| 입력·상태 | 기대 결과 |
| --- | --- |
| 구독 규칙이 받은 1화가 `Work/Season 01/`에 나타남 | 규칙의 `season_id`가 그 시즌이 되고, 작품 상세 머리의 한국어 작품명이 Anissia 제목, 선택 시즌의 제작자가 구독 제작자로 보여요. |
| 다른 규칙이 받은 영상이나 사람이 넣은 영상만 있는 같은 이름의 작품 폴더 | 구독이 그 시즌에 이어지지 않아요. |
| 이어질 시즌이 다른 Anissia 작품에 이미 이어져 있음 | 잇지 않고 까닭이 규칙 상세에 보여요. |
| 9화를 받았고 AniList 회차 수가 미상인 구독 | 진행 막대의 남은 부분이 점선이고 `9 / ?화 받음`이에요. 회차 수가 12면 `9 / 12화 받음`이에요. |
| 구독이 아닌 규칙, 받은 것이 없는 구독, 표지가 없는 작품 | 네 줄이 모두 있고(`미정`·`아직 없음`), 포스터 크기가 다른 규칙과 같아요. 구독이 아닌 규칙에는 `편성표와 연결`이 보여요. |
| 기존 규칙에서 `편성표와 연결`로 작품과 제작자를 고름 | 그 규칙이 구독이 되고 일치 문구·저장 폴더·순서는 그대로예요. |
| 규칙 상세에서 제작자를 `변경` | 규칙 상세와 작품 상세 머리의 제작자가 함께 바뀌어요. 오래된 버전으로 보낸 변경은 `409`예요. |
| 구독 규칙의 `영상 받기`를 끔 | 배지가 `멈춤`이고 다음 RSS 확인에서 그 규칙에 맞는 항목을 받지 않으며(미리보기도 같음), 작품 폴더는 제자리예요. `자막 받기`가 비활성이 되고 `영상 받기를 켜야 자막을 받을 수 있어요`가 보여요. 다시 켜면 다음 확인부터 받아요. |
| `SubKor 따라 받기`인 구독의 `자막 받기`를 껐다 켬 | 끄면 자막 줄이 `받지 않음`, 켜면 다시 `SubKor 따라 받기`예요. 편성표 연결은 그대로예요. |
| 보관된 규칙 | 두 스위치가 꺼진 채 비활성이에요. 복원하면 `영상 받기`가 켜지고 `자막 받기`는 보관 전 설정이에요. |
| 구독이 아닌 규칙 | `영상 받기`만 있고 `자막 받기`는 없어요. |
| 휴대폰 너비 | 상태·제목·원제가 전체 너비이고 포스터와 네 줄이 나란히 있어요. 가로 스크롤이 없어요. |

## 결과

### 구현한 것

- 마이그레이션 16(`src/store/channels/paused.sql`): `rules.state`에 `paused`를 더하려고 `rules`를 다시 만들고(외래 키·구독 행 유지), `rule_subscriptions.season_blocked`를 더했어요. 옛 DB가 규칙·구독을 그대로 두고 `paused`를 받아들이는 것은 `src/store/db.rs`의 시험이 봐요.
- `paused` 처리: RSS 평가·미리보기는 보관처럼 맞는 항목이 없고 뒤의 규칙을 가리지 않아요(`src/worker/plan.rs`). 수집 상태·기록·주기·`receive_once`(멈춘 규칙의 지난 항목 받기는 보관처럼 거절)·Anissia 하루 갱신(멈춘 규칙의 작품은 계속 갱신, 보관은 제외)은 깨지지 않게 맞췄고, 옛 YAML 가져오기는 그대로 `active`를 만들어요. 일반 규칙 저장(`PUT /rules/{id}`)은 규칙을 멈추게 할 수 없고, 멈춘 규칙도 필드는 저장돼요.
- 스위치(`PUT /api/rules/{id}/switch`): `{ version, video?, subtitles? }`. `영상 받기` 끔은 `paused`, 켬은 `active`예요. `자막 받기` 끔은 `subtitles = none`(제작자 유지), 켬은 제작자가 있으면 `follow`, 없으면 `undecided`예요. 영상 받기가 꺼졌거나 보관됐거나 구독이 아니면 자막 스위치는 거절돼요. 버전이 어긋나면 `409`와 `current`예요. 보관·복원은 그대로이고 복원하면 `영상 받기`가 켜져요. 어느 스위치도 Anissia 연결을 건드리지 않아요.
- 제작자 변경(`PUT /api/rules/{id}/creator`)과 `편성표와 연결`(`POST /api/rules/{id}/subscription`, 일치 문구·저장 폴더·순서 유지): `src/web/subscriptions_api.rs`. 제작자는 그 작품의 Anissia 자막 목록의 제작자 또는 `제작자 미정`이고, 자막을 받지 않는 구독(`none`)에서는 바꿀 수 없어요.
- 시즌 연결(`src/worker/season_link.rs`, worker 주기의 `watch::scan_all` 다음): 구독 규칙의 `received` 기록 항목 → 그 토렌트의 Transmission 파일 경로 → 라이브러리가 그 영상을 가진 (작품, 시즌) 순서로만 정해요. 폴더 이름의 비슷함은 보지 않아요. 영상이 여러 시즌에 나타나면 잇지 않고, 한 번 이은 시즌은 그대로예요. 그 시즌이 다른 Anissia 작품에 이미 이어져 있으면 잇지 않고 `season_blocked`에 남겨 규칙 상세에 까닭을 보여요(그 구독이 사라지면 다음 확인에서 다시 이어 봐요). 멈춘 규칙도 이미 받은 영상으로 이어져요.
- API: 규칙 응답에 `season`(작품·시즌·표지·보유 회차·AniList 회차 수)과 `season_blocked`, 구독 목록의 `quarter`·`state`(멈춘 구독도 나와요), 작품 상세의 `korean_title`(이어진 시즌 중 가장 낮은 번호의 Anissia `subject`)과 `subscriptions`.
- 웹: 규칙 상세 맨 위 네 줄과 포스터(이어진 작품의 표지, 없으면 같은 크기의 빈 자리), 진행 막대(`9 / 12화 받음`, 총 회차 미상이면 `9 / ?화 받음`과 점선), `영상 받기`·`자막 받기` 스위치(`role="switch"`, 비활성 까닭 문장, 버전 어긋남이면 지금 상태로 바꿔 보여줘요), 멈춤 배너, 시즌 연결 거절 안내, `편성표와 연결`, 제작자 `변경`(규칙 상세와 작품 상세 머리가 같은 `CreatorPicker`), 목록 배지(`수집 중`·`멈춤`·`보관됨`·`제목 대기`), 구독 카드의 `멈춤` 표시, 작품 상세 머리의 Anissia 제목·제작자·`제작자 변경`.
- `readme.md`에 `Season link` 절을 더했어요.

### 결정

- `영상 받기`를 끄면 규칙 상태가 `paused`가 되고 보관과는 별개예요(조정자 결정). 멈춘 규칙의 폴더는 옮기지 않아요.
- `구독 취소`는 어디에도 없어요.
- 작품 상세 머리의 한국어 작품명은 이어진 구독이 여럿이면 가장 낮은 시즌 번호의 것이고, 선택한 시즌에 구독이 있으면 그 시즌의 제작자를 보여요.
- 스위치는 저장 줄을 거치지 않아요. 규칙 편집의 "바뀐 곳 없음" 비교(`sameDraft`)는 상태를 무시해서 스위치로 저장 줄이 깜박이지 않아요.
- 이 변경이 마이그레이션(규칙 표 재생성), 규칙 상태 계약과 worker 주기의 핵심 경로를 건드려서 별도 검토를 권해요.

### 검증한 것

- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`(합계 824개 통과, 실패 0개), 웹 `bun run typecheck`, `bun run build`가 통과했어요. 전체 `cargo test` 첫 실행에서 `artwork::tests::an_image_whose_caller_went_away_is_stored_whole_and_then_cleaned_up`이 한 번 실패했고, 이 변경이 건드리지 않은 시간 의존 시험이에요. 단독으로 세 번, 전체로 한 번 더 돌리니 모두 통과했어요.
- 완료 기준의 행과 자동 시험:
  - 시즌 연결(`Work/Season 01/`에 나타난 1화), 다른 규칙·사람이 넣은 영상, 이미 이어진 시즌, 여러 시즌, 멈춘 규칙: `tests/season_link.rs`(다섯 시험)와 `a_season_is_connected_once_and_a_season_another_anime_holds_is_noted_not_taken`(저장소).
  - 작품 상세의 한국어 작품명·제작자: `the_view_of_a_connected_subscription_has_its_season_and_progress`, `a_work_without_a_connected_subscription_has_no_korean_title`, `the_view_says_which_anime_holds_the_season_that_kept_a_rule_unconnected`(`src/web/subscriptions_api/tests.rs`).
  - `편성표와 연결`: `an_existing_rule_becomes_a_subscription_and_keeps_what_it_was`(저장소), `an_existing_rule_is_linked_to_the_schedule_keeping_phrase_folder_and_order`(API).
  - 제작자 변경과 `409`: `the_creator_changes_to_another_one_or_to_undecided_and_a_stale_version_conflicts`, `the_creator_is_changed_to_one_of_the_anime_or_to_undecided`, `the_creator_is_not_changed_without_subtitles_or_a_subscription`.
  - `영상 받기` 끔·켬: `turning_video_receiving_off_pauses_the_rule_and_on_collects_again`, `video_receiving_pauses_the_rule_and_leaves_its_folder_and_subscription`, 미리보기와 평가는 `paused_rules_never_apply_and_never_shadow_a_later_rule`, `a_paused_rule_matches_nothing_in_the_preview_and_does_not_shadow_a_later_one`. 비활성 까닭은 `the_switches_refuse_what_the_detail_disables`와 아래 브라우저 확인.
  - `자막 받기` 끔·켬: `turning_subtitle_receiving_off_keeps_the_creator_and_on_follows_it_again`, `a_subscription_without_a_creator_returns_to_undecided`, `subtitle_receiving_is_off_none_and_on_follows_the_kept_creator`.
  - 보관·복원: `an_archived_rule_is_restored_not_switched`, `subtitles_are_switched_only_for_a_collecting_subscription`. 오래된 버전은 `a_switch_with_an_old_version_is_a_conflict_with_the_current_rule`.
  - 마이그레이션: `a_database_from_before_paused_rules_keeps_rules_and_subscriptions_and_accepts_paused`.
- 브라우저(로컬 `trss-web`, 스크래치 DB, 가짜 Anissia(`TRSS_ANISSIA_URL`), 데스크톱과 375px, 2026-10-01): 네 줄과 `9 / ?화 받음`의 점선 남은 부분, `영상 받기` 끔·켬(`멈춤` 배지·배너·`자막 받기` 비활성과 까닭), `자막 받기` 끔·켬(`받지 않음` ↔ `SubKor 따라 받기`), 규칙 상세와 작품 상세 머리에서 제작자 변경(목록에 반영), 구독이 아닌 규칙의 `편성표와 연결`(이미 구독 중인 작품 거절, 일치 문구·저장 폴더 유지한 채 연결 성공), 이어질 시즌이 막힌 규칙의 안내 문장, 보관된 규칙(`영상 받기` 꺼진 채 비활성, `자막 받기` 없음), 받은 것이 없는 구독(`아직 없음`). 375px에서 상태·제목·원제가 전체 너비이고 포스터와 네 줄이 나란하며 `scrollWidth == innerWidth`였어요.

### 검증하지 못한 것

- 시즌 연결은 가짜 Transmission의 통합 시험으로만 봤어요. 브라우저에서는 worker를 띄우지 않아 연결된 시즌은 DB에 직접 넣었어요. 실제 Transmission·실제 Anissia로 확인하지 못했어요.
- `409`가 났을 때의 화면 문구(지금 상태로 바꿔 보여주기)는 API 시험으로만 보고 브라우저에서 일으켜 보지 않았어요.
- 다크 모드, 실제 휴대폰·스크린 리더, 376–1099px 너비. 자동 화면 시험은 없어요(이 저장소에 웹 시험 도구가 없어요).

### 남은 일

- 받은 토렌트를 Transmission이 지운 규칙은 근거(파일 경로)를 잃어 시즌을 잇지 못해요. 이어진 뒤에는 상관없어요.
- 자막 자동 수신은 이 저장소에 없어서 `자막 받기`는 저장된 방식만 바꿔요([구독 제작자 자동 수신](../specs/subtitles.md#구독-제작자-자동-수신)의 구현은 다른 티켓).
- 연결(자동·`편성표와 연결`)이 규칙 버전을 올려서, 열어 둔 규칙 편집이 한 번 `409`를 받을 수 있어요.
