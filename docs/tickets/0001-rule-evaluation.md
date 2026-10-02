# 0001 규칙 판정을 공통 모듈로 옮기고 정규식을 동작시켜요

- 상태: 완료
- 출처: [채널과 다운로드 규칙](../specs/collection.md#채널과-다운로드-규칙), [공통 라이브러리의 모듈 구성](../specs/web-app.md#공통-라이브러리의-모듈-구성)
- 막는 티켓: 없음

## 작업

지금 규칙 판정은 실행 파일 안의 `collect_items`에 묶여 있고, `regex: true`는 `unimplemented!()`로 패닉해요.
worker의 실제 처리와 웹의 미리보기가 같은 판정을 쓰도록, 채널 제외 조건·규칙 일치·첫 일치 선택·저장 위치 계산을 공통 라이브러리의 RSS 모듈로 옮기고 정규식을 실제로 동작시켜요.
이 티켓은 뒤의 미리보기 티켓(0006)과 worker 티켓(0004)이 같은 판정을 쓰게 하려는 선행 정리이며, 기존 YAML로 동작하는 지금 실행 파일의 결과를 바꾸지 않아요.

- 판정 결과는 항목마다 선택 여부, 적용 규칙, 저장 위치를 돌려주고, 선택하지 않았다면 그 까닭(채널 제외·규칙 불일치)을 구분해요.
  수집 화면의 `겹침` 표시에 쓰도록, 첫 일치 규칙 말고도 같은 항목에 맞는 뒤쪽 규칙을 알려줘요.
- 일치 문구가 없는 규칙(`제목 대기` 구독)은 어떤 항목에도 맞지 않아요.
- 정규식 컴파일 오류는 규칙 단위의 오류로 돌려주며, 부분 문자열 검색으로 바꾸거나 패닉하지 않아요.
  잘못된 규칙이 있어도 같은 채널의 다른 규칙 판정은 계속돼요.
- 영상 이름 변경(trname)과 회차 변환은 이 티켓의 판정에 넣지 않아요.

## 완료 기준

외부 Transmission·네트워크 없이 도는 격리된 테스트로 확인해요.
지금의 `test_get_torrent`처럼 외부 주소에 기대는 테스트에 의존하지 않아요.

| 입력 | 기대 결과 |
| --- | --- |
| 채널 `excludes`에 든 문자열을 포함한 제목 | 규칙과 관계없이 채널 제외로 선택하지 않아요. 제외 문자열 비교는 지금처럼 대소문자를 구분해요. |
| `case_insensitive: true` 규칙과 대소문자만 다른 제목 | 선택해요. `false`면 선택하지 않아요. |
| 두 규칙에 모두 맞는 제목 | 채널 안 순서의 첫 규칙을 적용하고, 두 번째 규칙을 겹친 규칙으로 알려요. |
| `regex: true`, `^\[SubsPlease\] Sono Bisque Doll - (1[3-9]\|2[0-4])` 같은 규칙 | 정규식으로 판정해요. `case_insensitive`도 정규식에 적용해요. |
| 컴파일되지 않는 정규식 | 그 규칙의 오류를 돌려주고 다른 규칙은 판정해요. 패닉하지 않아요. |
| 일치 문구가 없는 규칙 | 어떤 제목에도 맞지 않아요. |
| 채널 경로 `/media/anime`, 규칙 경로 `Sayonara Lara/Season 01` | 저장 위치가 두 경로를 이은 값이에요. |

- 기존 실행 파일이 새 모듈을 쓰도록 바꾼 뒤, 사용자의 기존 채널 설정과 같은 구조의 표본 YAML(토큰은 가린 값)과 RSS 표본에서 바꾸기 전과 같은 항목·규칙·저장 위치가 선택되는 것을 비교 테스트로 확인해요.
- `cargo test`가 외부 연결 없이 통과해요.

## 결과

채널 제외 조건·규칙 일치·첫 일치 선택·겹침 알림·저장 위치 계산을 공통 라이브러리의 `rss` 모듈(`crates/trss-collect/src/rss/`)로 옮기고, `regex: true`를 실제 정규식으로 판정하게 했어요.
`transmission-rss` 실행 파일은 `rss::legacy::collect_items`를 거쳐 이 모듈을 써요.

- 판정은 `ChannelEvaluator::new(ChannelSpec)`로 정규식을 한 번 컴파일한 뒤 `evaluate(title)`로 항목마다 돌려요.
  결과는 선택 여부·적용 규칙 번호·저장 위치·전달만 하는 `episode`, 선택하지 않은 까닭(`ChannelExcluded`·`NoRuleMatched`), 뒤에서 함께 맞는 규칙 번호 목록(`overlapping`)이에요.
  규칙은 목록 안의 번호로 가리키므로, 웹과 worker가 자기 저장소의 규칙 식별자로 옮기는 일은 뒤 티켓의 몫이에요.
- 컴파일되지 않는 정규식은 `rule_errors()`에 규칙 번호와 함께 남고, 그 규칙은 어떤 제목에도 맞지 않아요. 패닉하지 않고 부분 문자열 검색으로 바꾸지 않으며, 같은 채널의 다른 규칙은 계속 판정해요.
- `pattern: None`(일치 문구 없음)은 어떤 제목에도 맞지 않아요.
- 실행 파일은 잘못된 정규식 규칙을 표준 오류로 알리고 넘어가요. 예전에는 패닉했어요.

### 확인한 것

| 확인 | 명령 | 결과 |
| --- | --- | --- |
| 완료 기준 표의 각 행을 격리된 단위 테스트로 확인(`crates/trss-collect/src/rss/evaluate.rs`) | `cargo test` | 16개 통과 |
| 기존 로직을 그대로 옮긴 기준 구현과 새 모듈의 선택 결과 비교(`tests/legacy_comparison.rs`, 표본 `crates/trss-import/tests/fixtures/legacy_channels.yml`·`sample_feed.xml`) | `cargo test` | 2개 통과. 항목 링크·저장 위치·회차 시작값이 같음 |
| 빌드와 정적 검사 | `cargo build`, `cargo clippy --all-targets` | 통과, 경고 없음 |

- 완료 기준 표의 행과 테스트의 대응은 다음과 같아요.
  채널 제외(`channel_exclude_wins_over_matching_rule`, `channel_exclude_is_case_sensitive`), 대소문자(`case_insensitive_rule_ignores_case`), 겹침(`first_rule_applies_and_later_matching_rules_are_reported`), 정규식(`regex_rule_matches_by_pattern`, `regex_honors_case_insensitive`, `regex_is_not_a_substring_search`), 컴파일 오류(`invalid_regex_is_a_rule_error_and_other_rules_keep_evaluating`, `invalid_regex_does_not_fall_back_to_substring`), 일치 문구 없음(`rule_without_pattern_matches_nothing`, `title_waiting_rule_does_not_shadow_or_overlap`), 저장 위치(`save_path_joins_channel_and_rule_directories`).
- 표본 YAML은 `url`·`directory`·`excludes`와 `match`·`regex`·`case_insensitive`·`directory`·`episode`(`-12`, `-24` 같은 음수 포함)를 갖고, 토큰은 `token=REDACTED`로 가렸어요.
  예전 로직은 `regex: true`에서 패닉하므로 비교 표본에는 정규식 규칙을 넣지 않았어요.
- 외부 Transmission에 기대는 `test_get_torrent`, `test_add_torrent`에 `#[ignore]`와 까닭을 달아 `cargo test`가 외부 연결 없이 돌아요. 테스트는 지우지 않았어요.

### 확인하지 못한 것

- 실제 Transmission·실제 RSS 채널과 함께 실행 파일을 돌려 보지 않았어요. 실행 파일의 선택 결과 동일성은 표본 YAML·RSS를 `collect_items`에 넣어 확인했어요.
- 사용자의 실제 채널 YAML로는 비교하지 않았고, 표본은 같은 구조를 흉내 낸 것이에요.
- musl 빌드(`clux/muslrust`)는 돌려 보지 않았어요. 추가한 `regex` 크레이트는 시스템 라이브러리에 기대지 않아요.

### 남은 메모

- 영상 이름 변경(trname)과 회차 변환은 이 판정 밖이에요. `episode`는 값을 그대로 돌려줄 뿐 판정에 쓰지 않아요.
- 빈 문자열 일치 문구(`Some("")`)는 예전 부분 문자열 검사처럼 모든 제목에 맞아요. 기존 YAML의 `match: ""`가 그대로 유지되도록 한 선택이에요.
  뒤 티켓에서 DB의 `제목 대기`(빈 문구)를 이 모듈에 넘길 때는 `None`으로 바꿔 넘겨야 모든 항목을 받는 일이 없어요.
- 제외 문자열 비교는 지금처럼 대소문자를 구분하는 부분 문자열이에요.
- 규칙 번호와 `RuleSpec`·`ChannelSpec`은 DB 기록 형식에 기대지 않아요. DB 형식과의 변환은 0002 이후 티켓에서 정해요.
- `rss::legacy`(YAML 어댑터)와 `Rule`·`ChannelConfig`는 worker 전환(0004) 뒤에 실행 파일과 함께 정리할 대상이에요.
