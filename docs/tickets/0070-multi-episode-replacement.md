# 0070 여러 회차의 교체를 회차 목록에서 골라요

- 상태: 완료 (2026-10-05)
- 출처: [교체 비교와 승인](../specs/subtitles.md#교체-비교와-승인)의 회차 목록(사용자 결정, 2026-10-04)
- 막는 티켓: [0068](0068-replacement-approval.md)

## 작업

- 한 작업이 여러 회차의 교체를 기다리면 작업 상세의 주 영역은 회차 목록이에요.
  회차 줄마다 변경 요약과 `현재 유지`·`새 자막으로 교체`를 두고, 줄을 펼치면 그 회차의 비교를 봐요. 목록 위에 `모두 교체`·`모두 유지`를 둬요.
- 한 번에 골라도 승인 증거와 반영 직전 검사는 회차마다 그 회차의 계획에 묶어요. 계획이 바뀐 회차만 다시 비교를 요구해요.
- 일부 회차가 실패하면 회차별 결과에 성공·실패와 이유가 있고, 작업 상태는 `일부 실패`예요.

## 완료 기준

| 입력·상태 | 기대 결과 |
| --- | --- |
| 한 작업에서 12회차가 교체를 기다림 | 회차 목록에 12줄이 있어요. |
| `모두 교체`를 누르기 전에 3화에 새 수정본이 들어옴(시험) | 3화만 `다시 비교 필요`로 남고 나머지 11회차는 반영돼요. 승인 기록이 회차마다 있어요. |
| 2화는 `현재 유지`, 나머지는 `모두 교체` | 2화 파일은 그대로이고 나머지가 바뀌어요. |
| 한 회차의 반영이 실패 | 그 회차만 실패 이유가 있고 작업 상태는 `일부 실패`예요. |
| 휴대폰 폭 | 목록과 버튼을 가로 스크롤 없이 쓸 수 있어요. |

## 결과

### 만든 것 (2026-10-05)

- **한 번에 여러 결정**(`records::decide_all`, `JobStore::decide_replacements`): 결정 목록을 한 IMMEDIATE 트랜잭션에서 계획마다 따져요. 그 판이 줄의 마지막 판이고 아직 정하지 않았으면 하나를 정할 때와 똑같이 적고(계획 상태·결정 시각, `현재 유지`면 그 줄을 `보관만 함`, 사건 기록), 아니면 `stale`로 두고 건너뛰어요. 하나라도 적었으면 `교체 승인`에서 기다리던 작업을 한 번만 다시 줄에 세워요. 이 작업의 계획이 아닌 것이 하나라도 있으면 아무것도 적지 않아요. 한 계획의 결정(`decide`)도 이 경로를 써요.
- **API**: `POST /api/subtitle-jobs/{id}/replacements`가 `{decisions:[{plan,version,decision}]}`를 받아 요청 순서대로 `{results:[{plan,state}]}`(`approved`·`kept`·`stale`)를 돌려줘요. 빈 목록, 1,000개 초과, 같은 계획 두 번, `replace`·`keep`이 아닌 결정은 `400`이고, 다른 작업의 계획이 섞이면 `404`예요. 무엇이든 적었을 때만 worker를 깨워요.
- **웹**(`Replacement.tsx`, `replacementView.ts`, `changes.ts`): 열린 계획이 둘 이상이면 작업 상세의 주 영역은 회차 목록이에요. 줄마다 회차, 그 계획만의 변경 태그(`planChanges`가 서버의 카드 합 규칙을 한 계획에 적용해요), `현재 유지`·`새 자막으로 교체`가 있고, 펼치면 결정 카드의 버전 줄·나란한 비교·경고·`변경 사항`이 보여요. 목록 머리의 `모두 교체`·`모두 유지`는 보인 계획을 한 요청에 담고, `stale`로 돌아온 줄에는 다시 비교가 필요하다고 알려요. PC에서는 목록 머리가 위쪽에 붙어 따라와요.
- **명세**: [교체 계획과 반영](../specs/subtitles.md#교체-계획과-반영)에 회차 목록과 한 번에 여러 결정을 더하고, 한계 줄의 0070 예고를 지웠어요. [작업 화면](../specs/jobs.md)의 스크롤 기준에 목록 머리를 더했어요.

### 검증한 것

| 완료 기준 | 근거 |
| --- | --- |
| 한 작업에서 12회차가 교체를 기다림 | 작업 시험 `twelve_episodes_waiting_give_twelve_open_plans_in_one_job`이 한 작업에 열린 계획 12개를 확인해요. 웹 시험(`asEpisodeList`)이 계획이 둘 이상일 때 목록을 고르는 것을 확인해요. `approving_all_at_once_replaces_every_episode_with_one_requeue_and_a_record_each`는 12개를 한 번에 승인하면 다시 줄에 서는 것이 한 번이고, 12회차가 모두 새 바이트이며 회차마다 결정 기록이 있는 것을 확인해요. 개발 환경(2026-10-05, 이 변경을 담은 이미지)에서는 열린 계획이 둘인 작업 `d7422d05`에 회차 목록이 보였어요. `모두 교체`를 누르자 `POST .../replacements` 한 번이 `200`이었고, 1화·2화의 영상 옆 자막이 계획한 SHA-256으로 바뀌었어요. 회차마다 `새 자막으로 교체하기로 했어요` 기록이 남고 작업은 `받음`이 됐어요(`probe-out/0070-e2e-before.txt`, `probe-out/0070-e2e-after.txt`). |
| `모두 교체` 전에 3화에 새 수정본이 들어옴 | `a_new_revision_of_one_episode_leaves_it_to_compare_again_and_the_others_are_replaced`에서 나머지 11회차는 회차마다 결정 시각과 함께 반영되고, 3화는 파일이 그대로이며 계획이 `새 수정본 발견`(`다시 비교 필요`)으로 닫혀요. 3화의 새 비교는 새 수정본의 작업이 맡고 그 작업은 `교체 승인`에서 기다려요. 앞 작업은 3화 줄을 `보관만 함`으로 두고 `완료`예요. 명세의 기존 규칙(새 수정본은 그 작업이 비교해요)과 같아요. 결정 직전에 계획이 다시 만들어진 경우는 `an_episode_planned_again_before_the_batch_is_stale_and_the_others_are_approved`가 그 회차만 `stale`이고 나머지는 승인되는 것을 확인해요. |
| 2화는 `현재 유지`, 나머지는 `모두 교체` | `keeping_one_episode_and_approving_the_rest_in_a_batch`에서 2화 파일은 그대로이고 그 줄은 `보관만 함`이며 나머지가 바뀌어요. `keeping_every_episode_in_a_batch_leaves_the_files_and_settles_the_rows`는 모두 유지를 확인해요. |
| 한 회차의 반영이 실패 | `one_episode_that_cannot_be_carried_out_fails_alone_and_the_job_is_partial`에서 12화 폴더를 읽기 전용으로 두자, 12화만 계획과 줄에 "기존 자막을 옮기지 못해 교체하지 않았어요"가 있고 작업은 `일부 실패`예요. 나머지 11회차는 바뀌어요. |
| 휴대폰 폭 | 개발 환경의 375px 내장 브라우저에서 `scrollWidth`가 375이고 넘치는 요소가 없었어요(2026-10-05). `모두` 버튼과 줄의 버튼은 반씩 나눠 놓여요. |

그 밖의 시험도 있어요.

- 다른 작업의 계획이 섞인 목록은 아무것도 적지 않아요(`a_batch_with_a_plan_of_another_job_writes_nothing`, API `a_plan_of_another_job_is_not_found_and_nothing_of_the_list_is_written`).
- 모두 `stale`인 목록은 아무것도 적지 않고 작업을 다시 줄에 세우지 않아요(`a_batch_of_stale_decisions_writes_nothing_and_does_not_put_the_job_back_in_line`). API 시험 `the_worker_is_woken_only_when_a_decision_was_written`은 그때 worker를 깨우지 않는 것을 확인해요.
- API 시험 `several_decisions_at_once_answer_each_plan_in_the_order_asked`는 결과의 순서와 상태를, `a_list_of_decisions_that_cannot_be_read_is_refused_with_nothing_written`은 `400`의 네 경우를 확인해요.
- 웹 시험은 한 계획의 태그(`rowTags`, `planChanges`)가 서버의 합 규칙과 같은지, `decisionsFor`가 보인 계획마다 그 판을 담는지 확인해요.
- `decide_all`의 `404`, `stale` 검사, 다시 줄에 세우는 조건을 하나씩 망가뜨리면 해당 시험이 실패해요(`probe-out/0070-mutations.txt`).

### 독립 검토

검토(2026-10-05)는 서버 쪽에 막을 결함이 없다고 봤어요. 다른 작업의 계획이 섞이면 아무것도 적지 않고, `stale`인 계획은 그대로 두며, 작업은 한 번만 다시 줄에 서요. 한 계획의 결정은 전과 같아요. 지적한 것은 아래처럼 고쳤어요.

- 화면은 `stale` 알림을 계획 ID로 들고 있었어요. 다시 만든 계획은 ID가 새로 생겨서, 다시 읽은 뒤의 줄에는 알림이 보이지 않았어요. 이제 배치 줄의 `position`으로 들고 있어요. 줄에서는 "줄을 펼쳐 새로 읽은 비교를 확인해 주세요"라고, 결정 카드에서는 아래의 비교를 가리켜 알려요.
- `모두 교체`·`모두 유지` 요청이 실패하면 열두 줄 모두에 같은 알림이 붙었어요. 이제 목록 머리에 한 번만 보여요.
- 묶음에서 `현재 유지`한 줄이 결정 때 바로 `보관만 함`이 되는지는 실행 뒤에만 확인했어요. 섞인 묶음 시험(`a_mixed_batch_settles_the_kept_rows_at_once_and_the_run_replaces_the_rest`)을 더해 실행 전에 확인해요. 그 갱신을 지우면 이 시험과 기존 두 시험이 실패해요(`probe-out/0070-mutations.txt`).
- 다른 작업의 계획이 섞인 묶음 시험에 `현재 유지`를 섞고, 배치 줄의 결과와 메모가 그대로인지도 확인해요.
- `decide_all`에 같은 계획이 두 번 오면 뒤의 것은 앞의 것이 적은 상태로 따져져 한 번만 적힌다고 문서에 밝혔어요. API는 그런 목록을 `400`으로 거절해요.

### 시험

검토 반영 전의 작업 트리(2026-10-05, `55a9d5d` 위)에서 `cargo test --workspace -j 4`를 돌렸어요. 2,651개가 통과했고 실패는 0, 무시는 13개였어요. 검토를 반영한 뒤에는 바뀐 시험만 다시 돌렸어요. `replace_many.rs` 10개와 `replace.rs` 45개가 통과해요. `cargo clippy --workspace --all-targets -j 4 -D warnings`와 `cargo fmt --all --check`가 통과해요. 웹은 `bun test` 165개가 통과하고 `tsc -b`가 통과해요.

0070의 시험 수는 이래요.

- 여러 회차 시험(`replace_many.rs`) 10개
- 교체 API 시험 4개(모두 18개)
- 웹 시험 4개(`changes.test.ts` 1개, `replacementView.test.ts` 3개)

### 한계

- 요청이 응답 없이 끊기면 보낸 계획마다 같은 실패 문장이 보여요. 서버가 실제로 적었는지는 작업 상세를 다시 읽을 때 드러나요.
- 휴대폰 폭은 내장 브라우저의 375px로만 봤고 실제 기기에서는 보지 않았어요.
- 개발 환경에는 열린 계획이 둘인 작업이 하나뿐이어서 `모두 교체`만 눌러 봤어요. `모두 유지`, 섞인 결정, 12회차, 새 수정본, 실패는 시험으로만 확인했어요.
