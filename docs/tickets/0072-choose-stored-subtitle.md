# 0072 고른 보관본을 적용하고 작품별 자막 형식 순서를 정해요

- 상태: 완료 (2026-10-06)
- 출처: [보관본과 적용본](../specs/subtitles.md#보관본과-적용본)의 선택·추가 적용·지난 수정본 복원, [오른쪽 카드 열](../specs/library.md#오른쪽-카드-열)의 `자막`, [공통 정책](../specs/settings.md#공통-정책)의 작품별 재정의(목표 3의 [0049](../archive/tickets/3-subtitle-candidates-and-receiving/0049-common-policy-settings.md)가 남긴 편집 자리)
- 막는 티켓: [0068](0068-replacement-approval.md)

## 작업

- 작품 상세의 `자막` 카드는 받은 자막을 제작자별로 묶고, 영상 옆에 둔 사본을 `적용`이라고 불러요. 지난 수정본은 `9월 7일 받음`처럼 받은 날짜로 부르고, 적용 위치와 보관 위치를 구분해 보여줘요.
- 다른 제작자·형식·지난 수정본을 골라요. 그 회차에 적용본이 있으면 교체 비교를 바로 열고 `새 자막으로 교체`로 반영해요(사용자 결정, 2026-10-04). 없으면 바로 적용해요.
  보관만 한 회차의 `적용`([0064](0064-multi-file-packages.md))도 자막이 있는 회차면 이 비교로 이어요.
- 같은 제작자의 다른 형식은 추가로 적용할 수 있고, 추가 적용은 기존 적용본을 지우지 않아요.
- 작품별 형식 순서는 `자막` 카드에서 전역 순서를 재정의하거나 되돌려요(`work_subtitle_policy`). 그 뒤의 첫 적용과 형식 선택은 작품의 순서를 써요.
  명세에 편집 자리가 없으므로 [오른쪽 카드 열](../specs/library.md#오른쪽-카드-열)의 `자막`에 적어요.

## 완료 기준

| 입력·상태 | 기대 결과 |
| --- | --- |
| 적용본이 있는 회차에서 다른 제작자의 보관본을 고름 | 교체 비교가 열리고, `새 자막으로 교체`로 바뀌어요. |
| 적용본이 없는 회차에서 보관본을 고름 | 바로 적용돼요. |
| 지난 수정본(`9월 7일 받음`)을 고름 | 비교 뒤 그 보관본이 다시 복사되고, 지금의 보관본도 남아요. |
| ASS 적용본이 있는 회차에 같은 제작자의 SRT를 추가 적용 | SRT가 더해지고 ASS는 그대로예요. |
| 작품 순서를 SRT → ASS → SMI로 정함 | 그 작품의 다음 첫 적용이 SRT를 고르고, 설정의 재정의 목록에 그 작품이 있어요. 다른 작품은 전역 순서를 써요. |
| 작품 순서를 되돌림 | 그 작품이 전역 순서를 쓰고 재정의 목록에서 빠져요. |

## 결과

### 만든 것 (2026-10-06)

- **고르기**(`place::records::choose_stored`, `stored_options`): 고른 보관본은 0064의 회차 줄 `적용`처럼 그 보관본을 받은 작업의 줄을 다시 적용으로 돌려 그 작업이 맡아요(사용자 결정, 2026-10-05). 고를 때마다 새 작업을 만드는 길은 고르지 않았어요. 줄에는 사람이 골랐다는 표시(`chosen`: `apply`·`add`)가 남아요.
  - `적용`은 회차에 자막이 없으면 바로 적용하고, 있으면 작업이 교체 계획(내용 비교 포함)을 만들어 `교체 승인`을 기다려요. 이 경우 응답의 `compare`가 참이고 화면은 작업 상세로 가요. 0064가 이 경우에 내던 거절은 없앴어요.
  - 사람이 고른 줄은 같은 출처의 더 새 수정본이 보관돼 있어도 보관만 하지 않고, 반영 직전 검사도 `새 수정본 발견`으로 닫지 않아요. 지난 수정본은 이 길로 되돌려요.
  - `추가 적용`은 회차에 적용본이 있는 제작자의 다른 형식만 받아요. 제작자를 모르는 보관본과 이미 적용한 형식은 거절해요. 어느 적용본도 지우지 않고(다른 경로는 모두 `유지`), 새 적용본이 갈 경로가 비어 있으면 승인 없이 바로 적용해요. 그 자리에 파일이 있으면(대소문자만 다른 이름도) 그 파일만 `교체`로 둔 계획을 만들어요. 라이브러리가 그 파일을 기록했으면 응답의 `compare`가 참이라 화면이 작업 상세로 가요.
- **마이그레이션 58**(`chosen_rows.sql`): `subtitle_job_plan.chosen`을 더해요. 작업이 스스로 정한 줄과 그 전의 줄은 비어 있어요.
- **API**: 적용 요청이 `{mode}`를 받고 `{job_id, compare}`를 돌려줘요. 작품 상세에 `자막` 카드의 `subtitles`(형식 순서와 `own`, 제작자별 보관본의 `applied`·`choice`·`can_add`·`blocked`)를 더했어요. 카드와 요청은 같은 판정(`stored_options`)을 써요. `PUT`·`DELETE /api/library/works/{id}/subtitle-order`가 작품의 형식 순서를 정하고 되돌려요. 회차 줄의 `stored`에 `compare`를 더했어요.
- **웹**: 작품 상세 오른쪽 카드 열의 `수집`과 `파일` 사이에 `자막` 카드(`SubtitlesCard`, `subtitles.ts`)를 뒀어요. 고른 시즌의 보관본을 제작자별로 보여주고, 영상 옆 보관본에 `적용`과 `적용 위치`·`보관 위치`를 따로 적어요. 받은 날짜는 `9월 7일 받음`이고 같은 날 또 받았으면 시각을 붙여요. `적용`·`교체 비교`·`추가 적용`과 형식 순서 편집기(`위로`·`아래로`, `저장`, `전역 순서로 되돌리기`)가 있어요. 화면 낭독기는 각 버튼을 회차·형식·받은 날짜로 불러요(`2화 ASS 10월 5일 01:36 받음 교체 비교`). 회차 줄의 버튼은 자막이 있는 회차에서 `교체 비교`라고 적혀요.
- **명세**: [보관본과 적용본](../specs/subtitles.md#보관본과-적용본), [고르지 않은 회차](../specs/subtitles.md#고르지-않은-회차), [교체 계획과 반영](../specs/subtitles.md#교체-계획과-반영), [오른쪽 카드 열](../specs/library.md#오른쪽-카드-열), [공통 정책](../specs/settings.md#공통-정책)을 고쳤어요.

### 검증한 것

| 완료 기준 | 근거 |
| --- | --- |
| 적용본이 있는 회차에서 다른 제작자의 보관본을 고름 | 작업 시험 `another_creators_copy_on_an_episode_with_a_subtitle_is_compared_then_replaces_it`에서 고르면 작업이 다시 줄에 서고, 실행하면 비교가 있는 열린 계획과 `교체 승인`이 생겨요. 승인하면 고른 바이트가 영상 옆에 오고 바뀐 자막의 보관본은 남아요. API 시험 `an_episode_lists_its_stored_only_subtitles_and_one_is_applied_on_request`는 그때 `202`와 `compare: true`를 확인해요. 개발 환경(2026-10-06)에서는 `FX Senshi Kurumi-chan` 2화의 `가짜 출처` `dev-2v3.ass`에서 `교체 비교`를 누르자 작업 `f21a0fad` 상세로 갔어요. `새 자막으로 교체` 뒤 영상 옆 파일이 그 보관본의 SHA-256(`9a5152…`)이 됐고, 전에 적용한 `a02.ass`는 적용 기록이 끝나고 보관본으로 남았어요. |
| 적용본이 없는 회차에서 보관본을 고름 | `a_copy_on_an_episode_with_no_subtitle_is_applied_at_once_and_marked_chosen`에서 바로 적용되고 줄에 `chosen`이 남아요. |
| 지난 수정본을 고름 | `a_past_revision_is_restored_over_the_newer_one_which_stays_stored`에서 v2가 적용된 회차에 v1을 고르면, 더 새 수정본이 있어도 계획이 생기고 반영 직전 검사가 닫지 않아요. 승인하면 v1이 영상 옆에 오고 v2의 보관본은 남아요. 아무도 고르지 않은 줄은 전처럼 보관만 해요(`a_newer_revision_still_keeps_a_row_nobody_chose_stored_only`). |
| ASS 적용본이 있는 회차에 같은 제작자의 SRT를 추가 적용 | `the_creators_other_format_is_added_beside_the_applied_copy_with_no_approval`에서 SRT가 승인 없이 더해지고 ASS는 바이트도 적용 기록도 그대로예요. 감시기가 적용본과 다른 이름의 자막을 기록해 둔 회차에서도 SRT만 바로 더해지고 그 자막은 그대로예요(`an_added_format_counts_only_its_own_name_among_the_subtitles_the_library_recorded`). SRT 자리에 관리하지 않는 파일이 있고 라이브러리가 기록했으면 응답의 `compare`가 참이고, ASS를 `유지`로 둔 계획을 만들어요(`an_added_format_whose_path_holds_a_file_is_planned_with_the_applied_copy_kept`). 대소문자만 다른 이름도 그 자리로 봐요(`an_added_formats_name_is_held_in_any_case`). 다른 제작자, 이미 적용한 형식, 제작자를 모르는 보관본은 거절해요(`an_add_is_for_the_creators_other_format_and_the_other_refusals_stand`, `a_copy_with_no_creator_is_never_added`). 개발 환경(2026-10-06)에서는 3화의 `Kurumi-03.srt`를 `추가 적용`하자 승인 없이 SRT(`0bab43…`)가 생겼고, ASS(`5dbc40…`)는 바이트도 적용 기록도 그대로였어요. |
| 작품 순서를 SRT → ASS → SMI로 정함 | `a_works_own_order_decides_its_first_apply_and_going_back_restores_the_common_one`에서 ASS와 SRT가 함께 든 묶음의 첫 적용이 SRT를 고르고, 다른 작품은 전역 순서를 써요. API 시험 `a_works_own_format_order_is_set_listed_in_the_policy_and_taken_away`는 그 작품이 설정의 재정의 목록에 오는 것을 확인해요. 개발 환경(2026-10-06)에서는 편집기로 SRT를 위로 올려 저장하자 카드가 `이 작품의 순서`를 보여주고, 설정의 `작품별 재정의`에 이 작품이 나왔어요. |
| 작품 순서를 되돌림 | 같은 두 시험에서 되돌리면 전역 순서로 첫 적용을 하고 재정의 목록에서 빠져요. 같은 형식을 두 번 넣은 순서는 `400`이고 아무것도 바뀌지 않아요. 개발 환경에서 `전역 순서로 되돌리기`를 누르자 카드가 `전역 순서`로 돌아오고 재정의 목록이 비었어요. |

그 밖의 시험도 있어요.

- 마이그레이션 58의 CHECK와 그 전 줄(`migration_58_marks_a_chosen_row_apply_or_add_and_reads_older_rows_as_none`), 저장소의 순서 쓰기·지우기(`a_work_is_given_its_own_order_replaced_and_taken_away`), API의 `mode` 검사(`a_stored_subtitle_is_chosen_with_a_mode_and_a_bad_one_changes_nothing`), `자막` 카드의 묶음·순서·판정(`the_subtitles_card_groups_stored_copies_by_creator_with_what_choosing_each_does`), 추가 적용 요청과 그 자리에 기록된 파일이 있을 때의 `compare`(`the_creators_other_format_is_added_beside_the_applied_copy_on_request`)를 확인해요. 작품 순서 시험은 저장 전에 앞선 순서의 시각을 되돌려, 같은 밀리초의 두 저장에도 순서가 흔들리지 않아요.
- 웹 시험 17개(`subtitles.test.ts`)가 형식 글자, 순서 문장, 시즌 묶음, 받은 날짜의 시각, 적용·보관 위치, 보이는 버튼과 그 이름, 편집기의 이동과 `저장`을 확인해요.
- 고친 판정을 하나씩 되돌리면 해당 시험이 실패해요(`probe-out/0072-mutations.txt`, 검토 전 22개와 검토를 반영한 판정 5개). 처음에 잡히지 않은 두 경우는 시험을 더하거나 망가뜨리는 방법을 고쳐 다시 확인했어요.

### 독립 검토 (2026-10-06)

`stora:reviewer`가 코드를 읽고 따라가며 검토했어요. 그때 cargo는 돌리지 않았어요. 마이그레이션 58과 `chosen`을 쓰는 모든 곳, `추가 적용`이 다른 경로를 건드리지 않는 것, 새 수정본 우회의 두 자리, `stored_options`와 쓰기가 한 트랜잭션인 것, API와 카드의 판정에서 문제를 찾지 못했어요. 지적은 이렇게 처리했어요.

- `추가 적용`이 교체 계획으로 이어질 때도 응답이 `compare: false`라 화면이 작업 상세로 가지 않았어요. 새 적용본이 갈 이름에 라이브러리가 기록한 자막이 있으면 `compare`가 참이 되도록 고쳤어요.
- 감시기가 기록한 자막 줄(`media_files`의 `subtitle`)을 쓴 시험이 없었어요. 그래서 `existing_subtitle`의 기록 쪽 이름 거르기를 빼도 시험이 통과했어요. 기록된 자막 곁의 추가 적용 시험과 대소문자만 다른 이름의 시험을 더했어요.
- 작품 순서 시험이 같은 밀리초에 두 번 저장되면 흔들릴 수 있었어요. 저장 전에 앞선 순서의 시각을 되돌려 고정했어요.
- 작품 상세가 보관본마다 그 시즌의 파일 기록을 다시 읽었어요. 이제 한 요청에서 시즌마다 한 번 읽어요.
- 검사와 반영 사이에 새 적용본 자리에 파일이 생기는 경우, 앱의 적용본 기록만으로 자막이 있다고 보는 경우는 시험을 더하지 않았어요. 검토자가 코드를 따라가 맞다고 본 경로예요.

### 시험

검토를 반영한 작업 트리(2026-10-06, `233d8f2` 위)에서 `cargo test --workspace -j 4`를 돌렸어요. 시험 바이너리 44개에서 2,669개가 통과했고 실패는 0, 무시는 13개였어요(`probe-out/0072-workspace-test.txt`). `cargo clippy --workspace --all-targets -j 4`는 경고가 없었어요. 컴파일 캐시(kache)가 결과를 돌려준 것인지 가리려고, 캐시를 끄고 따로 둔 target 폴더에서 `trss-core`·`trss-jobs`·`trss-web`을 다시 검사했어요. 이때도 경고가 없었어요(`probe-out/0072-clippy-nocache.txt`). `cargo fmt --all --check`도 통과해요. 웹은 `bun test` 182개가 통과하고 `tsc -b`가 통과해요.

개발 환경(2026-10-06, 이 변경을 담은 이미지)에서 `자막` 카드의 세 동작과 편집기를 확인했어요. 결과는 [검증한 것](#검증한-것)에 적었고, 전후 상태는 `probe-out/0072-e2e-before.txt`와 `probe-out/0072-e2e-after.txt`에 있어요. 이때 같은 회차·형식의 보관본 버튼들이 화면 낭독기에서 모두 `2화 ASS 교체 비교`로 불리는 것을 찾았어요. 받은 날짜를 이름에 넣어 고치고, 다시 띄운 화면에서 이름이 서로 다른 것을 확인했어요.

0072의 시험 수는 이래요.

- 작업 시험(`tests/it/choose.rs`) 12개
- 저장소 시험(`policy/tests.rs`) 1개
- API 시험(`library_work_api/tests.rs`) 4개. 0064의 회차 줄 시험은 `202`와 `compare: true`를 기대하도록 고쳤어요.
- 웹 시험(`subtitles.test.ts`) 17개

### 한계

- 받은 작업이 없는 보관본은 고를 수 없어요. 교체하면서 들인 `제작자 알 수 없음` 보관본(교체 전에 영상 옆에 있던 파일)이 그래요. 고를 때마다 새 작업을 만드는 길을 고르지 않은 대가예요.
- 고른 보관본을 맡을 작업이 모두 진행 중·보류·승인 대기이면 고를 수 없어요(0064와 같아요).
- 작품의 형식 순서에는 판 검사가 없어요. 두 화면에서 동시에 바꾸면 나중 저장이 남아요.
- 응답의 `compare`는 라이브러리 기록과 앱의 적용본으로 정해요. 감시기가 아직 기록하지 않은 파일이 있으면 작업은 교체 계획을 만들지만, 화면은 작업 상세로 가지 않고 `작업 보기` 링크만 보여줘요.
