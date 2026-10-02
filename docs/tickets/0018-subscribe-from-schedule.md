# 0018 Anissia 편성표에서 방영작을 구독해요

- 상태: 완료 (앱 YAML 교환 행과 실제 Anissia 화면 확인은 남았어요)
- 출처: [방영작 구독](../specs/collection.md#방영작-구독), [수집 화면](../specs/collection.md#수집-화면)(구독 탭), [채널과 규칙 필드](../specs/settings.md#채널과-규칙-필드)(`rules[].subscription`)
- 막는 티켓: 없음(라이브러리 0012–0017 위에 얹어요)

## 작업

수집 화면 구독 탭에서 Anissia 편성표의 작품을 골라 구독하면, 고른 채널의 실제 RSS 릴리스 제목을 일치 문구로 하는 규칙이 생기고 다음 RSS 확인부터 수집해요.
이 티켓은 첫 화가 이미 채널 기록에 있는 작품의 구독이에요. 첫 화 전의 `제목 대기`와 `제목 후보`는 [0020](0020-title-waiting.md)이에요.

- Anissia 클라이언트: 요일별 편성(`/anime/schedule/<0–6>`), `기타`(7), `신작`(8)과 작품별 자막(`/anime/caption/animeNo/<n>`)을 읽어요. 지금의 `src/anissia.rs`는 기능 플래그 뒤의 쓰지 않는 초안이고 `--features anissia` 빌드가 깨져 있으므로, 이 클라이언트로 바꾸고 플래그를 걷어내요. 시각은 `Asia/Seoul`이에요.
- 편성은 웹이 사용자가 볼 때 Anissia에 묻고 짧게 캐시해요. 구독한 작품의 편성 정보(제목·원제·요일·시각·시작·종료일·상태)는 DB에 남겨 Anissia가 닿지 않아도 규칙 상세와 [이번 주 편성](0021-weekly-schedule.md)이 보여줄 수 있게 하고, worker가 하루에 한 번 다시 받아요. Anissia 요청 간격과 응답 크기 한도는 AniList 클라이언트([0015](0015-work-artwork.md))와 같은 방식으로 둬요.
- 구독 흐름: 작품 고르기 → 채널 → 릴리스 제목(그 채널의 수집 이력에 기록된 항목에서만 고르고 직접 입력하지 않아요) → 자막 제작자(그 작품의 Anissia 자막 목록의 제작자 또는 `제작자 미정`) → 저장 폴더(고른 릴리스 제목에서 만든 제안을 고칠 수 있어요) → 지난 항목 미리보기에서 확인한 것만 받기. 미리보기와 받기는 지금의 규칙 미리보기와 `receive_once` 명령을 써요.
- 저장: 규칙에 `subscription`(`anissia_anime_no`, `creator`, `season_id`)을 더하는 마이그레이션이에요. `season_id`는 [0019](0019-subscription-rule-detail.md)에서 채우므로 여기서는 늘 `null`이에요. Anissia 연결은 구독할 때 확정되고, 이름이 비슷한 다른 작품으로 넓히지 않아요.
- 구독 탭: 이번 분기 구독 목록과 편성표에서 추가하는 입구를 둬요. 카드는 제목을 눌러 규칙을 열어요. 다음 분기 묶음과 제목 후보는 0020이 채워요.
- 별도의 `구독 취소`는 없어요. 구독을 멈추는 일은 규칙 보관이 맡아요([방영작 구독](../specs/collection.md#방영작-구독)).

## 완료 기준

| 입력·상태 | 기대 결과 |
| --- | --- |
| 구독 탭의 편성표에서 수요일 작품을 고르고, SubsPlease 채널 기록의 `[SubsPlease] Work - 01 (1080p)`를 릴리스 제목으로 고름 | 일치 문구가 그 릴리스의 작품 부분이고 저장 폴더 제안이 채워진 구독 규칙이 생겨요. 다음 RSS 확인의 새 회차가 그 규칙으로 추가돼요. |
| 같은 흐름에서 이미 기록된 1–3화 중 1·2화만 확인 | 1·2화만 Transmission에 추가되고 3화는 받지 않아요. 규칙을 만든 것만으로는 아무것도 받지 않아요. |
| 그 채널 기록에 맞는 항목이 없는 작품 | 릴리스 제목을 고를 수 없다고 알리고 규칙을 만들지 않아요(제목 대기는 0020). 직접 입력 칸은 없어요. |
| 자막 목록에 제작자가 없는 작품 | `제작자 미정`만 고를 수 있고, 그대로 구독돼요. 자막 후보가 없으면 할 일이 생기지 않아요. |
| Anissia가 응답하지 않거나 비정상 응답 | 편성표 자리에 까닭과 다시 시도가 보이고, 이미 구독한 규칙의 요일·시각은 저장된 값으로 보여요. |
| `version: 1` 내보내기·앱 YAML 가져오기 | 해당하는 필드 모양은 [채널과 규칙 필드](../specs/settings.md#채널과-규칙-필드)를 따르며, 구독이 아닌 규칙은 `subscription: null`이에요. 교환 형식이 아직 없으면 저장 모양만 맞춰 두고 이 행은 교환 형식 구현 때 확인해요. |
| 휴대폰 너비 | 구독 흐름의 각 단계와 구독 탭에 가로 스크롤이 없어요. |
| 실제 Anissia(`api.anissia.net`) | 요일별 편성과 자막 목록을 한 번 읽어 화면에 보이는지 확인해요. 확인하지 못하면 결과에 남겨요. |

## 결과

### 구현한 것

- Anissia 클라이언트(`crates/trss-collect/src/anissia/`): 기능 플래그와 `tl`·`chrono` 의존을 걷어내고 새로 썼어요. 요일별 편성(0–6), `기타`(7), `신작`(8)과 작품별 자막을 읽고, 시각은 `Asia/Seoul`로 다뤄요. 요청 간격은 `anissia_pace` 표로 웹과 worker가 함께 지켜요(2초 간격, `429`의 `Retry-After` 대기, 없으면 60초, 최대 1시간). 응답은 2 MiB까지, 시간 제한 20초, 리다이렉트는 따라가지 않아요. 웹은 답을 5분 캐시하고 같은 요청은 한 번만 보내며, 자기 차례를 8초까지만 기다려요. 주소는 `TRSS_ANISSIA_URL`로 바꿔요(시험용).
- 저장(`crates/trss-collect/src/store/anissia/`, 마이그레이션 15): 구독한 작품의 편성 값(`anissia_anime`)과 규칙에 붙는 구독(`rule_subscriptions`: `anissia_anime_no`, `subtitles`(`follow`/`undecided`/`none`), `creator`, `season_id`(늘 `null`), `subscribed_at`)이에요. `follow`는 제작자가 있어야 하고 `undecided`는 제작자가 없어야 하며, `none`은 이전에 고른 제작자를 가질 수 있어요. 한 채널에서 같은 작품을 두 번 구독할 수 없어요(`409`).
- 하루 갱신(`crates/trss-collect/src/anissia/mod.rs`, `trss-worker`): 구독한 작품의 편성을 하루에 한 번 다시 받아요. 저장된 요일을 먼저 묻고 없으면 다른 요일을 차례로 물어요. 실패는 1시간 뒤(`429`는 그 시각)로, 편성에서 사라진 작품은 하루 뒤로 미루고 저장된 값은 그대로 둬요. 잠금은 `<DB>.anissia.lock`이에요.
- 순수 로직(`crates/trss-collect/src/subscriptions/`): 릴리스 제목에서 작품 부분·회차를 읽고(`- 01`, `S01E01`, 숫자만 있는 꼴, 그룹 태그), 저장 폴더 제안(`<작품>/Season 01`, 경로에 못 쓰는 글자를 지우고 120바이트까지)을 만들고, 분기를 `Asia/Seoul` 기준으로 정해요.
- 웹 API(`crates/trss-web/src/subscriptions_api.rs`): `GET /api/anissia/schedule/{week}`, `GET /api/anissia/anime/{no}/creators`, `GET /api/subscriptions`, `GET /api/subscriptions/titles`, `POST /api/subscriptions`. 구독은 채널 기록에 있는 작품, 그 요일 편성에 있는 작품, 자막 목록에 있는 제작자만 받고 아무것도 받지 않아요. Anissia가 닿지 않으면 `502`와 까닭(`unavailable`)을 줘요. 규칙 응답에는 `subscription`이 실려요.
- 지난 항목 받기: `receive_once`에 선택적인 `rule_id`를 더했어요. 있으면 그 규칙이 혼자서도 그 항목을 골랐을 때만 받고, 채널이 다르거나 규칙이 보관됐거나 일치하지 않거나 이미 다른 규칙이 받은 항목은 까닭과 함께 끝내요. 주기에는 지난 항목 방어를 더했어요. 구독 규칙은 `subscribed_at`보다 먼저 처음 본 `규칙 불일치`·`제외` 항목을 건드리지 않고, 기록을 읽지 못한 주기에는 구독 규칙의 일치를 건너뛰어요. 이것이 없으면 규칙을 만든 다음 주기가 피드에 남은 지난 항목을 모두 받아요.
- 구독 탭(`web/src/screens/collect/subs/`): 이번 분기 구독 카드(제목을 누르면 규칙이 열려요, 자막 선택·요일과 시각·채널·저장 폴더), 다음 분기에 시작하는 구독은 `다음 분기 구독`에 따로, `편성표에서 추가`(`/collect/subs/add`). 구독 흐름은 작품(요일 탭, 기본은 오늘 요일) → 채널 → 릴리스 제목(채널 기록에서만, 직접 입력 없음) → 자막 → 저장 폴더 → 확인이에요. 자막은 `〇〇 따라 받기`(제작자마다), `제작자 미정`, `받지 않음` 셋이에요. 확인 단계는 지금의 규칙 미리보기를 쓰고, 지난 항목은 체크한 것만 `receive_once`로 받으며 화면이 worker가 끝낼 때까지 따라가요(실패는 `다시 받기`).
- 구독을 멈추는 별도 동작은 없어요. 규칙 보관이 맡아요(결정).
- 앱 YAML 교환 형식이 코드에 아직 없어서(옛 YAML 가져오기만 있고 `subscription: null`로 들어와요) 저장 모양만 맞췄어요.
- `readme.md`에 `Anissia schedule` 절을 더했어요.

### 결정

- 자막 고르기에 `받지 않음`을 더했어요(조정자 결정, 2026-10-01). 완료 기준 표의 "제작자가 없는 작품은 `제작자 미정`만"은 "제작자 목록이 없어도 `제작자 미정`과 `받지 않음`"으로 읽었어요. 구독 규칙의 자막 저장·표시만 여기서 하고, 자막을 받는 일은 다른 티켓이에요.
- 다음 분기에 시작하는 작품을 구독해도 목록에서 사라지지 않도록 구독 탭에 `다음 분기 구독` 구역을 최소로 두었어요. 티켓은 "다음 분기 묶음은 0020이 채운다"고 했으므로 0020이 이 구역을 이어받아 다듬어요.
- 구독하려면 저장 폴더가 비어 있으면 안 돼요(`수집 폴더` 자체에 받는 구독은 없어요). `.`·`./`처럼 구성이 모두 `.`인 폴더도 같아요.
- 지난 항목의 체크는 모두 해제된 채 시작해요(명세의 "확인한 것만 받기"). `모두 선택`·`선택 해제`가 있어요. 체크할 수 있는 항목은 이 규칙이 고르고 기록의 결과가 `규칙 불일치`인 것뿐이에요.
- 이 변경이 `receive_once`의 공개 명령 계약(선택 필드 `rule_id`)과 주기의 핵심 경로(지난 항목 방어)를 건드려서 별도 검토를 권해요.

### 검토 뒤 고친 것

- 규칙 상세 미리보기가 주기의 지난 항목 방어와 어긋났어요. 구독 규칙이 구독 전에 처음 본 `규칙 불일치`·`제외` 항목을 주기는 건드리지 않는데 미리보기는 `이 규칙이 받아요`와 저장 경로로 보여줬어요. 미리보기가 주기와 같은 `ChannelPlan::is_past`를 쓰고 그런 항목을 `past`(`지난 회차`)로 나눠 보여줘요. `past_cause`가 구독 전(`subscribed`)인지 멈춘 동안(`resumed`)인지 알려줘요. 시험: `the_preview_calls_the_items_the_cycle_leaves_alone_past`(`crates/trss-worker/tests/receive_once.rs`, 같은 기록과 규칙에서 주기는 건드리지 않고 미리보기는 `past`), `crates/trss-collect/src/plan.rs`의 `is_past` 시험.
- 지난 항목은 구독 확인 단계에서만 받을 수 있었어요. 탭을 닫았거나 체크를 풀었다가 마음을 바꾸면 받을 길이 없었고, 같은 요청을 다시 보내면 `409`만 받았어요. 규칙 상세 미리보기의 `지난 회차` 행에 `받기`를 두고, 구독 흐름과 같은 `useReceive`로 `receive_once`(`rule_id`)를 보내 같은 상태·`다시 받기`로 따라가요. 항목이 추가되면 미리보기를 다시 불러요. 규칙이 멈춤·보관이거나 저장하지 않은 변경이 있으면 worker가 저장된 규칙으로 받으므로 `받기` 대신 까닭을 적어요. `POST /api/subscriptions`의 `409`(이미 구독 중)에는 `current.rule_id`를 실어, 구독 흐름이 `그 구독 규칙 열기`를 보여줘요. 시험: `a_past_item_of_the_rule_detail_is_received_by_that_rule_and_then_reads_as_received`, `an_anime_is_followed_once_per_channel`.
- 저장 폴더 `.`·`./`는 수집 폴더 자체에 받는데 서버와 구독 흐름이 받아들였어요. 서버(`folders::is_collect_folder_itself`)와 `folderProblem`이 거절해요. 구독이 아닌 규칙은 명세대로 빈 저장 폴더(수집 폴더 자체)를 받으므로 그대로 두었고, `편성표와 연결`은 규칙의 저장 폴더를 그대로 두므로 새로 받는 값이 없어요. 시험: `a_subscription_is_refused_when_what_it_names_is_not_there_and_creates_nothing`의 `.` 행들, `crates/trss-core/src/folders.rs`.
- `rule_id`가 있는 `receive_once`가 규칙이 기록되지 않은 `add_failed` 항목을 "다른 규칙이 받으려다 실패한…"으로 거절했어요. 이제 `NoRule`("규칙 없이 받으려다 실패한…")로 거절해요. 시험: `an_item_that_failed_without_any_rule_is_not_said_to_belong_to_another_rule`.
- 사용자 결정(2026-10-01): 규칙이 멈춘 동안(보관했다가 복원한 경우 포함) 처음 나타난 항목은 다시 켜도 자동으로 받지 않고 구독 전 항목처럼 `지난 회차`로 남아 `받기`로 받아요. 구독 규칙뿐 아니라 멈출 수 있는 모든 규칙에 적용해요. `rules.resumed_at`(마이그레이션 19)에 켠 시각을 적고(`PUT /rules/{id}/switch`의 켜기, `rule_archive` 복원), `is_past`는 `max(subscribed_at, resumed_at)`보다 먼저 처음 본 `규칙 불일치`·`제외` 항목을 지난 항목으로 봐요. 주기와 미리보기가 같은 함수를 써요. 한 번도 켜지 않은 규칙은 그대로예요. 지금 멈춘 규칙의 미리보기는 지금까지 기록된 항목 뒤에 켠 것처럼 보여줘요. 멈춤·보관 안내 문구도 이에 맞췄어요. 시험: `an_item_first_seen_while_a_rule_was_paused_is_left_to_the_user_when_it_resumes`(멈춤 → 항목 → 켬: 주기가 받지 않고 미리보기가 `past`, `받기`가 받음, 켠 뒤 항목은 저절로 받음), `an_item_first_seen_while_a_rule_was_archived_is_left_to_the_user_after_the_restore`(`rule_archive` 보관·복원 명령), `a_rule_that_was_never_paused_takes_the_recorded_items_as_before`, `a_restored_rule_notes_when_it_was_turned_back_on`, 마이그레이션 `a_database_from_before_resume_times_keeps_its_rules_with_no_resume_time`.

### 검증한 것

- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`(합계 777개 통과, 실패 0개)와 웹 `bun run typecheck`, `bun run build`가 통과했어요.
- 시험은 가짜 Anissia(`crates/trss-anissia/src/fake.rs`)와 손으로 옮기는 시계로만 돌아 실제 네트워크에 나가지 않아요. 완료 기준의 행과 시험:
  - 첫 화가 기록된 작품의 구독: `only_the_works_the_channels_history_holds_are_offered_with_a_folder`, `subscribing_creates_the_rule_with_the_chosen_work_and_receives_nothing`(`crates/trss-web/src/subscriptions_api/tests.rs`), 다음 RSS 확인의 새 회차는 `a_subscription_receives_nothing_that_was_recorded_before_it_until_the_user_picks`의 끝부분(`crates/trss-worker/tests/receive_once.rs`).
  - 기록된 1–3화 중 일부만 받기: 위 시험이 25·26화 중 25화만 체크해서 26화는 주기가 거듭돼도 받지 않고, 규칙만 만든 주기는 아무것도 받지 않음을 보여요. 거절은 `a_request_for_a_rule_is_refused_when_the_rule_would_not_pick_the_item`, 다른 규칙이 가진 항목·보관된 규칙·반복 요청은 `a_repeat_of_a_request_is_not_stored_twice_and_an_archived_rule_receives_nothing`, 구독이 아닌 규칙은 그대로인 것은 `a_plain_rule_still_takes_the_items_in_the_feed_the_cycle_has_recorded_without_a_rule`.
  - 맞는 항목이 없는 채널: `a_subscription_is_refused_when_what_it_names_is_not_there_and_creates_nothing`(기록에 없는 작품은 `400`, 규칙은 생기지 않아요).
  - 자막 제작자가 없는 작품: `the_creators_of_an_anime_come_from_its_captions_and_may_be_none`, `an_anime_with_no_creator_is_subscribed_undecided_or_without_subtitles`.
  - Anissia가 응답하지 않음: `when_anissia_does_not_answer_the_reason_is_told_and_nothing_is_kept`(`502`와 까닭), 저장된 요일·시각은 `the_subscription_lists_under_this_quarter_with_its_stored_schedule`와 아래 브라우저 확인.
  - 클라이언트: 캐시 5분·같은 요청 한 번, 웹과 worker가 나누는 간격, `429` 대기(기본 60초·상한 1시간), 응답 크기 한도(`Content-Length` 유무), 잘못된 봉투, 하루 갱신 대기열(요일 이동·사라진 작품·실패·`429`), 잠금 경로는 `crates/trss-anissia/src/tests.rs`가 봐요. 마이그레이션 15와 업그레이드는 `crates/trss-core/src/db.rs`와 `crates/trss-collect/src/store/anissia/tests.rs`가 봐요.
- 브라우저(로컬 `trss-web`, 스크래치 DB, 가짜 Anissia(`TRSS_ANISSIA_URL`), 채널 둘, 1100px와 375px, 2026-10-01):
  - 수요일 작품 → `SubsPlease` 채널 → `LIAR GAME` 릴리스 제목(저장 폴더 `LIAR GAME/Season 01` 제안) → `자막팀 가 따라 받기` → 확인에서 `[Batch]`와 3화는 체크하지 않고 1·2화만 체크해 `구독하고 2개 받기`: 규칙과 구독 행이 생기고 `receive_once` 명령이 1·2화에 대해서만 둘 생겼어요. worker는 띄우지 않아서 DB에서 하나는 `done`, 하나는 `failed`로 바꾸니 화면이 `추가함`과 `추가하지 못함`·까닭·`다시 받기`로 바뀌었고, `다시 받기`는 새 명령을 만들었어요.
  - 기록이 없는 채널은 릴리스 제목을 고를 수 없다는 문장만 보이고 입력 칸이 없어요. 같은 채널에 이미 구독한 작품은 채널이 비활성화돼 `이 채널에서 이미 구독 중이에요`가 보여요.
  - Anissia를 `500`으로 만들면 편성표 자리에 `Anissia가 오류로 답했어요(HTTP 500)…`와 `재시도`가 보이고, `GET /api/subscriptions`는 저장된 값으로 계속 답해요.
  - 375px: 구독 탭(긴 제목·긴 제작자 이름·`다음 분기 구독` 포함)과 구독 흐름의 여섯 단계 모두 `scrollWidth == innerWidth`였어요.
  - 실제 Anissia: 2026-10-01에 읽기 전용 `GET`으로 요일별 편성과 자막 목록의 봉투(`{"code":"ok","data":[…]}`)와 필드 모양(`week`가 문자열, `신작`의 `time`에 날짜, `2027-01-99` 같은 일부 날짜, 빈 `endDate`)을 확인해 파서에 반영했어요.

검토 뒤 고친 것의 확인: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`(합계 849개 통과, 실패 0개)와 웹 `bun run typecheck`, `bun run build`가 통과했어요. 브라우저(로컬 `trss-web`, 스크래치 DB, 가짜 Anissia, 2026-10-01): 구독 흐름의 저장 폴더 칸에 `./`를 적으면 문장이 보이고 `다음`이 막혀요. 확인 단계에서 다른 쪽이 먼저 구독하게 한 뒤 `구독`을 누르면 `409` 문장과 `그 구독 규칙 열기`가 보이고 규칙 상세가 열려요. 상세의 `지난 회차` 행 둘에서 `받기`를 누르면 `추가하는 중`, DB에서 명령을 `failed`로 바꾸면 까닭과 `다시 받기`, 새 명령을 `done`으로 바꾸고 기록을 `received`로 바꾸면 행이 `이 규칙이 받아요`·`기록: 추가함`으로 바뀌어요. `영상 받기`를 끄면 `받기` 자리에 `영상 받기를 켠 뒤에 받을 수 있어요.`가 보이고, 다시 켜면 멈춘 동안 기록한 항목이 `규칙이 멈춰 있는 동안 올라온 항목…`과 함께 `받기`와 나와요(375px도 봤어요). worker는 띄우지 않았어요.

### 검증하지 못한 것

- 앱 화면을 실제 Anissia(`api.anissia.net`)로 돌려 보지 않았어요. 화면은 가짜 서버로만 봤어요.
- 앱 YAML 내보내기·가져오기 행은 교환 형식이 없어서 확인하지 못했어요. 형식을 구현할 때 `rules[].subscription`을 확인해요.
- 지난 항목 받기의 Transmission 쪽(실제 토렌트 추가)은 가짜 Transmission의 통합 시험으로만 봤고, 브라우저에서는 worker를 띄우지 않았어요.
- 하루 갱신은 손으로 옮기는 시계 시험으로만 봤어요.
- 다크 모드, 실제 휴대폰·스크린 리더, 376–1099px 너비, 자막을 실제로 받는 일(다른 티켓)은 보지 않았어요. 자동 화면 시험은 없어요(이 저장소에 웹 시험 도구가 없어요).

### 남은 일

- 첫 배포 뒤 실제 Anissia로 편성 한 요일과 자막 한 작품을 화면에서 확인해요.
- 분기가 바뀌는 날(`Asia/Seoul`)의 `이번 분기`·`다음 분기` 나눔은 시험으로만 봤어요. 실제 날짜 경계는 첫 분기 전환에 확인해요.
- `다음 분기 구독` 구역은 [0020](0020-title-waiting.md)이 제목 대기와 함께 맡아요.
