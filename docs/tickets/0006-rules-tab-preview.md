# 0006 수집 화면 규칙 탭에서 규칙을 고치고 미리 봐요

- 상태: 완료
- 출처: [채널과 다운로드 규칙](../specs/collection.md#채널과-다운로드-규칙), [수집 화면](../specs/collection.md#수집-화면), [웹 명령과 상태 갱신](../specs/web-app.md#웹-명령과-상태-갱신)
- 막는 티켓: [0001](0001-rule-evaluation.md), [0002](0002-app-state-db.md), [0003](0003-web-shell.md), [0004](0004-worker-collection-history.md)

## 작업

[0007](0007-channels-tab-secrets.md)이 세운 수집 화면의 탭 틀에 상태 판을 더하고, `규칙` 탭에서 모든 채널의 규칙을 한 목록으로 보고 규칙을 만들고 고치며, 저장 전에 그 규칙으로 무엇이 받아질지 미리 봐요.
미리보기는 0001의 판정을 그대로 써서, 수집 이력에 기록된 항목(worker가 마지막으로 읽은 RSS 항목 포함)을 편집 중인 규칙과 그 채널의 제외 조건·기본 경로·규칙 순서로 평가해요.
웹이 RSS를 따로 읽지 않으므로 미리보기와 실제 처리의 입력이 같아요.

- 목록은 채널을 먼저 고르지 않는 통합 목록이고, 줄마다 상태 배지·채널 태그와 필요할 때만 `겹침`을 둬요.
  정렬에 `검사 순서`가 있고, 표시 정렬을 바꿔도 실행 순서는 바뀌지 않아요.
- PC는 목록 옆 상세, 휴대폰은 상세 화면으로 전환해요.
  상세에서 일치 문구·정규식·대소문자·저장 폴더·회차 변환과 채널 안 순서를 고쳐요.
  바꾼 내용이 있을 때만 저장 줄을 붙이고, 보관·삭제는 상세 맨 아래 관리 줄에 둬요.
- 규칙 상세 맨 위의 포스터와 네 줄(방영·제작자·진행·마지막 수집)은 구독과 Anissia 연결이 생기는 결과 목표 2에서 채워요.
  이 티켓에서는 네 줄 중 알 수 있는 `마지막 수집`만 채우고 나머지는 `미정`·`아직 없음`으로 줄 수를 유지해요.
- 저장은 확인한 버전을 보내고, 충돌이면 입력을 남긴 채 서버 값과 비교해 다시 저장할 수 있어요.
- 상태 판은 RSS 채널 상태, 최근 7일 받은 개수와 날짜별 막대, Transmission 받는 중·시딩 수를 보여주고, 휴대폰에서는 한 줄로 접혀요.
  `구독` 탭은 결과 목표 2에서 채우고, 이 티켓에서는 빈 상태 문장만 둬요.

## 완료 기준

| 입력·상태 | 기대 결과 |
| --- | --- |
| 같은 수집 이력과 같은 설정으로 미리보기와 worker 처리를 비교 | 항목마다 선택 여부·적용 규칙·저장 위치가 같아요. 격리된 테스트로 고정해요. |
| 편집 중 일치 문구를 넓혀 다른 규칙이 먼저 가져가던 항목에도 맞게 함 | 미리보기에 그 항목이 앞 규칙으로 간다고 보이고, 저장 뒤 목록의 해당 규칙 줄에 `겹침`이 붙어요. |
| 정규식을 `(`로 끝나게 입력 | 미리보기에 규칙 오류를 문장으로 보여주고 저장하지 않아요. 다른 규칙의 미리보기는 계속 보여요. |
| 규칙 순서를 바꿔 저장 | 다음 worker 처리가 바뀐 순서로 첫 일치를 골라요. `검사 순서` 정렬이 새 순서예요. |
| 두 화면에서 같은 규칙을 열고 한쪽이 먼저 저장 | 다른 쪽 저장은 충돌로 거부되고 입력값이 남아요. |
| 목록 정렬을 제목순으로 바꿈 | 실행 순서와 저장된 규칙이 바뀌지 않아요. |
| 390px 휴대폰에서 규칙을 열어 고치고 저장 | 상세 화면으로 전환되고, 저장 줄이 하단 메뉴에 가리지 않아요. |

## 결과

### 구현한 것

- 규칙 API는 `crates/trss-web/src/rules_api.rs`예요. 모든 쓰기는 화면이 본 버전을 보내고, 버전이 다르면 `409`에 `current`(규칙, 순서 변경은 채널의 규칙 목록)를 담아요.

  | 호출 | 응답 |
  | --- | --- |
  | `GET /api/rules` | `{ rules: [규칙], channels: [채널 요약] }` |
  | `POST /api/rules` | `201` 규칙 (채널의 맨 뒤에 붙어요) |
  | `PUT /api/rules/{id}` | 규칙 (`version`, `channel_id` 필요, 채널은 옮길 수 없어요) |
  | `DELETE /api/rules/{id}?version=N` | `{ removed: true }` |
  | `PUT /api/rules/order` | `{ channel_id, order: [{ id, version }] }` → `{ rules }` |
  | `POST /api/rules/preview` | `{ error, counts, masked_total, items, truncated }` |

  규칙은 `id, channel_id, version, order(1부터), match, regex, case_insensitive, directory, episode, episode_auto, state, overlap, error, last_received_at`이에요.
  잘못된 정규식과 `/`로 시작하는 저장 폴더는 `400`으로 거절하고 아무것도 저장하지 않아요. 일치 문구를 비우면 `제목 대기` 규칙이 돼요.
- 규칙 삭제는 `ChannelStore::delete_rule(id, version)`이에요(`crates/trss-collect/src/store/channels/delete_rule.rs`). 다른 규칙의 순서는 그대로 두고, 수집 이력은 건드리지 않아요.
- 미리보기는 worker의 판정을 그대로 써요. `crates/trss-collect/src/plan.rs`의 `ChannelPlan`(저장된 채널·규칙을 `crates/trss-collect/src/rss`의 평가로 옮기는 worker의 변환)에 `evaluate`(판정 + 뒤에서 함께 맞는 규칙)와 `rule_errors`(규칙별 정규식 오류)를 더했고, `judge`는 `evaluate`를 거쳐요.
  미리보기는 편집 중인 규칙을 채널의 규칙 자리에 끼워(수집 중으로 취급, 순서 변경도 반영) 그 채널의 수집 이력 항목을 `evaluate`로 판정해요. 제외 조건을 뺀 두 번째 계획으로 제외 때문에 못 받는 항목을 가려내요. 웹이 제목을 따로 판정하는 코드는 없어요.
  `겹침`은 이력 항목 중 앞 규칙이 가져가는데 이 규칙도 맞는 항목이 있는 수집 중 규칙에 붙어요. 제외된 항목과 보관한 규칙은 세지 않아요.
- 상태 판은 `GET /api/collect/status?tz_offset=<분>`이에요(`crates/trss-web/src/status_api.rs`). 웹은 RSS도 Transmission도 부르지 않고, worker가 남긴 스냅숏과 수집 이력만 읽어요.
  - 마이그레이션 4(`crates/trss-core/migrations/status/schema.sql`): `channel_read_status`(채널별 마지막 읽기 성공 여부·시각·마지막 성공 시각)와 `transmission_snapshot`(받는 중·시딩 수와 시각, 한 행).
  - worker는 `crates/trss-worker/src/cycle.rs`에서 한 주기가 끝날 때 `StatusStore`로 기록해요. 기록·조회에 실패해도 출력만 하고 주기는 그대로 끝나요. Transmission에 물을 수 없으면 이전 수와 시각을 그대로 둬요(0을 쓰지 않아요). 읽기에 실패한 채널은 마지막 성공 시각을 지키고, 없어진 채널의 행은 지워요.
  - 날짜별 막대는 화면의 시간대(`tz_offset`) 기준 오늘까지 7일이에요. `실패·중복 N개`는 같은 7일의 추가 실패·중복 개수이고 `/collect/history?result=add_failed,duplicate`로 가요.
- 규칙 탭(`web/src/screens/collect/rules/`)은 통합 목록(상태 배지·채널 태그·`겹침`, `검사 순서`·`제목순` 정렬)과 상세예요. 정렬은 화면에서만 바뀌고 저장된 순서를 건드리지 않아요.
  PC는 목록과 상세가 각자 스크롤하고 규칙을 골라도 화면이 움직이지 않아요(선택은 `?rule=<id>`로 주소 경로를 바꾸지 않아요). 휴대폰은 상세 화면으로 전환하고 `← 규칙 목록`으로 돌아와요.
  상세는 맨 위에 포스터 자리와 네 줄(방영·제작자 `미정`, 진행 `아직 없음`, 마지막 수집만 실제 값), 일치 문구·정규식·대소문자·저장 폴더·회차 변환, 채널 안 순서(`앞으로`·`뒤로`), 미리보기, 바꾼 내용이 있을 때만 붙는 저장 줄, 맨 아래 관리 줄(`보관`·`복원`·`삭제`)이에요. 필드 저장과 순서 저장은 두 번의 호출이에요.
  저장할 수 없는 정규식이면 저장 줄이 저장을 막고 문장을 보여줘요. 저장하지 않은 변경이 있는 채 다른 규칙으로 옮기면 확인을 물어요.
- 0008이 쓸 주소: `/collect/rules/new?channel=<채널 id>&match=<URL 인코딩한 제목>`은 채널과 일치 문구가 채워진 새 규칙 상세를 열고, 저장하기 전에는 아무것도 만들지 않아요. 이를 위해 `CollectScreen`의 탭 경로를 `<탭>/*`로 넓혔어요.

### 검증한 것

| 완료 기준 | 근거 |
| --- | --- |
| 미리보기와 worker 처리의 일치 | 통합 테스트 `crates/trss-worker/tests/rules_preview_then_cycle.rs` `the_preview_of_every_rule_agrees_with_what_the_worker_did_with_the_same_items`: fake Transmission·피드 서버로 한 주기를 돌린 뒤 규칙마다 미리보기를 받아, 기록된 항목 7개 각각의 선택 여부·적용 규칙·저장 위치(Transmission이 받은 `download-dir`)가 같아요. 단위 테스트 `the_preview_agrees_with_the_worker_mapping_for_every_recorded_title`은 제목 표본 전체를 `ChannelPlan::evaluate`와 대조해요. |
| 일치 문구를 넓혀 다른 규칙이 먼저 가져가던 항목에도 맞게 함 | API 테스트 `widening_a_phrase_shows_the_items_an_earlier_rule_takes_and_saving_marks_the_overlap`. 통합 테스트 `what_the_preview_predicts_for_an_edit_is_what_the_next_cycle_does_after_it_is_saved`는 미리보기가 예측한 새 항목이 저장 뒤 다음 주기에 예측한 폴더·규칙으로 추가되는지 봐요. 브라우저: 미리보기에 `앞 규칙이 가져가요`가 뜨고 저장 뒤 목록에 `겹침`이 붙어요. |
| 정규식이 `(`로 끝남 | 브라우저: `정규식이 올바르지 않아요…` 문장이 뜨고 저장 버튼이 꺼지며 DB는 그대로예요. 다른 규칙을 열면 미리보기가 계속 나와요. API 테스트 `an_invalid_regex_is_refused_with_a_sentence_and_nothing_is_saved`, `an_invalid_regex_is_shown_in_the_preview_and_other_rules_still_preview`. |
| 규칙 순서를 바꿔 저장 | 통합 테스트 `after_a_reorder_through_the_api_the_next_cycle_picks_the_first_match_in_the_new_order`(두 규칙이 맞는 항목을 다음 주기가 바뀐 순서의 첫 규칙으로 받음). 브라우저: `앞으로` 뒤 저장하면 API와 `검사 순서` 목록이 새 순서예요. |
| 두 화면에서 같은 규칙을 열고 한쪽이 먼저 저장 | 브라우저(컨텍스트 두 개): 나중 저장은 `409`, `다른 곳에서 먼저 저장했어요.`와 저장된 값·내 입력이 함께 보이고 입력이 남아요. 다시 저장하면 그 버전으로 저장돼요. API 테스트 `the_second_save_from_an_old_version_is_a_conflict_that_shows_the_saved_rule`, `a_stale_order_is_a_conflict_that_carries_the_current_rules`. |
| 목록 정렬을 제목순으로 바꿈 | 브라우저: 목록만 재배열되고, 전후로 `rules` 표(id·position·version·match_text·state)와 API의 `order`가 같아요. |
| 390px 휴대폰에서 규칙을 열어 고치고 저장 | 브라우저(390x844): 목록이 상세로 바뀌고, 저장 줄의 아래쪽이 하단 메뉴 위쪽보다 위에 있으며(779px 대 784.8px), 저장 버튼 중심을 `elementFromPoint`가 저장 버튼으로 돌려줘요. 끝까지 스크롤해도 같아요. 저장하면 DB에 반영돼요. |
| 가로 넘침 없음 | 브라우저: 1440·768·390·320px x 밝은·어두운 화면 x (목록, 규칙 상세, 새 규칙 화면; 휴대폰 폭에서는 상태 판을 연 채로) 24곳 모두 `scrollWidth <= innerWidth`예요. |
| 상태 판 | 브라우저: RSS `2개 중 1개 정상`과 읽지 못한 채널 이름, 7개 막대, `받는 중 2개 · 시딩 5개`, `실패·중복 2개` 링크의 주소. 0008과 합친 뒤에는 이 링크가 기록 탭을 `추가 실패`·`중복`만 보이게 열어요. 휴대폰은 한 줄로 접혀 있다가 눌러서 펴져요. 통합 테스트 `crates/trss-worker/tests/status_snapshots_from_cycle.rs`: 한 주기 뒤 스냅숏이 채워지고, 피드 실패는 마지막 성공 시각을 지키며, Transmission이 꺼져 있어도 주기가 끝나고 이전 수가 남아요. |
| `/collect/rules/new?channel=&match=` | 브라우저: 채널과 일치 문구가 채워진 상세가 열리고 열기만 해서는 규칙 수가 그대로이며, `규칙 만들기`로 저장하면 만들어져 열려요. |

- 백엔드 검사: `cargo test --offline`(라이브러리 200개와 통합 테스트 전부), `cargo clippy --offline --all-targets`(경고 없음), `cargo fmt --check`. 새 테스트는 저장소 `delete_rule` 4개, 상태 저장소 5개, `ChannelPlan::evaluate` 1개, 규칙 API 22개, 상태 API 6개, 통합 6개예요.
- 프런트 검사: `cd web && bun run build`(`tsc -b` 포함).
- 브라우저 검사는 임시 DB(`TRSS_DB_PATH`)와 `web/dist`를 붙인 로컬 `trss-web`에 headless Chromium(playwright-core)으로 했어요. 스크립트는 저장소 밖에 두었고 끝난 뒤 서버와 브라우저를 종료했어요. 시드 채널 주소의 토큰은 가짜 값이에요.

### 남은 것과 한계

- 수집 이력의 제목은 채널 주소의 긴 비밀 값(8자 이상)이 `***`로 가려진 채 저장돼요. 미리보기는 저장된 제목으로 판정하므로, 제목에 그 값이 들어 있던 항목은 원문을 본 worker와 다르게 판정될 수 있어요. 그런 항목은 `가려진 제목`으로 표시하고 미리보기 아래에 개수와 함께 안내해요. 이력 저장 방식을 바꾸지 않고는 없앨 수 없어요.
- 미리보기와 `겹침`은 채널당 최근 20,000개 이력 항목까지만 봐요(`MAX_ITEMS_PER_CHANNEL`). 미리보기의 항목 목록은 100개까지이고 개수는 전체를 세요.
- 규칙 저장과 순서 저장은 두 번의 호출이라 원자적이지 않아요. 필드 저장이 성공하고 순서 저장이 충돌하면 필드는 저장된 채로 순서 충돌 안내가 떠요.
- `실패·중복 N개`는 최근 7일의 개수인데 링크가 여는 기록 필터에는 기간이 없어요. Transmission 수의 `받는 중`·`시딩`은 대기열에 있는 항목을 같은 종류로 세요.
- worker의 이름 바꾸기(`rename_torrent`)는 `trname`이 이름을 만들지 못하면 토렌트를 데이터와 함께 지워요. 순서 통합 테스트에서 `overlap` 폴더 규칙(`episode` 기본값 1)의 토렌트가 이렇게 지워지는 것을 봤어요. 기존 동작이라 이 티켓에서 바꾸지 않았고, 원인은 `trname`의 규칙이라 따로 확인하지 않았어요. 그래서 그 테스트는 Transmission의 토렌트 목록 대신 `torrent-add` 호출로 저장 위치를 확인해요.
- 0008과 합치며 상태 스냅숏은 마이그레이션 4, 0008의 명령은 5로 두었고 `AppState`에 `status`와 `commands`가 함께 있어요. `ChannelStore::db()`는 worker가 스냅숏을 같은 DB에 쓰려고 더한 읽기 접근자예요.
- 포스터, 방영·제작자·진행 줄, `편성표와 연결`, 보관 제안 배너는 구독·Anissia 연결이 생기는 결과 목표 2에서 채워요. `구독` 탭은 빈 상태 문장만 있어요.
