# 0010 채널 기본 경로를 앱 전체의 수집 폴더로 바꿔요

- 상태: 완료
- 출처: [수집 폴더와 보관 폴더](../specs/collection.md#수집-폴더와-보관-폴더), [설정 화면](../specs/settings.md#설정-화면), [기존 YAML](../specs/settings.md#기존-yaml)
- 막는 티켓: 없음

## 작업

채널마다 두던 기본 경로(`channels.base_dir`)를 앱 전체의 `수집 폴더` 하나로 바꾸고, 짝이 되는 `보관 폴더`를 설정할 자리를 만들어요.
[0011](0011-archive-folder-move.md)의 보관 이동이 "작품 폴더가 어디서 어디로 가는지"를 하나의 기준으로 알 수 있게 하는 준비예요.
이 티켓만으로는 파일을 옮기지 않아요.

- 규칙이 받는 위치는 `수집 폴더 + 규칙 저장 폴더`이고, 규칙 주기·미리보기·`다시 받기`가 모두 같은 계산(`rss::save_path`)을 써요.
- 기존 DB는 마이그레이션으로 옮겨요. 모든 채널의 기본 경로가 같으면 그 경로가 수집 폴더가 되고, 다르면 공통 상위 폴더를 수집 폴더로 두고 각 규칙의 저장 폴더 앞에 남은 부분을 붙여요. 어느 경우에도 규칙마다 실제로 받는 위치가 한 글자도 바뀌지 않아야 해요.
- 설정 화면에 `수집 폴더` 항목을 두고 두 폴더를 정해요. 보관 폴더는 비워 둘 수 있어요.
  웹은 미디어를 읽기 전용으로 보므로, 웹이 확인하는 것은 있는 폴더인지, 같은 폴더이거나 한쪽이 다른 쪽 안에 있지 않은지, 같은 파일시스템인지예요. 쓰기 가능 여부는 worker가 쓸 때 드러나요.
- 채널 탭에서 기본 저장 폴더의 표시와 편집을 빼고, 채널 API도 그 필드를 받지 않아요.
- 기존 YAML 가져오기는 [기존 YAML](../specs/settings.md#기존-yaml)의 채널 `directory` 처리를 따라요(수집 폴더가 없으면 정하고, 안이면 규칙 폴더 앞에 붙이고, 밖이면 까닭과 함께 가져오지 않음).
- 수집 폴더가 비어 있는 새 DB(채널이 없던 DB)에서는 worker가 토렌트를 추가하지 않고, 상태 판에 수집 폴더를 정하라는 까닭을 보여줘요. 이때 항목을 `추가 실패`로 쌓지 않아요.

## 완료 기준

| 입력·상태 | 기대 결과 |
| --- | --- |
| 두 채널 모두 `/downloads/Shows (current)`인 DB를 마이그레이션 | 수집 폴더가 `/downloads/Shows (current)`이고 규칙 저장 폴더는 그대로예요. 모든 규칙의 받는 위치가 마이그레이션 전과 같아요(규칙마다 비교하는 시험). |
| 채널 A `/downloads/Shows (current)`, 채널 B `/downloads/Movies`를 마이그레이션 | 수집 폴더가 `/downloads`이고 A의 규칙은 `Shows (current)/…`, B의 규칙은 `Movies/…`로 시작해요. 받는 위치는 전과 같아요. |
| 마이그레이션 뒤 규칙 주기·미리보기·`다시 받기` | 세 경로가 같은 항목에 같은 폴더를 내요. |
| 설정 화면에서 보관 폴더를 수집 폴더 안의 폴더로 저장 | 까닭과 함께 저장하지 않아요. 없는 폴더도 같아요. |
| 설정 화면에서 보관 폴더를 비워 저장 | 저장되고, 목록 줄에 수집 폴더 이름만 보여요. |
| 채널 탭 | 기본 저장 폴더가 보이지 않고, 받는 폴더가 앱의 수집 폴더라는 한 줄이 있어요. |
| 수집 폴더 밖의 채널 폴더가 든 기존 YAML을 가져오기 | 검토 단계에서 그 채널을 까닭과 함께 알리고 가져오지 않아요. |
| 수집 폴더가 없는 새 DB에서 규칙에 맞는 항목 | Transmission에 추가되지 않고, 상태 판에 까닭이 보이며, 기록에 `추가 실패`가 쌓이지 않아요. |

## 결과

### 구현한 것

- 마이그레이션 7(`crates/trss-core/src/settings/migrate.rs`, 코드 마이그레이션 `Migration::Code`)이 `channels.base_dir`를 수집 폴더로 접고 컬럼을 지워요(`DROP COLUMN`을 골랐어요. 남겨 두면 두 곳이 같은 뜻을 들고 어긋날 수 있어서예요). 같은 트랜잭션에서 `collection_settings`(`id`, `collect_folder`, `archive_folder`, `version`; `crates/trss-core/src/settings/`)을 만들어요.
  - 모든 채널의 기본 경로가 같은 글자이면 그 글자가 수집 폴더이고 규칙은 그대로예요.
  - 다르면 경로 조각 단위의 가장 긴 공통 상위가 수집 폴더이고, 각 채널의 남은 조각이 그 채널 규칙의 저장 폴더 앞에 붙어요. 끝의 `/`(저장 폴더가 빈 규칙)와 절대 경로 저장 폴더도 `Path::join`이 옛 기본 경로와 만들던 글자 그대로 나오도록 조각을 글자 단위로 접어요(`crates/trss-core/src/folders.rs`). 접은 결과는 모든 기본 경로·저장 폴더 쌍에서 옛 경로와 같은지 스스로 검사하고, 어긋나면 마이그레이션이 실패해 DB가 v6으로 남아요.
  - 채널이 없으면 수집 폴더는 정해지지 않아요.
  - 공통 조각이 하나도 없는 상대 경로 기본 경로(`a/x`와 `b/y`)는 접을 수 없어서 마이그레이션이 실패해요(DB는 바뀌지 않아요). 절대 경로는 최악의 경우 `/`가 수집 폴더가 돼요.
- 받는 위치는 `수집 폴더 + 규칙 저장 폴더`이고 규칙 주기(`worker::cycle`), 규칙 미리보기(`web::rules_api`), `다시 받기`(`worker::plan::rule_destination`)가 모두 `rss::save_path`를 써요. `ChannelInput`·`Channel`과 채널 API에서 `base_dir`가 빠졌고, 본문에 `base_dir`가 있으면 모르는 필드로 거부해요.
- `GET`/`PUT /api/settings/collection`(`crates/trss-web/src/settings_api.rs`): `{folder, archive_folder, version}`, 버전이 다르면 `409 conflict`, 그 밖은 `ApiError`의 `invalid`·`internal`과 해요체 문장이에요. 검사는 절대 경로인 있는 폴더, `canonicalize` 뒤 같거나 한쪽이 다른 쪽 안이 아닐 것, 같은 `st_dev`예요. 보관 폴더는 비워도 돼요. 폴더는 적은 그대로(끝의 `/`만 뗌) 저장해요.
- 수집 폴더가 없으면: 주기는 피드를 읽고 규칙이 고르지 않은 항목만 기록하고, 규칙이 고른 항목은 Transmission에 넣지도 기록하지도 않아요(보고서의 `waiting_for_collect_folder`). 기록하지 않으니 폴더를 정한 뒤 다음 주기가 새 항목으로 판정해 받아요. 토렌트 정리도 건너뛰어요. `다시 받기` 명령은 "수집 폴더가 정해지지 않아서 받지 않았어요"라는 까닭으로 `failed`로 끝나고 Transmission에 아무것도 보내지 않으며, 항목의 기록은 바꾸지 않아요. `/api/collect/status`에 `collect_folder_set`이 있고 상태 판이 까닭과 설정 링크를 보여줘요.
- 기존 YAML(`crates/trss-import/src/fit.rs`, `crates/trss-web/src/import_api.rs`): 수집 폴더가 없으면 파일의 채널 폴더(다르면 공통 상위)로 정하고, 안이면 규칙 저장 폴더 앞에 남은 부분을 붙이고, 밖이면 검토 단계에 까닭과 함께 알리고 가져오지 않아요(결과 단계에도 나와요). 폴더는 채널과 같은 트랜잭션으로 정해요. 검토한 수집 폴더가 적용 때 다르면 `409`로 다시 검토하게 해요.
- 웹: 설정 `수집 폴더` 항목(폴더 두 칸·저장·되돌리기·충돌 안내), 목록 줄은 `Shows (current) · 보관 Shows` 또는 수집 폴더 이름만, 채널 카드·편집기에서 기본 저장 폴더를 빼고 채널 탭에 설정 링크가 든 한 줄, 규칙 상세가 전체 경로를 보여주고, 폴더를 바꾸면 규칙·미리보기·상태 캐시를 버려요. `readme.md`와 `docker-compose.trss.yml` 주석, `worker`·`channels_api` 모듈 문서를 고쳤어요.

### 검토 뒤 고친 것

- 기존 YAML이 `..`로 수집 폴더 밖으로 나가는 채널 폴더(`/downloads/Shows/../Movies`)를 안으로 잘못 보던 것을 고쳤어요. `..`가 든 채널 폴더는 까닭과 함께 가져오지 않고, 수집 폴더를 정할 때의 공통 상위 계산에서도 빠져요(`relative_under`·`common_ancestor`, `fit`). 시험: `folders::tests::a_rest_with_parent_components_is_not_inside`, `the_common_ancestor_stops_before_a_parent_component`, `import::fit::tests::a_channel_folder_with_parent_components_is_reported_not_placed`, `a_channel_folder_with_parent_components_takes_no_part_in_choosing_the_collect_folder`, `web::import_api::tests::a_channel_folder_that_climbs_out_of_the_collect_folder_is_not_imported`.
- 마이그레이션 7이 접지 못하는 까닭(빈 기본 경로, 공통 상위 없음, 글자 그대로 접을 수 없는 `//` 끝 등)을 나눠 말하고 문제의 기본 경로와 채널 ID를 오류에 적어요. DB가 v6으로 남는 것은 그대로예요(`folders::FoldError`, `store::settings::migrate`). 시험: `folders::tests::folding_says_which_bases_it_refused_and_why`, `store::settings::tests::folders_with_no_common_parent_stop_the_migration_and_change_nothing`, `a_base_that_cannot_be_folded_byte_for_byte_is_named_not_blamed_on_the_parent`, `an_empty_base_is_named_with_its_channel`.
- 이미 정한 수집 폴더를 다른 폴더로 바꿔 저장하면, 저장 전에 그 자리에서 확인을 물어요(이미 받은 파일은 그대로, 규칙 저장 폴더는 새 수집 폴더 기준, 앞으로 추가하는 항목은 새 폴더 아래). `CollectionPanel.tsx`의 자동 시험은 없어요. 로컬 `trss-web`(1280px, 스크래치 DB)에서 수집 폴더를 바꿔 저장하자 확인 문장과 `바꾸기`·`취소`가 나오고, `바꾸기` 뒤 `저장했어요.`와 목록 줄 `other · 보관 archive`로 바뀌는 것을 봤어요(2026-10-01).
- 가져오기가 수집 폴더를 새로 정할 때 설정 화면과 같은 검사를 받아요. 정하게 될 폴더가 절대 경로가 아니거나 `/`이면 검토 단계에서 가져오기 전체를 까닭과 함께 거절하고, 웹이 볼 수 있는 폴더인지도 설정의 `check_folders`로 확인해요(수집 폴더가 이미 있으면 검사하지 않아요). 채널 폴더가 모두 상대 경로이면 거절하고, 절대·상대가 섞이면 절대 경로끼리만 공통 상위를 구하고 상대 경로 채널은 밖으로 알려요. 시험: `import::fit::tests::relative_channel_folders_alone_cannot_set_the_collect_folder`, `a_common_ancestor_that_is_the_filesystem_root_is_refused`, `with_no_collect_folder_absolute_folders_win_over_relative_ones`, `web::import_api::tests::a_collect_folder_the_import_would_set_must_be_an_existing_directory`, `an_import_that_would_set_a_relative_or_root_collect_folder_is_refused`. 폴더를 새로 정하는 기존 가져오기 시험은 임시 폴더 아래의 실제 폴더를 쓰도록 바꿨어요.

### 검증한 것

- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`가 모두 통과했어요(라이브러리 296개, 통합 시험 파일들 포함). 웹은 `npm run typecheck`와 `npm run build`가 통과했어요.
- 마이그레이션(`crates/trss-collect/src/store/collect_folder_migration_tests.rs`, 이전 버전 DB를 손으로 만들어 열어요): 같은 기본 경로, 다른 기본 경로(끝의 `/`가 있는 기본 경로·빈 저장 폴더·보관 규칙 포함), 채널 없음, 공통 조각 없음(v6 그대로)을 확인하고, 규칙마다 `save_path`와 글자 단위로 같은지 견줘요.
- `crates/trss-worker/tests/collect_folder.rs`: 마이그레이션한 DB에서 규칙 주기·미리보기·`다시 받기`가 같은 항목에 같은 폴더를 내고 마이그레이션 전 계산과 글자까지 같아요(`/media/other/`의 끝 `/` 포함). 수집 폴더가 없는 DB에서는 추가 0·`add_failed` 0·기록에 `추가 실패` 없음·토렌트 정리 없음·상태 판 `collect_folder_set=false`이고, 폴더를 정하면 다음 주기가 3개를 받아요. 폴더 없는 상태의 `다시 받기`는 까닭과 함께 끝나고 항목을 그대로 둬요.
- 설정 API 시험(`crates/trss-web/src/settings_api/tests.rs`): 읽기·저장·비운 보관 폴더, 없는 폴더·폴더가 아닌 것·상대 경로, 같은 폴더와 양방향 안쪽(`..`과 심볼릭 링크 포함), 다른 파일시스템(`/proc`, Linux 전용), 오래된 버전 `409`, 잘못된 본문.
- 기존 YAML: `crates/trss-import/src/fit.rs`와 `crates/trss-web/src/import_api/tests.rs`에서 폴더 미설정·안·밖·공통 상위·검토 뒤 폴더가 바뀜·모두 건너뜀을 확인해요. `tests/worker_legacy_comparison.rs`는 YAML을 가져오기 경로로 넣은 worker가 옛 실행 파일과 같은 Transmission 요청(같은 폴더)을 내는지 견줘요.
- 브라우저(로컬 `trss-web`, 1280px과 390px, 스크래치 DB): v6 DB를 열어 마이그레이션됨을 확인했고, 설정 화면에서 수집 폴더 안의 보관 폴더가 까닭과 함께 거부되고 바깥 폴더는 저장돼 목록 줄이 `media · 보관 Shows-archive`로 바뀌고, 채널 탭에 기본 저장 폴더가 없고 설정 링크 한 줄이 있고, 규칙 상세가 전체 경로를 보여주고, 빈 DB에서 상태 판이 까닭을 보여주고 설정 패널이 `수집 폴더를 정해 주세요`를 보여주고, 가져오기 검토·결과 단계가 수집 폴더 설정과 밖의 채널을 보여주는 것을 봤어요. 390px에서 가로 넘침이 없었어요(`scrollWidth == innerWidth`).

### 검증하지 못한 것

- 실제 서버의 운영 DB(0.4.1)에 대한 마이그레이션, 실제 Transmission과 배포 이미지·Compose는 시험하지 않았어요(시험은 가짜 Transmission과 임시 DB예요). 배포 전에 `trss.db`를 복사해 두세요. 컬럼을 지우므로 이전 버전으로 되돌릴 수 없어요.
- 읽기 전용 미디어 마운트(`/downloads:ro`) 안에서의 폴더 검사와 `st_dev` 비교는 컨테이너에서 확인하지 못했어요. 다른 파일시스템 시험은 Linux의 `/proc`에 기대요.
- 웹 화면의 자동 시험은 없어요(이 저장소에 웹 시험 도구가 없어요). 화면은 위 브라우저 확인과 타입 검사·빌드로만 봤고, 라이트 모드·키보드 조작은 보지 않았어요.
- 쓰기 가능 여부는 웹이 확인하지 않아요(명세대로). Transmission이 쓰지 못하는 폴더는 추가가 실패할 때 드러나요.

### 남은 일

- 보관 폴더로 작품 폴더를 옮기는 일은 [0011](0011-archive-folder-move.md)이 맡아요. 이 티켓은 보관 폴더를 저장하고 검사하기만 해요.
- 배포 때 실제 DB로 마이그레이션 결과(수집 폴더와 규칙 저장 폴더 앞 조각)를 눈으로 확인해요. 두 채널이 모두 `/downloads/...` 아래이면 수집 폴더는 그 공통 상위예요.
