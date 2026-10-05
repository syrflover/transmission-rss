# 0069 교체 비교에 대사·타이밍·스타일·폰트의 차이를 보여줘요

- 상태: 완료 (2026-10-05)
- 출처: [교체 비교와 승인](../specs/subtitles.md#교체-비교와-승인), [할 일 화면](../specs/jobs.md#할-일-화면)의 교체 승인 카드
- 막는 티켓: [0068](0068-replacement-approval.md)

## 작업

- ASS·SRT·SMI를 읽어 대사(추가·삭제·변경), 타이밍, 스타일, 폰트의 차이를 만들어요.
- 변경 사항은 처음에 접어 `추가 2`·`변경 12`·`삭제 0` 같은 숫자 태그만 보이고, 항목마다 펼쳐 상세를 봐요.
  대사 비교의 두 줄은 `−`·`+`와 취소선·밑줄로 나누고, 새로 추가된 대사의 이전 줄은 취소선과 같은 회색·굵기의 가로 막대예요.
- 이미지 자막, 읽지 못한 형식, 인코딩을 알 수 없는 SMI는 `비교 불가`로 보여주고 차이 없음으로 보이지 않아요.
- 할 일 화면의 교체 승인 카드 사유 줄은 `대사 추가 2`·`대사 변경 12`·`타이밍 3` 같은 숫자 태그예요.
- SMI는 인코딩(CP949·UTF-8·UTF-16)과 구조가 제각각이에요. 개발 환경의 실제 SMI(Naver, WinPNG)로 읽기를 확인하되, 실제 표본은 공개 저장소에 올리지 않고 시험에는 만든 표본을 써요.

## 완료 기준

| 입력·상태 | 기대 결과 |
| --- | --- |
| 대사 2줄을 더하고 12줄을 고친 ASS | 태그가 `추가 2`·`변경 12`·`삭제 0`이고, 펼치면 그 줄들이 보여요. |
| 타이밍만 옮긴 SRT | 대사 태그는 0이고 타이밍 태그에 바뀐 줄 수가 있어요. |
| ASS 스타일의 글꼴만 바뀜 | 스타일과 폰트 항목에 그 변화가 있어요. |
| 내용이 같은 CP949 SMI와 UTF-8 SMI | 대사 차이가 없어요. |
| 읽을 수 없는 SMI, PGS | `비교 불가`로 보이고 차이 없음으로 보이지 않아요. |
| 개발 환경의 실제 SMI(Naver·WinPNG) | 읽혀서 대사 수가 나오거나, 읽지 못하면 까닭이 나와요. 결과가 날짜와 함께 이 티켓에 있어요. |
| 할 일 화면의 교체 승인 카드 | 사유 줄이 숫자 태그예요. |

## 결과

### 만든 것 (2026-10-05)

- **비교 엔진**(`crates/trss-subtitles/src/compare/`): ASS·SSA, SRT, WebVTT, SMI·SAMI를 읽어 대사(추가·변경·삭제), 타이밍, 스타일, 폰트의 차이를 만들어요. 인코딩은 BOM, BOM 없이 올바른 UTF-8, 오류 없이 읽히는 CP949 순서로 정하고, 어느 것도 아니면 읽지 않아요. 대사는 서식 태그를 뺀 글자가 같은 줄을 시간 순서로 가장 길게 맞추고(Hirschberg, 메모리는 줄 수에 비례), 맞지 않은 줄 가운데 시간이 겹치는 줄을 `변경`으로 세요. SMI는 언어 구분(Class)끼리 견주고, 견주지 못한 구분은 `일부 비교 불가`와 까닭으로 알려요. 인터넷에서 받은 파일이라 읽기는 파일 길이에 비례하고, 대사·SMI 문단·스타일·줄 길이·폰트·언어 구분·형식 항목·이름 길이에 상한을 둬요([내용 비교](../specs/subtitles.md#내용-비교)).
- **계획에 남기는 비교**(`crates/trss-jobs/src/place/replace/diff.rs`, `records.rs`): worker가 계획을 만들 때 `현재` 자막(`Plan::current`)과 새 자막을 한 번 견줘 계획과 같은 트랜잭션에 남겨요. `현재`는 교체할 파일이고, 없으면 처음 제거할 이전 적용본, 그것도 없으면 처음 유지할 파일이에요. 8 MiB보다 큰 파일, 계획이 기록한 SHA-256과 다른 현재 자막, 일반 파일이 아닌 현재 자막은 견주지 않고 어느 쪽인지 까닭에 밝혀요. 엔진이 패닉해도 계획은 만들고 그 비교만 `비교 불가`예요.
- **마이그레이션 57**(`replacement_diffs.sql`): 계획마다 요약·줄·읽지 못한 까닭 가운데 맞는 것만 담게 하는 `subtitle_replacement_diffs` 표와, 변경을 거절하는 트리거를 만들어요. 그 전에 만든 계획에는 비교가 없고 나중에 채우지 않아요.
- **API**: 작업 상세의 계획에 `comparison`(`null`, 까닭이 있는 `unreadable`, 줄을 뺀 요약인 `compared`)을 더했어요. 대사·타이밍의 줄은 `GET /api/subtitle-jobs/{id}/replacements/{plan}/lines`로 따로 받아요. 그 작업의 계획이 아니거나 견주지 않은 계획이면 `404`예요. 할 일의 교체 승인 카드에 열린 계획들의 합(`changes`: 대사 추가·변경·삭제, 타이밍, 스타일, 폰트, `uncompared`, `partial`, `plans`)을 더했어요.
- **웹**: 결정 카드 아래 `변경 사항`(`Changes.tsx`, `changes.ts`)은 네 항목을 접어 숫자 태그만 보여줘요. 펼치면 대사·타이밍의 줄(처음 펼칠 때 받고 200줄씩 보여요)과 스타일·폰트의 이름, 바뀐 필드가 나와요. 대사의 두 줄은 `−`·`+`와 취소선·밑줄로 나누고, 추가된 대사의 이전 줄은 취소선과 같은 회색·굵기의 막대예요. 할 일 카드의 사유 줄은 숫자 태그예요(`todoTags`). 비교한 계획의 버전 줄은 비교가 읽은 대사 수를 보여줘요(`shownVersions`).
- **명세**: [내용 비교](../specs/subtitles.md#내용-비교)를 새로 쓰고, [교체 계획과 반영](../specs/subtitles.md#교체-계획과-반영)의 줄 수 한계와 [할 일 화면](../specs/jobs.md#할-일-화면)의 카드 문장을 고쳤어요.

### 검증한 것

| 완료 기준 | 근거 |
| --- | --- |
| 대사 2줄을 더하고 12줄을 고친 ASS | 엔진 시험 `an_ass_with_two_lines_added_and_twelve_changed`가 2·12·0과 그 줄들을 확인해요. 작업 시험 `an_ass_with_two_lines_added_and_twelve_changed_is_stored_with_its_counts_and_lines`는 그 요약과 줄이 계획과 함께 남는 것을 확인해요. API 시험 `the_detail_says_what_changed_without_the_lines`와 `the_lines_of_a_compared_plan_come_from_their_own_route`는 상세에 줄이 빠지고 줄은 따로 오는 것을 확인해요. 웹 시험(`changes.test.ts`)은 접힌 항목의 태그와 펼친 줄의 모양을 확인해요. 개발 환경(2026-10-05)에서 1화에 손으로 고친 ASS를 올리자(작업 `a4245570`) 추가 2·변경 3·삭제 1, 타이밍 2, 스타일 추가 `Sign`·`Default.Fontname` Arial→Noto Sans CJK KR, 폰트 추가 3·삭제 Arial이 보였어요. 고친 내용과 같아요. |
| 타이밍만 옮긴 SRT | `an_srt_with_only_its_timings_shifted`와 `an_srt_with_only_its_timing_moved_has_no_dialogue_change_and_counts_its_moved_lines`에서 대사는 0이고 타이밍에 옮긴 줄 수가 있어요. |
| ASS 스타일의 글꼴만 바뀜 | `a_style_whose_font_name_alone_changed`와 `an_ass_whose_styles_font_only_changed_has_a_style_change_and_fonts_added_and_removed`에서 스타일 변경(`Fontname`의 이전·새 값)과 폰트 추가·삭제가 있어요. |
| 내용이 같은 CP949 SMI와 UTF-8 SMI | `the_same_smi_in_cp949_and_in_utf8`와 `a_cp949_smi_and_a_utf8_smi_of_the_same_content_do_not_differ`에서 대사 차이가 없어요. |
| 읽을 수 없는 SMI, PGS | 엔진 시험 `an_smi_in_no_known_encoding_is_unreadable`, `pictures_and_unknown_formats_are_unreadable`, 작업 시험 `a_current_smi_whose_encoding_is_unknown_is_not_compared_and_says_which_side`, `a_picture_subtitle_beside_the_video_is_the_current_one_and_is_not_compared`가 까닭과 함께 `비교 불가`인 것을 확인해요. API 시험 `a_plan_with_no_comparison_or_an_unreadable_one_says_so_and_never_no_difference`와 웹 시험은 그런 계획이 `차이 없음`으로 보이지 않는 것을 확인해요. 개발 환경에서는 마이그레이션 57 전에 만든 계획 9개가 `비교 불가`로 보였어요. |
| 개발 환경의 실제 SMI(Naver·WinPNG) | 2026-10-05에 개발 PC의 실제 SMI 4개를 읽었어요. Naver(수퍼소닉EX)의 UTF-16 두 벌은 479줄, Tistory(하느)의 UTF-8은 2,406줄, 코코렛의 UTF-8은 327줄이었고, 저마다 자신과 견주면 차이가 없었어요. 개발 환경의 3화(작업 `481f672b`)에서 코코렛 SMI 327줄을 ASS 24줄과 견주자 추가 303·변경 24이고, 스타일·폰트는 까닭과 함께 `비교 불가`였어요. 버전 줄은 `대사 24줄`·`대사 327줄`로 비교가 읽은 수와 같았어요. 계획이 센 SMI `<SYNC>`는 493개였어요. WinPNG SMI는 개발 데이터에 없어 읽지 못했어요. 실제 표본은 저장소에 넣지 않았어요. |
| 할 일 화면의 교체 승인 카드 | API 시험 `the_replacement_to_do_sums_what_its_open_plans_change`, `the_replacement_to_do_sums_timing_styles_and_fonts_too`, `the_replacement_to_do_counts_a_plan_compared_only_in_part`와 웹 시험(`todoTags`)이 합과 태그를 확인해요. 일부를 견주지 못한 계획에는 `일부 비교 불가`, 견주지 못한 계획에는 `비교 불가`가 붙고, 모두 온전히 견줬는데 차이가 없을 때만 `차이 없음`이에요. 개발 환경에서 마지막 수정까지 담은 이미지로 다시 띄웠을 때, 열린 계획 11개의 카드가 `대사 추가 305`·`대사 변경 27`·`대사 삭제 1`·`타이밍 2`·`스타일 2`·`폰트 4`·`일부 비교 불가 1`·`비교 불가 9`였어요. 계획들의 합과 같아요. |

그 밖의 시험도 있어요.

- 8 MiB 경계(`a_current_file_over_eight_mib_is_not_compared`, `a_file_of_exactly_the_limit_is_compared`), 계획이 본 뒤 바뀐 현재 자막(`a_current_file_changed_after_the_plan_saw_it_is_not_compared`), 다시 계획한 판의 새 비교(`a_plan_made_again_after_a_change_has_a_comparison_of_its_own`), 이전 빌드의 계획(`a_plan_made_by_an_earlier_build_has_no_comparison`), `현재`를 고르는 순서(`the_current_subtitle_is_the_replaced_file_else_the_removed_copy_else_a_kept_file`), 마이그레이션 57의 CHECK와 트리거(`migration_57_keeps_a_comparison_for_a_plan_and_refuses_what_is_not_one`)를 확인해요.
- 일부러 만든 파일의 시험이 읽기 시간과 상한을 확인해요. 닫히지 않는 ASS 블록, `&`만 가득한 글, 한 시각에 몰린 SMI `<SYNC>`, 닫히지 않는 SMI 태그는 5초 안에 읽혀요. 대사·SMI 문단·스타일·줄 길이·폰트·언어 구분·형식 항목·이름 길이가 상한을 넘으면 까닭과 함께 읽지 않아요. 10자리를 넘는 SMI `Start`는 건너뛰고 시간 계산이 넘치지 않아요. 엔진이 패닉하면 계획만 `비교 불가`가 돼요(`an_engine_that_panics_leaves_the_plan_not_compared`).
- 줄 맞추기는 개발 PC의 release 빌드에서 50,000줄끼리 모든 줄이 다를 때 3.26초, 같을 때 4.5ms였어요(2026-10-05, `probe-out/0069-lcs-timing.txt`). 줄 JSON은 6.78 MB였어요.

### 독립 검토

검토(2026-10-05)는 인터넷에서 받은 파일이 비교에 쓰는 시간·메모리·남기는 크기를 지적했고, 아래처럼 고쳤어요.

- ASS 블록, 글자 참조, SMI `<SYNC>`·`Format:`을 다시 훑는 부분이 파일 길이의 제곱으로 걸렸어요. 이제 모두 길이에 비례해요.
- 엔진의 패닉이 작업을 다섯 번 다시 돌린 뒤 보류하게 했어요. 이제 그 비교만 `비교 불가`예요.
- 작업 상세가 몇 초마다 읽는 요약이 파일에 따라 끝없이 커질 수 있었어요. 이제 읽는 도중에 상한을 넘으면 멈춰요.
- 할 일 카드가 일부만 견준 계획을 `차이 없음`으로 보였어요. 이제 `일부 비교 불가`예요.
- 시간이 끝에 가까운 SMI 줄의 겹침 계산이 넘칠 수 있었어요. 화면의 줄 목록은 다시 그릴 때마다 줄을 새로 만들었어요.

고친 뒤의 재검토(2026-10-05)는 위 수정이 맞다고 봤고, 네 가지를 더 찾았어요. `Format:` 줄의 항목 이름에 상한이 없어 스타일마다 복사되면 8 MiB 파일 하나가 수 GB를 잡을 수 있었어요. 이제 `Format:` 줄도 1,024바이트까지예요. SMI 문단은 상한을 보기 전에 목록을 만들었는데, 이제 먼저 세요. SMI 문단 상한과 폰트의 첫 표기에는 시험이 없었는데 더했어요. 열린 계획이 없는 카드는 `차이 없음` 대신 태그를 보이지 않아요. 세 번째 확인(2026-10-05)에서 네 가지 수정이 모두 맞다고 봤어요. 고친 코드를 하나씩 되돌리면 해당 시험이 실패해요(`probe-out/0069c-mutations.txt`, `probe-out/0069d-mutations.txt`). ASS를 읽는 도중의 대사 상한만은 예외예요. 다 읽은 뒤 모든 언어 구분을 합쳐 다시 세므로 결과가 같고, 읽는 동안의 메모리만 줄여요.

### 시험

마지막 수정 뒤의 작업 트리(2026-10-05, `d28dd45` 위)에서 `cargo test --workspace`를 돌렸어요. 2,632개가 통과하고 6개가 실패했으며 13개는 무시했어요. 실패한 6개는 `trss-archive`의 `refusals.rs`였고, 원인은 시험 바이너리였어요. 지운 worktree에서 같은 `target/`에 빌드한 바이너리가 그 worktree의 표본 경로를 담고 있었어요. 그 바이너리를 지우고 다시 빌드하자 `refusals.rs` 30개와 `trss-jobs`의 `unpack.rs` 11개가 모두 통과했어요. 그래서 2,638개 통과, 실패 0이에요.

0069의 시험 수는 이래요.

- 비교 엔진 39개
- `diff.rs` 8개
- 교체 시험(`replace.rs`) 45개, 그중 0069의 것이 11개
- 교체 API 시험 13개

`cargo clippy --workspace --all-targets -D warnings`와 `cargo fmt --all --check`가 통과해요. 웹은 `bun run test` 161개가 통과하고 `tsc -b`가 통과해요.

### 한계

- WinPNG SMI는 개발 데이터에 없어 읽어 보지 못했어요.
- 줄 맞추기 시간은 개발 PC에서만 쟀고 j4105에서는 재지 않았어요.
- BOM 없는 UTF-16과 CP949가 아닌 옛 인코딩은 읽지 못해요. 다른 인코딩의 바이트가 CP949로 오류 없이 읽히면 깨진 글자로 견줄 수 있어요.
- 같은 글자의 줄이 여러 번 나오면 시간 순서로 맞추므로, 짧은 줄의 타이밍 차이가 실제와 다른 줄에 붙을 수 있어요.
- 화면은 한 번 받은 줄을 페이지를 떠날 때까지 다시 받지 않아요. 비교는 바뀌지 않으므로 같은 계획의 줄은 그대로예요.
- 마이그레이션 57 전에 만든 계획은 `비교 불가`로 남아요.
- 휴대폰 폭은 내장 브라우저의 375px로만 봤고 실제 기기에서는 보지 않았어요.
