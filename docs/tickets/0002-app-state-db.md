# 0002 채널·규칙을 앱 DB에 저장해요

- 상태: 완료
- 출처: [앱이 소유하는 설정](../specs/settings.md#앱이-소유하는-설정), [채널과 규칙 필드](../specs/settings.md#채널과-규칙-필드), [구현 경계와 실행 순서](../specs/web-app.md#구현-경계와-실행-순서), [저장소 ADR](../adr/0005-local-sqlite-app-state.md)
- 막는 티켓: 없음

## 작업

앱 상태를 담을 로컬 SQLite DB와 그 마이그레이션을 세우고, 첫 내용으로 채널과 다운로드 규칙을 저장·조회·수정하는 기능을 공통 라이브러리에 둬요.
웹과 worker가 같은 DB를 쓰며, 이후 결과 목표의 테이블도 같은 마이그레이션 흐름으로 더해요.

- DB 위치는 배포 설정(환경 변수)으로 정하고, 앱 설정으로 바꾸지 않아요.
- 채널은 안정적인 ID, URL, 기본 저장 폴더, 제외 조건, 쿼리 이름별 비밀 여부와 비밀 값, 지난 회차 검색 형식, 채널 순서를 가져요.
- 규칙은 안정적인 ID, 채널 안 순서, 일치 문구(없을 수 있음), `regex`·`case_insensitive`, 저장 폴더, 회차 변환과 그 값을 자동으로 정했는지, 상태(`active`·`archived`)를 가져요.
  규칙 하나는 채널 하나에 속해요.
  구독 정보는 결과 목표 2에서 더해요.
- 수정은 확인한 대상 버전을 받아, 그 사이 다른 쪽이 먼저 저장했으면 바꾸지 않고 충돌로 거부해요([웹 명령과 상태 갱신](../specs/web-app.md#웹-명령과-상태-갱신)).
- 여러 채널·규칙을 함께 바꾸는 동작(가져오기의 채널 교체, 규칙 순서 변경)은 한 트랜잭션으로 반영해요.
- 비밀 값은 DB에는 원문으로 두되, 조회 결과를 로그나 오류 문구에 찍을 때는 가린 값만 내보내요.

## 완료 기준

격리된 임시 DB로 확인해요.

| 입력·상태 | 기대 결과 |
| --- | --- |
| 빈 DB로 시작 | 마이그레이션이 스키마를 만들고, 다시 열어도 다시 만들지 않아요. |
| 채널 둘, 규칙 다섯을 저장하고 DB를 다시 엶 | ID·순서·모든 필드가 그대로 읽혀요. |
| 같은 규칙을 버전 3으로 두 번 수정 | 첫 수정은 반영되고, 두 번째는 충돌로 거부되며 DB가 바뀌지 않아요. |
| 채널 안 규칙 순서 변경 도중 실패를 일으킴 | 순서가 일부만 바뀌지 않고 이전 순서가 남아요. |
| `?r=1080&token=abc` URL에서 `token`만 비밀로 표시 | 조회는 원문을 돌려주고, 로그·표시용 값은 `token`을 가린 값이에요. |
| 규칙의 채널을 바꾸려는 수정 | 거부해요. 규칙의 채널은 만들 때 정해지고, 다른 채널로 옮기려면 새 규칙을 만들어요. |

- DB 파일을 SMB·NFS 공유 경로에 두지 않는다는 배포 조건은 [0009](0009-deploy-web-worker.md)에서 확인해요.

## 결과

`rusqlite`(`bundled`)로 로컬 SQLite DB와 채널·규칙 저장소를 만들었어요.
DB 기반은 `src/store/db.rs`, 채널·규칙 기능은 `src/store/channels/`에 있어요([웹·worker 분리 ADR](../adr/0006-separate-web-worker-binaries.md)의 "기반은 분리, 기능별 SQL은 기능 모듈"에 맞췄어요).

- 라이브러리: musl 릴리스에서 시스템 라이브러리 없이 SQLite를 C 소스로 함께 빌드하려고 `rusqlite`의 `bundled`를 골랐어요. 비동기 코드에서는 `Db::run`이 `spawn_blocking`으로 감싸 호출해요.
- 마이그레이션: SQL을 바이너리에 내장하고 `PRAGMA user_version`으로 버전을 관리해요. 열 때 `BEGIN IMMEDIATE` 트랜잭션 안에서 밀린 것만 적용하므로, 두 프로세스가 동시에 열어도 한 번만 적용돼요. DB가 이 빌드보다 새 버전이면 열기를 거부해요.
- DB 위치: `Db::open(path)`는 경로를 받고, `Db::open_from_env()`는 환경 변수 `TRSS_DB_PATH`를 읽어요. 앱 설정에는 두지 않았어요.
- 동시성: 연결마다 WAL, `busy_timeout` 5초, 외래 키를 켜요. 쓰기는 모두 `BEGIN IMMEDIATE`로 잡고 그 안에서 버전을 확인해요.
- 수정 충돌: `update_channel`·`update_rule`·`replace_channel`은 확인한 버전을 받고, 다르면 `ChannelError::Conflict`로 거부해요. 순서 변경은 항목별 버전을 함께 받고, 목록이 현재와 다르면 `OrderMismatch`로 거부해요. 순서가 바뀐 항목은 버전이 올라요.
- 비밀 값: URL은 원문으로 저장하고, 비밀인 쿼리 이름 목록(`secret_query`)을 따로 저장해요. `mask_url`·`Channel::masked_url`은 비밀 값을 `***`로 가리고(빈 값은 "아직 입력 전"이라 그대로 둬요), `Debug`와 오류 문구에는 가린 값만 나와요. `ChannelInput::new`는 모든 쿼리 이름을 비밀로 두고 시작해요.
- 규칙 `episode`의 기본값은 기존 YAML의 `starts_episode_at`처럼 1이에요(`RuleInput::default()`).

### 확인한 것

`cargo test --lib store`로 17개 테스트가 통과해요. 모두 임시 DB 파일을 써요.

| 완료 기준 | 테스트 |
| --- | --- |
| 빈 DB로 시작, 다시 열기 | `store::db::tests::empty_db_is_migrated_once_and_reopen_keeps_it` |
| 채널 둘·규칙 다섯 저장 후 다시 열기 | `two_channels_five_rules_survive_reopen_with_every_field` |
| 같은 버전으로 두 번 수정 | `second_update_with_the_same_version_conflicts_and_changes_nothing`, `stale_channel_update_conflicts_across_handles` |
| 순서 변경 도중 실패 | `failure_during_reorder_keeps_the_previous_order`(트리거로 세 번째 쓰기에서 실패시켜 앞선 쓰기까지 되돌아가는지 확인), `reorder_channels_is_versioned_and_atomic` |
| 비밀 값 가리기 | `secret_query_is_stored_verbatim_but_masked_for_display`, `error_messages_do_not_contain_secret_values`, `mask_url_masks_only_secret_values` |
| 규칙의 채널 변경 | `changing_a_rules_channel_is_rejected` |

그 밖에 채널 교체의 원자성(`replace_channel_swaps_the_whole_rule_list_atomically`), 순서 목록 검증, 입력 검증, 새 스키마 거부, 두 핸들이 같은 파일을 공유하는지도 확인했어요.
`cargo build`와 `cargo clippy --all-targets`는 경고 없이 끝나요.

### 확인하지 못한 것

- `cargo test` 전체는 `src/main.rs`의 `test_get_torrent`가 외부 Transmission 호스트에 연결하지 못해 실패해요(이번 변경과 무관해요). 그래서 라이브러리 테스트(`cargo test --lib store`)만 근거로 삼았어요. [0001](0001-rule-evaluation.md)과 합친 뒤에는 이 테스트가 `#[ignore]`라 `cargo test` 전체가 오프라인으로 통과해요.
- musl 타깃 빌드(`clux/muslrust`)는 이 환경에 타깃과 `musl-gcc`가 없어 시험하지 못했어요. `bundled` 빌드라 문제가 없으리라 기대하지만 [0009](0009-deploy-web-worker.md)에서 이미지 빌드로 확인해야 해요.
- 웹과 worker가 서로 다른 프로세스로 동시에 쓰는 경우는 확인하지 못했어요. 같은 파일을 여는 두 핸들(연결)까지만 시험했어요.

### 남은 것

- 구독 필드, 채널·규칙 삭제, 채널 교체 때 기존 규칙 ID를 유지하는 방식은 다루지 않았어요(교체하면 규칙 ID를 새로 발급해요).
- `RuleInput.directory`는 절대 경로만 거부하고 `..` 구성 요소는 막지 않아요. 저장 폴더의 경로 검증은 파일을 실제로 쓰는 쪽에서 해야 해요.
- 규칙 평가 타입([0001](0001-rule-evaluation.md))과는 독립된 레코드 타입이라, 서로 옮기는 코드는 이후 티켓에서 더해요.
