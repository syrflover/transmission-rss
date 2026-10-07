# 0093 테스트 DB를 테스트 프로세스마다 한 번만 만들어요

- 상태: 완료 (2026-10-07)
- 출처: [테스트 DB를 한 번만 만들기](refactoring.md#테스트-db를-한-번만-만들기)
- 막는 티켓: 없음. 계획은 [0092](0092-refactor-baseline.md)(기준값) 뒤였지만, musl 테스트 시간을 줄이려고 그보다 먼저 했어요(사용자 요청, 2026-10-07).

## 작업

지금 테스트는 새 DB를 열 때마다 trss-core의 마이그레이션 62개를 처음부터 돌려요. 파일 DB와 메모리 DB 모두 같아요.
2026-10-07 `07c454d`의 측정에서 한 번에 64 ms(열두 스레드가 함께 열 때 110 ms)가 걸렸고, workspace 테스트 한 번에 약 1,814번 열었어요. 만든 DB 파일을 복사해 열면 1.09 ms, 메모리 DB에서 backup으로 받으면 52 µs였어요.
테스트 프로세스마다 마이그레이션한 DB를 한 번 만들고, 테스트는 그 복사본을 받게 해요. 리팩터링의 다른 티켓이 테스트를 자주 돌리므로 가장 먼저 해요.

- 마이그레이션 자체를 확인하는 테스트는 지금처럼 처음부터 돌려요. trss-core의 `db.rs` 테스트와 마이그레이션 사례 테스트가 여기에 들어요.
- 제품 코드의 DB 열기는 바뀌지 않아요. 테스트 전용 경로를 feature로 두면, Cargo의 feature 통합 때문에 제품 빌드에 켜지지 않는지 `cargo tree -e features`로 확인해요.

## 완료 기준

- trss-jobs, trss-web, trss-worker, trss-collect 테스트 바이너리의 실행 시간을 앞뒤로 견준 표가 결과 절에 있어요. 0092와 같은 명령과 빌드 상태로 재요.
- 통과·건너뛴 테스트 수가 앞뒤로 같아요.
- 마이그레이션 테스트가 여전히 빈 DB에서 마이그레이션을 돌리는 것을, 그 테스트들의 이름과 함께 결과 절에 적어요.
- trss-web과 trss-worker의 제품 빌드에 테스트 전용 경로가 들어가지 않아요.

## 결과

### 만든 것 (2026-10-07)

- 테스트 빌드(`cfg(test)`와 trss-core의 `test-support` feature)에서 `Db::open`이 새 DB에 마이그레이션을 돌리지 않아요. 테스트 프로세스마다 메모리 DB 하나를 처음 한 번 마이그레이션하고, 새 DB는 그것을 SQLite backup API로 받아요(`crates/trss-core/src/db.rs`의 `template`).
  - `:memory:`는 연결을 연 뒤 template에서 받아요.
  - 파일은 경로에 아무것도 없을 때만 받아요. SQLite가 파일을 만들기 전에 옆 경로에 복사본을 쓰고 hard link로 들여놔요. 그래서 같은 경로를 함께 여는 쪽은 아무것도 없거나 완성된 파일만 봐요. 복사나 link가 안 되면 평소처럼 열어서 평소의 오류를 내요.
  - 이미 있는 파일은 빈 파일이어도 지금처럼 마이그레이션해요.
- rusqlite의 `backup` feature는 `test-support`와 trss-core의 dev-dependency에서만 켜요.
- 측정한 시간을 표로 모으는 `dev/measure/test-times.py`를 더했어요.

### 검증한 것

2026-10-07, `b70ae9f` 위의 이 변경과, trss-core만 `b70ae9f`로 되돌린 상태를 견줬어요. 개발 PC(Ryzen 5 5600X, RAM 32 GB)에서 0092와 같은 `cargo test --locked --workspace -j 4`를, 한 번 빌드한 뒤 바꾸지 않고 다시 돌린 상태로 측정했어요.
시간은 테스트 바이너리의 `finished in`이고, `dev/measure/test-times.py`로 모았어요. 원자료는 이 PC의 `dev/local/measure/musl-tests/`에 있어요.

glibc(개발 PC의 평소 실행)는 앞뒤로 두 번씩 돌렸어요.

| 바이너리 | 앞 | 뒤 |
| --- | ---: | ---: |
| trss-worker 통합 테스트 | 15.08 / 15.35초 | 10.93 / 11.01초 |
| trss-jobs 통합 테스트 | 14.84 / 14.54초 | 10.51 / 10.53초 |
| trss-web lib | 11.11 / 11.21초 | 7.13 / 7.16초 |
| trss-collect lib | 3.75 / 2.84초 | 0.94 / 0.92초 |
| 테스트 바이너리 31개의 합 | 71.5 / 68.5초 | 53.0 / 53.1초 |
| 통과·실패·무시 | 2,755·0·14 | 2,756·0·14 |

musl(`dev/musl-test.sh`와 같은 컨테이너, tmpfs `/tmp`)은 앞뒤로 한 번씩 돌렸어요.

| 바이너리 | 앞 | 뒤 |
| --- | ---: | ---: |
| trss-worker 통합 테스트 | 44.10초 | 13.86초 |
| trss-jobs 통합 테스트 | 41.00초 | 12.08초 |
| trss-web lib | 37.94초 | 9.05초 |
| trss-collect lib | 18.93초 | 1.83초 |
| 테스트 바이너리 31개의 합 | 185.9초 | 65.6초 |
| 걸린 시간(doc test 포함) | 188.8초 | 68.8초 |
| 통과·실패·무시 | 2,755·0·14 | 2,756·0·14 |

- 뒤에 하나 많은 통과는 더한 테스트 `a_new_database_holds_what_the_migrations_make_of_an_empty_file`이에요. 빈 파일을 마이그레이션한 DB와, template에서 받은 파일 DB와 `:memory:` DB의 `user_version`, schema, 모든 표의 행이 같은지 견줘요.
- 뒤의 musl 실행은 처음에 trss-web의 `the_holder_shown_is_the_first_subscription_by_rule_id_whatever_the_listing_order`에서 한 번 실패했어요. 무작위 rule ID 64개 중 첫째가 가장 작으면 실패하는 테스트라서(1/64로 추정), 이 변경과 관계가 없어요. 다시 돌린 값을 위에 적었어요. 그 테스트를 고치는 일은 따로 남겨요.
- 새 DB가 정말 template에서 오는지는 trss-core에서만 확인했어요. 복사나 link가 실패하면 panic하도록 잠깐 바꿔도 trss-core lib 테스트 133개가 모두 통과했어요.

마이그레이션을 확인하는 테스트는 여전히 빈 DB나 옛 DB에서 마이그레이션을 돌려요.

- trss-core `db.rs`의 `empty_db_is_migrated_once_and_reopen_keeps_it`와 `the_rule_start_triggers_survive_every_migration`은 빈 파일을 먼저 만들어서 연 뒤 마이그레이션 62개를 돌려요.
- `database_at`으로 옛 schema의 DB를 만들고 여는 테스트는 이미 있는 파일을 열어서 지금처럼 마이그레이션해요. `db.rs`의 테스트 40개(`database_from_before_*`·`a_database_from_before_*` 25개와 `the_same_number_migration_keeps_every_index_and_trigger` 같은 마이그레이션별 테스트 15개), 그리고 trss-collect(`collect_folder_migration_tests.rs`, `store/history/tests.rs`, `store/status/tests.rs`), trss-library(`store/seasons/tests.rs`, `store/artwork/tests.rs`), trss-web(`setup_api/tests.rs`), trss-jobs(`tests/it/`의 `placement`, `choose`, `place`, `drive_fonts`, `replace`), trss-worker(`tests/it/collect_folder.rs`)의 마이그레이션 사례가 여기에 들어요.

제품 빌드에는 테스트 전용 경로가 들어가지 않아요.

- `cargo tree --locked -e features,normal`로 `--workspace`, `-p trss-web`, `-p trss-worker`, `-p trss-probe -p trss-jobs`(이미지의 두 빌드), `-p trss-browser`를 봤어요. trss-core의 `test-support`도, rusqlite의 `backup`도 켜지지 않아요. dev 의존까지 보면 `test-support`가 6줄, `backup`이 1줄 잡혀서, 질의가 이것들을 찾는다는 것도 확인했어요.
- release로 빌드한 `trss-web`에는 template 코드의 문자열(`migrate the template`, `.template-`)이 없고, trss-core의 테스트 바이너리에는 있어요.

0092의 기준값은 이 변경 뒤에 재므로, 이 효과는 리팩터링의 앞뒤 비교에 들어가지 않아요. 리팩터링 전체의 효과를 볼 때는 이 표를 함께 봐요.

### 함께 견준 다른 방법

musl 테스트를 빠르게 하려고 같은 날 두 가지를 먼저 측정했지만 효과가 없어서 넣지 않았어요(`7f1aebb`, musl).

- SQLite lookaside를 키우는 것(`LIBSQLITE3_FLAGS`의 `-DSQLITE_DEFAULT_LOOKASIDE`)이에요. 기본값, 1200×200, 1200×1000, 2400×1000에서 trss-core lib 테스트가 모두 8.26–8.37초였어요(9.58초 한 번은 튄 값). 플래그가 빌드에 들어간 것은 바이너리의 compile option으로 확인했어요. SQLite는 `CREATE`와 `ALTER TABLE … ADD`를 parse하는 동안 lookaside를 쓰지 않아서, 마이그레이션의 할당은 크기와 관계없이 malloc으로 가요(SQLite 3.53.2 소스의 `disableLookaside`). trss-core에서만 측정했어요.
- 테스트 스레드를 줄이는 것(`RUST_TEST_THREADS`)이에요. workspace의 테스트 바이너리 시간 합이 12개(기본)에서 185.8초, 4개에서 181.9초, 2개에서 228.8초였어요. 할당이 malloc lock 하나에서 줄을 서는 동안은 스레드를 줄여도 처리량이 늘지 않았어요.
