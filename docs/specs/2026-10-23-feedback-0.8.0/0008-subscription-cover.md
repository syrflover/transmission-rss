# 0008 구독할 때 AniList에서 표지를 찾아 첫 화 전의 편성 카드에 보여줘요

- 상태: 완료
- 출처: [명세의 편성 카드의 표지](spec.md#편성-카드의-표지), [작품 표지 ADR](../../adr/0007-anilist-work-artwork.md), 사용자 피드백 7번(2026-10-23)
- 막는 티켓: 없음

## 작업

이번 주 편성 카드와 규칙 상세의 포스터는 구독이 라이브러리 시즌에 붙은 뒤에만 작품의 표지를 보여줘요. 구독은 그 규칙이 받은 영상으로 시즌에 붙으므로, 첫 화를 받기 전에는 빈 칸이에요. 사용자는 이번 주 편성 카드에서 표지가 없는 것을 봤어요(2026-10-23). 그것이 이 경우였는지는 확인하지 않았고, [명세](spec.md#가정과-확인하지-않은-것)는 그렇다고 가정해요.

사용자가 정한 것은 다음과 같아요(2026-10-23).

- 구독할 때 trss가 AniList에서 그 작품의 표지를 찾아, 첫 화 전의 카드에 써요.
- 작품이 생기면 그 표지를 작품에 넘겨요.
- 속편만 작품의 표지를 쓰는 안과 지금처럼 비워 두는 안은 고르지 않았어요.

후보를 고르는 기준은 [표지 자동 판정](../../requirements/library.md#자동-판정)을 따라요. 첫 결과나 유사도 순위로 고르지 않아요. 구독은 사용자가 Anissia 작품을 골라 만든 것이라, Anissia의 `originalSubject`와 `startDate`를 힌트로 써도 돼요. 이 명세가 정한 기준은 정규화한 원제가 AniList 항목의 원제와 정확히 같고, 방영 시작일이 맞는 후보가 하나뿐임을 끝까지 확인했을 때예요. 원제가 없거나 후보가 모호하면 표지를 두지 않아요.

구독을 만드는 곳은 구독 화면과 규칙의 연결(지금은 `편성표와 연결`, [0007](0007-rule-anissia-link.md) 뒤에는 `Anissia 연결`)이에요. 찾기와 받기는 구독 만들기를 막지 않고, 작품 표지의 수신 큐처럼 요청 간격과 재시도를 지켜요. 이미지는 앱 데이터 폴더에 두고([작품 폴더 배치 ADR](../../adr/0008-trss-folder-and-app-data-files.md)), 이미지를 제공자의 허용된 주소에서만 받는 규칙도 같아요.

작품에 넘기는 때는 구독이 시즌에 붙는 때예요. 그 작품의 표지가 자동 상태이고 이미지가 없을 때만 넘겨요. 작품이 생길 때 폴더 이름으로 하는 자동 검색이 이미 표지를 골랐거나, 사용자가 고르거나 비운 표지는 바꾸지 않아요. 작품에 넘기기 전에 구독이 지워지면(보관 포함) 그 이미지를 지워요. 이 변경 전에 만든 구독은 다시 찾지 않아요. 2026-10-23 서버에서 시즌에 붙지 않은 구독은 [0003](0003-archive-deletes-the-rule.md)이 지울 `Seihantai na Kimi to Boku`의 것 하나뿐이었기 때문이에요.

## 완료 기준

- [x] 구독을 만들면 trss가 Anissia 원제로 AniList를 찾아, 정확히 맞고 시작일이 맞는 후보가 하나일 때만 표지를 받아 구독에 둬요. 후보가 없거나 여럿이거나, 검색 페이지가 남았거나, 요청이 실패하면 표지를 두지 않아요. 가짜 AniList로 테스트해요.
- [x] AniList가 실패하거나 느려도 구독은 바로 만들어져요. 테스트로 확인해요.
- [x] 구독이 시즌에 붙기 전에는 이번 주 편성 카드와 규칙 상세의 포스터가 구독의 표지를 보여주고, 구독의 표지도 없으면 빈 칸이에요. API 테스트로 확인해요.
- [x] 구독이 시즌에 붙을 때 작품의 표지가 자동 상태이고 이미지가 없으면 구독의 표지가 작품의 표지가 되고, 이미 이미지가 있거나 사용자가 고르거나 비운 표지는 그대로예요. 테스트로 확인해요.
- [x] 작품에 넘기기 전에 구독을 지우면 그 이미지 파일도 지워져요. 테스트로 확인해요.
- [ ] 개발 환경에서 다음 분기(`신작`) 작품 하나를 구독해 카드에 표지가 보이는지 보고, 결과에 적어요. 실제 AniList를 쓰므로 요청 수를 그 한 번으로 줄여요.
- [x] [라이브러리 명세의 작품 표지](../../requirements/library.md#작품-표지), [웹 앱 공통 명세의 이번 주 편성](../../requirements/web-app.md#이번-주-편성), [RSS 수집 명세의 수집 화면](../../requirements/collection.md#수집-화면)에 구독의 표지를 적고, 검증 표의 행을 더해요.
- [x] `cargo test --workspace`, `npm test`, `npm run typecheck`가 통과해요.

## 결과

### 만든 것

구독 화면과 규칙의 `Anissia 연결`이 구독을 만들면, 같은 트랜잭션에서 새 표 `subscription_artwork`(마이그레이션 69)에 그 구독의 표지 찾기를 요청해요. 가져오기는 요청하지 않아요. 작품 표지의 수신 큐가 이 일을 작품의 일보다 먼저 맡아, Anissia 원제로 AniList를 찾고(`search`) 고른 항목의 이미지를 받아요(`fetch`). 요청 간격, 재시도와 포기, 이미지 검사, 받은 파일을 앱 데이터 폴더에 두는 것은 작품 표지의 코드를 함께 써요.

후보는 `trss_anilist::title::decide_original`이 골라요. 원제는 작품 표지의 자동 판정과 같은 정규화로 AniList 항목의 원제(`native`)와 견주고, 시작일은 `starts_together`로 견줘요. 이를 위해 AniList 요청에 `startDate`를 더했어요.

이번 주 편성의 카드(`cover_url`)와 규칙의 구독(`subscription.cover_url`, 규칙 상세와 구독 목록)은 구독에 시즌이 없을 때 구독의 표지를 `GET /api/rules/{id}/cover`로 가리켜요. 이 경로는 작품 표지의 이미지와 같은 검사와 머리글로 답해요. 규칙 상세의 포스터는 시즌이 없을 때 이 표지를 보여줘요. 표지가 바뀌면 `rules` 바뀜으로 알려서, 이번 주 편성과 규칙 화면이 다시 읽어요.

구독이 시즌에 붙을 때(`link_season`) 같은 트랜잭션에서 구독의 표지를 끝내요. 그 시즌의 작품 표지가 `auto`이고 고른 것도 이미지도 없으면 이미지와 AniList 항목을 작품에 넘기고 선택 버전을 올려요. 그래서 그 작품의 자기 검색이 돌고 있었어도 그 결과는 버려져요. 넘기지 않은 표지와, 시즌에 붙기 전에 지워진 구독의 표지는 이미지 정리가 지워요. 정리는 `subscription_artwork`의 경로도 참조로 세요.

### 정한 것

- 시작일은 두 날짜가 모두 일까지 알면 7일 안이면 맞다고 봐요. 하나라도 일을 모르면 같은 해여야 하고, 둘 다 아는 달은 같아야 해요. 시작 연도가 없으면 아무 후보도 맞지 않아요. Anissia의 시작일은 한국 방영일이라 일본보다 며칠 늦을 수 있어서 같은 날을 요구하지 않았어요(구현 결정).
- 구독의 표지 일은 작품 표지의 일보다 먼저 돌아요. 사용자는 방금 구독하고 편성을 보는데, 감시 폴더를 처음 읽으면 작품의 일이 수백 개 쌓일 수 있어서예요(구현 결정).
- 작품에는 받은 이미지만 넘겨요. 구독이 항목을 골랐지만 이미지를 아직 받지 못했으면 넘기지 않고, 작품은 자기 검색으로 표지를 찾아요(구현 결정).
- 구독의 표지에는 선택 버전을 두지 않았어요. 사용자가 이 표지를 바꾸는 동작이 없으므로, 일의 결과는 그 행이 같은 일(`job`과 `job_requested_at`)을 아직 기다릴 때만 받아요(구현 결정).
- 원제가 없는 구독은 AniList에 묻지 않고 `no_match`로 끝내요(구현 결정).

### 테스트

2026-10-23에 355042ad 위의 작업 트리에서 아래 리뷰에서 고친 뒤 `cargo test --workspace`가 모두 통과했어요(3,238개 통과, 12개 무시, 테스트 바이너리 27개의 합 38.61초, `dev/measure/when-quiet.sh`로 다른 빌드가 없을 때 측정했어요). `cargo clippy --workspace --all-targets`에 경고가 없고, `npm test`(318개)와 `npm run typecheck`도 통과했어요. 원본 로그는 이 PC의 `dev/local/measure/feedback-0.8.0-0008/`에 있어요.

- `trss-anilist` `src/title.rs`의 `an_anissia_anime_takes_the_one_entry_of_its_native_title_that_starts_with_it`, `a_remake_of_the_same_title_is_told_by_its_start_and_a_doubt_leaves_no_cover`, `a_text_date_keeps_the_parts_it_can_read`는 원제와 시작일의 판정을 겨눠요. 로마자 제목만 같은 항목, 7일을 넘는 시작일, 같은 원제와 시작일의 두 항목, 읽다 만 검색은 고르지 않아요.
- `trss-library` `src/artwork/tests.rs`의 `a_subscription_takes_the_one_entry_of_its_original_title_that_starts_with_it`, `a_subscription_without_one_clear_match_of_its_whole_search_gets_no_cover`, `a_subscription_cover_that_anilist_keeps_failing_is_tried_three_times_and_left`는 가짜 AniList로 찾기와 받기를 겨눠요. 원제가 없거나, 리메이크만 있거나, 후보가 둘이거나, 페이지가 남았거나, 시작일이 없으면 표지가 없고, 실패는 세 번 다시 해 본 뒤 그만둬요.
- 같은 파일의 `a_subscription_hands_its_cover_only_to_a_work_whose_auto_cover_has_nothing_yet`는 `auto`이고 고른 것이 없는 작품만 표지를 받고, 이미 고른 표지와 `disabled`는 그대로인 것을 겨눠요. 받는 작품은 자기 검색이 도는 중이고, 그 검색의 결과는 버려져요. `a_subscription_deleted_before_its_season_takes_its_cover_file_along`은 지운 구독의 파일과, 받는 도중에 지워진 구독의 새 파일이 정리로 지워지는 것을 겨눠요. `a_work_of_an_unregistered_folder_takes_no_subscription_cover`는 등록한 폴더 밖의 작품이 표지를 받지 않는 것을 겨눠요.
- `trss-collect` `src/store/channels/season_anime_tests.rs` `a_subscription_asks_for_its_cover_and_hands_it_to_the_season_it_is_connected_to`는 구독 화면과 `Anissia 연결`의 저장이 찾기를 요청하고, 시즌 연결이 표지를 넘기며, 다른 작품이 잡은 시즌에는 넘기지 않는 것을 겨눠요. `import_subscriptions_tests.rs`의 가져오기 테스트는 요청이 없는 것을 겨눠요.
- `trss` `src/web/subscriptions_api/tests.rs` `rule_detail::a_subscription_shows_its_own_cover_until_its_season_has_the_work_s`는 AniList가 실패하는 동안 구독이 AniList에 묻지 않고 만들어지고, 시즌 전에는 규칙 상세·구독 목록·이번 주 편성이 구독의 표지를, 시즌 뒤에는 작품의 표지를 가리키며, 구독 표지의 경로가 이미지를 답하다가 시즌 뒤에는 `404`인 것을 겨눠요.

### 개발 환경 확인

2026-10-23에 서버 DB 사본으로 띄운 개발 환경을 이 작업 트리로 다시 만들어, 실제 Anissia와 AniList로 확인했어요. 원본 기록은 이 PC의 `dev/local/measure/feedback-0.8.0-0008/`에 있어요.

- 금요일 편성의 `반딧불이의 혼례`(Anissia 3417, 원제 `ホタルの嫁入り`, 시작일 2026-10-09)를 구독 API로 구독했어요. 응답은 8.5 ms에 왔고 `cover_url`은 비어 있었어요.
- 2초 안에 구독의 표지가 생겼어요. 고른 항목은 AniList 205909였고, AniList 페이지의 원제는 `ホタルの嫁入り`, 시작일은 2026-10-09였어요. 받은 이미지는 JPEG 460×645, 92,151바이트였고 파일은 `published`로 기록됐어요.
- 내장 브라우저에서 이번 주 편성의 금요일 카드와 그 규칙 상세의 포스터가 이 표지를 보였어요.

### 리뷰와 고친 것

`stora:complex-reviewer`가 커밋 전의 변경을 리뷰했어요. 일의 결과를 받는 조건과 넘기기의 경합, 지운 구독의 파일 정리, 마이그레이션, 바뀜 알림, 판정 기준, 새 테스트가 이름대로의 동작에 닿는지를 봤어요. 다음을 고쳤어요.

- `trss-core` `db.rs`의 마이그레이션 테스트 둘(`the_effect_records_go_and_what_an_earlier_release_left_under_way_ends`, `the_archived_rules_go_with_their_subscriptions_and_leave_the_rest`)이 새 표와 색인을 기대하지 않아 실패했어요. 두 테스트가 견주는 마이그레이션 뒤에 더해진 스키마 객체를 기대에 더하게 고쳤어요.
- 등록한 폴더 밖의 작품도 구독의 표지를 받을 수 있었어요. 작품 표지는 등록한 폴더의 작품만 찾으므로, 넘기기도 같은 조건을 보게 고치고 `a_work_of_an_unregistered_folder_takes_no_subscription_cover`로 확인했어요.
- 일을 미룰 때 시계를 두 번 읽었어요. 한 번 읽은 시각을 쓰게 고쳤어요.

나머지 지적은 아래 한계에 적었어요.

### 한계

- 다음 분기 `신작`이 아니라 이번 주에 방영하는 작품으로 확인했어요. `신작`은 이번 주 편성에 카드가 없어서, 카드와 포스터를 한 번의 AniList 검색으로 함께 보려고 바꿨어요. 그래서 완료 기준의 개발 환경 확인 상자는 비워 뒀어요.
- 실제 구독이 첫 화를 받아 시즌에 붙으며 표지를 넘기는 것은 보지 않았어요. 개발 환경의 Transmission은 더한 토렌트를 멈춰 두기 때문이에요. 서버에서는 [0011](0011-deploy-and-check-on-server.md)에서 봐요.
- iOS 홈 화면 앱에서 이번 주 편성의 표지가 보이는지는 [0011](0011-deploy-and-check-on-server.md)에서 봐요.
- Anissia와 AniList가 원제의 문자를 다르게 쓰면(예: `〜`와 `～`) 정규화가 같게 만들지 않으므로 표지를 두지 않아요. 이 경우는 실제로 보지 않았어요.
- 포스터와 카드의 화면 코드는 웹 테스트가 없어요.
- 구독의 이미지 파일이 넘기기 전에 없어지거나 망가져도 넘겨요. 작품은 자기 검색을 그만두므로, 사용자가 표지를 다시 골라야 해요. 리뷰에서 나왔어요.
- 넘긴 뒤 구독이 시즌을 다시 잃으면(보관 폴더로 옮기며 합쳐진 시즌) 구독의 표지가 없어서 카드가 빈 칸이에요. 이 변경 전과 같아요. 리뷰에서 나왔어요.
- 같은 Anissia 작품을 구독한 규칙이 둘이면 각자 AniList를 찾고 이미지를 받아요. 리뷰에서 나왔어요.
- 개발 환경에 `반딧불이의 혼례` 구독을 남겨 뒀어요.
