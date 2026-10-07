# 0118 작품 상세가 trss가 적용한 자막의 제작자를 보관본에서 읽어요

- 상태: 완료 (2026-10-07)
- 출처: [머리와 시즌](../specs/library.md#머리와-시즌), [0083](0083-deployed-end-to-end.md)의 관찰(2026-10-07)
- 막는 티켓: 없음. 목표 5의 확인을 마친 뒤 0118–0123을 함께 마쳐 0.6.1로 실제 서버에 올리고, 그다음 리팩터링의 [0092](0092-refactor-baseline.md)를 시작해요(사용자 결정, 2026-10-07).

## 작업

0083에서 실제 서버의 작품 상세를 보다 찾았어요.
trss가 코코렛의 보관본으로 `FX Senshi Kurumi-chan` 1화에 적용한 자막이 있는데, 머리의 자막 제작자가 `코코렛, 제작자 알 수 없음`이에요.
작품 상세 API에서 회차 자막 파일의 `creator`는 사용자가 그 파일에 붙인 제작자(`media_files.creator_source_id`)뿐이고, 적용 단계는 이 값을 채우지 않아요. 그래서 머리(웹의 `headNames`)가 trss가 적용한 파일을 제작자 모르는 파일로 세요.
명세는 직접 넣은 파일처럼 제작자를 모를 때만 `제작자 알 수 없음`을 보여 달라고 해요.
같은 날 `All Works Maid`에서도 같았어요. 에루샤의 보관본으로 적용한 9–12화가 제작자 모르는 파일로 세어졌고, 사용자가 `제작자 지정`으로 에루샤를 붙였어요(0083).

- 영상 옆 자막 파일이 어떤 보관본의 적용본이면, 그 파일의 제작자는 그 보관본의 제작자로 보여줘요. 보관본이 `제작자 알 수 없음`이면 적용본도 그래요(사용자 결정, 2026-10-07).
  DB에 새 값을 쓰지 않고 이미 있는 적용 관계로 정해요. 그래서 이미 적용된 파일도 고친 릴리스가 올라가면 바로 맞게 보여요. 적용할 때 파일에 제작자를 적는 안은 이미 적용된 파일을 따로 채워야 해서 고르지 않았어요.
- 사용자가 붙인 제작자는 적용본이 아닌 파일의 기록으로 남아요. 같은 경로의 파일이 교체로 적용본이 되면, 그 경로에 남은 사용자의 제작자보다 적용 관계가 앞서요.
- 닿는 곳은 머리의 제작자 목록, `제작자 지정`이 세는 제작자 모르는 자막, 회차 줄을 펼친 자리와 `파일` 카드의 파일 제작자예요.
- 적용본 하나의 제작자를 바꾸는 자리를 숨길지, 보관본의 제작자로만 보여줄지는 구현 때 정해요. 정한 규칙은 명세의 [머리와 시즌](../specs/library.md#머리와-시즌)에 같은 변경으로 적어요.
- 구독 제작자 자동 수신은 받은 작업의 기록으로 수정본을 판단해서(trss-jobs의 `follow.rs`) 이 결함과 상관없어요. 이 티켓은 보여주는 것만 바꿔요.

## 완료 기준

| 상태 | 기대 결과 |
| --- | --- |
| 구독 제작자의 보관본이 적용된 회차만 있는 시즌 | 머리에 그 제작자만 보이고 `제작자 알 수 없음`은 없어요. |
| 직접 넣은 자막(적용본이 아닌 파일)이 있는 시즌 | 지금처럼 `제작자 알 수 없음`이 보이고, `제작자 지정`이 그 파일만 세요. |
| 사용자가 제작자를 붙인 경로의 파일이 다른 제작자의 적용본으로 교체됨 | 그 파일은 적용한 보관본의 제작자로 보여요. |
| `제작자 알 수 없음` 보관본의 적용본 | `제작자 알 수 없음`으로 보여요. |

- 규칙은 그 규칙을 가진 크레이트에서 한 번 시험해요([ADR 0015](../adr/0015-test-a-rule-once-in-its-crate.md)).
- 0.6.1을 올린 뒤 실제 서버의 `FX Senshi Kurumi-chan` 머리에서 `제작자 알 수 없음`이 사라진 것을 봐요.

## 결과

### 만든 것 (2026-10-07)

- trss-library의 `work_detail`이 자막 파일마다 적용 관계(`subtitle_applied`에서 지우지 않은 행이 그 경로를 가리키는지)를 함께 읽어요. 적용본이면 보관본(`subtitle_stored.creator`)의 이름을 `FileRecord::applied`(`AppliedCopy { creator }`)에 담고, 그 파일의 사용자가 붙인 제작자(`creator`)는 비워요. 그래서 같은 경로에 붙인 제작자가 있어도 적용 관계가 앞서요.
  DB에 값을 쓰지 않고 마이그레이션과 backfill도 없어요. 이미 적용된 파일도 올린 릴리스에서 바로 맞게 보여요.
- `name_unknown_creators`(`제작자 지정`의 한 번에 붙이기)가 적용본을 빼요. 붙인 개수에도 적용본이 들어가지 않아요.
- 작품 상세 API의 자막 파일에 `applied`를 더했어요. 사용자가 넣은 파일은 `null`, 적용본은 `{ "creator": 이름 | null }`이에요(`null`은 `제작자 알 수 없음`). 모듈 설명(`library_work_api.rs`)에 `creator`와 `applied`의 뜻을 적었어요.
- 웹은 `fileCreators.ts`에 모은 규칙(`creatorOf`, `isNameable`, `unknownFiles`, `headNames`)으로 머리의 제작자 목록, `제작자 지정`이 세는 개수, 회차 줄을 펼친 자리의 제작자, `파일` 카드의 파일 제작자를 같은 값으로 보여줘요.
- 명세의 [머리와 시즌](../specs/library.md#머리와-시즌)에 적용본의 제작자 규칙을 더했어요.
- 적용본에는 `제작자 바꾸기`를 두지 않고 보관본의 제작자만 읽기 전용으로 보여줘요(구현 결정, 2026-10-07). 제작자는 보관본에서 오는 값이고 적용 관계가 사용자가 붙인 값보다 앞서므로, 바꾸는 자리를 두면 바꿔도 보이지 않는 값을 쓰게 돼요.
  사람이 적용본을 지우면 그 경로에 남은 붙인 제작자가 다시 보여요.

### 검증한 것

| 완료 기준 | 근거 |
| --- | --- |
| 구독 제작자의 보관본이 적용된 회차만 있는 시즌 | 규칙은 trss-library의 `a_season_applied_from_a_creators_copies_shows_that_creator_and_has_no_unknown_file_to_name`이 확인해요(적용본 셋이 모두 보관본의 제작자로 읽히고, 한 번에 붙이기가 0개를 붙여요). 머리 이름은 웹의 `fileCreators.test.ts`의 `a season applied only from the subscribed creator's copies shows that creator and no unknown one`이 확인해요. |
| 직접 넣은 자막이 있는 시즌 | `a_file_put_in_by_hand_stays_unknown_and_is_named_while_applied_copies_are_left_out`이 적용본 하나와 직접 넣은 둘에서 직접 넣은 둘만 붙고(2개) 적용본의 열은 그대로인 것을 확인해요. 웹은 `a subtitle put in by hand keeps the head's unknown creator, and only it is counted for 제작자 지정`이 확인해요. |
| 사용자가 붙인 경로가 다른 제작자의 적용본으로 교체됨 | `a_named_file_the_app_replaced_shows_the_applied_copys_creator_and_the_named_one_returns_if_the_copy_is_gone`이 붙인 제작자(카이란)가 있는 경로의 적용본을 하느의 보관본으로 보이는 것과, 지운 적용본은 적용본이 아닌 것을 확인해요. 웹은 `a file the user named and trss then replaced shows the applied copy's creator`가 확인해요. |
| `제작자 알 수 없음` 보관본의 적용본 | `a_copy_of_a_stored_subtitle_with_no_creator_is_applied_from_an_unknown_creator_and_not_named`가 적용본이 `제작자 알 수 없음`으로 읽히고 붙이기에 세어지지 않는 것을 확인해요. 웹은 `an applied copy of a stored copy with no creator shows as unknown but is not counted for naming`이 확인해요. |

- 첫째·둘째·넷째 trss-library 테스트는 `name_unknown`의 `NOT EXISTS`를 무력화하면 실패해요(그렇게 바꿔 돌려 셋이 실패하는 것을 봤어요). 적용 관계를 읽는 쪽은 `FileRecord::applied`가 없던 때에는 컴파일되지 않아요.
- API 모양은 trss-web의 `a_subtitle_file_tells_whether_it_is_an_applied_copy_and_of_whom`이 확인해요(`applied`가 `null`에서 `{ "creator": "하느" }`로 바뀌고 `creator`는 `null`).
- 2026-10-07 실행(이 티켓의 커밋 위): `cargo test -p trss-library -j 3`은 240개와 `artwork_header_check` 1개가 통과했고, `cargo test -p trss-web -j 3`은 462개와 통합 테스트 파일이 통과했어요(건너뛴 4개는 Docker 테스트). 웹은 0119·0120을 합친 master 위에서 `npm run typecheck`(앱과 테스트의 두 project)가 오류 없이 끝났고 `npm test`가 211개 모두 통과했어요. `fileCreators.ts`는 처음에 `@/`를 쓰는 `api.ts`의 타입을 가져와, 경로 설정이 없는 테스트 project의 typecheck가 실패했어요. 그래서 읽는 모양을 파일 안의 타입으로 두고, 구독 줄의 문장(`subtitleChoice`)은 화면이 넘겨요.

### 검증하지 못한 것

- 0.6.1을 올린 뒤 실제 서버의 `FX Senshi Kurumi-chan` 머리에서 `제작자 알 수 없음`이 사라진 것은 아직 보지 못했어요. 이 티켓은 그 확인이 남아 있어요.
- 화면은 브라우저로 열어 보지 않았어요. 규칙은 `node --test`와 타입 검사로만 확인했어요.
- 구독 제작자 자동 수신과 수정 후보 판단(`attributed_subtitles`)은 `media_files`에 남은 사용자가 붙인 제작자를 그대로 읽어요. 붙인 파일을 trss가 교체한 경우 그 판단이 옛 제작자를 쓸 수 있는지는 이 티켓에서 다루지 않았어요.
- 화면이 읽은 뒤에 적용된 파일의 `제작자 바꾸기`를 눌러 보낸 요청은 서버가 막지 않아요. 값은 쓰이지만 적용 관계가 앞서 보이지 않아요.
