# 0096 파일과 경로의 작은 도우미와 폴더 검사를 모아요

- 상태: 완료 (2026-10-08)
- 출처: [흩어진 개념 모으기](refactoring.md#흩어진-개념-모으기), [ADR 0016](../adr/0016-shared-parts-in-core.md)
- 막는 티켓: [0092](0092-refactor-baseline.md)(기준값)

## 작업

2026-10-07에 같은 일을 하는 작은 도우미가 여러 크레이트에 있었어요. 쓰는 곳들의 가장 아래 크레이트에 하나씩 둬요.

| 도우미 | 지금 | 다른 점 |
| --- | --- | --- |
| 파일이 있는지(없으면 `false`) | trss-collect 2곳, trss-jobs 1곳 | 없어요. |
| 기록한 파일 지우기 | trss-jobs 2곳 | 없어요. |
| 16진수 표기 | trss-jobs, trss-library 2곳, trss-archive | trss-jobs의 수정본 재확인은 8바이트만 써서 다른 쓰임이에요. |
| Transmission이 받는 중인 `.part` | trss-library, trss-collect 2곳 | trss-library만 대소문자를 가리지 않고, 앞부분이 빈 이름을 받지 않아요. |
| 파일 이름 다듬기 | trss-jobs의 `safe_name`, 서버 브라우저, 올리기, WinPNG | 보이지 않는 글자를 지우는 규칙(`700b612`)은 trss-jobs에만 있어요. 서버 브라우저의 이름은 trss-jobs가 다시 다듬어요. |
| 앱 데이터 폴더의 이름 | trss-core의 쓰기 검사 목록과, 실제 주인인 trss-jobs·trss-library, trss-web | 목록이 같은지를 worker 테스트 하나가 지켜요. |
| 경로를 글자로 정규화 | trss-core의 폴더 잠금, trss-collect의 규칙 보관, trss-library의 표지 | 앞 둘은 같고, 표지는 뿌리 위의 `..`를 버려요. |
| 폴더 검사(절대 경로, 있음, 폴더임, 겹침, 같은 장치) | trss-web의 설정 API와 감시 폴더 API, trss-collect의 작업 폴더 | 웹의 두 `normalize`는 글자까지 같아요. 웹은 "같은 폴더"와 "안에 든 폴더"를 따로 알리고, worker는 한 문구로 알려요. |

- 다른 점이 동작으로 드러나는 것(`.part`의 대소문자, 뿌리 위의 `..`, 폴더 검사의 문구)은 맞는 쪽을 정해 결과 절에 적고, 동작이 바뀌면 정리 커밋과 따로 커밋해요.
- 웹의 폴더 사전 검사는 worker의 실행 검사를 대신하지 않아요([모듈 구성](../specs/web-app.md#공통-라이브러리의-모듈-구성)). 같은 검사 함수를 쓰더라도 worker는 실행할 때 다시 검사해요.
- 서버 브라우저의 이름 다듬기가 두 단계 설계로 의도된 것이면 남기고, 그렇지 않으면 trss-jobs의 규칙 하나로 모아요.

## 완료 기준

- 표의 도우미가 각각 한 곳에 있고, 예전 복사본은 없어요.
- 기존 테스트가 고치지 않고 통과해요.
- 다른 점마다 정한 쪽과 까닭이 결과 절에 있어요.

## 결과

### 모은 곳

| 도우미 | 지금 | 지운 복사본 |
| --- | --- | --- |
| 파일이 있는지 | trss-core `files::occupied`. 링크는 링크 자체로 봐요. | trss-collect `revisions`·`episode_undo`의 `exists`, trss-jobs `place::files::occupied` |
| 기록한 파일 지우기 | trss-jobs `place/files.rs`의 `remove_known`. trss-jobs만 써서 trss-jobs에 둬요. | 작업 실행기(`runner.rs`)의 같은 복사본 |
| 16진수 표기 | trss-core `files::hex`. trss-jobs의 `area::hex`는 테스트가 많이 불러서 다시 내보내기로 남아요. 수정본 재확인은 지금처럼 8바이트만 넘겨요. | trss-jobs `area::hex`의 몸통, trss-library의 2벌(`sha256_hex`, 쪽 커서) |
| Transmission이 받는 중인 `.part` | trss-core `files::part_name`·`without_part`. trss-library와 trss-collect가 함께 의존하는 가장 아래가 trss-core예요. | trss-library `discovery`, trss-collect `past_search/world`, `rule_archive/work_folder`의 판단 |
| 앱 데이터 폴더의 이름 | trss-core `app_data`(`RECEIVE_DIR`, `ARTWORK_DIR`, `ARTWORK_STAGING_DIR`, `SUBTITLE_FILES_DIR`, `SUBTITLE_FILES_TEMP_DIR`). 주인인 trss-jobs·trss-library와 trss-web, trss-core의 쓰기 검사 목록(`access::APP_DATA_FOLDERS`)이 이 이름을 써요. | 각 크레이트가 따로 쓴 글자 |
| 경로를 글자로 정규화 | trss-core `folders::lexical` | 폴더 잠금, trss-collect 규칙 보관, trss-library 표지의 `lexical` |
| 폴더 검사 | trss-core `folder_check`. `check_folder`·`resolve_folder`가 어느 조건에서 막혔는지(`Problem`), `overlap`·`conflict`가 어떻게 겹치는지(`Same`, `FirstInsideSecond`, `SecondInsideFirst`, `DifferentDevice`)를 돌려줘요. | trss-web 설정 API와 감시 폴더 API의 검사와 두 `normalize`(하나만 남김), trss-collect 작업 폴더의 검사, trss-library 자동 감시 폴더의 겹침 비교 |

- 문구는 부르는 곳마다 그대로예요. 웹은 "같은 폴더"와 "안에 있어요"를 따로 알리고, 규칙 보관 이동은 한 문구로 알려요. worker는 실행할 때 `resolve_folder`로 다시 검사하고, 다른 파일 시스템을 흉내 내는 테스트를 위해 장치 번호 함수를 넘겨받아요.
- trss-archive는 16진수 표기를 3줄짜리 자기 복사본으로 가져요. trss-archive는 정적인 trss-probe 안에서 자식 프로세스로 도는데, trss-core에 의존하면 rusqlite와 tokio가 딸려 와요. trss-archive와 trss-probe가 함께 의존하는 trss 크레이트는 trss-archive 자신뿐이에요.
- worker 테스트 `app_data_folders_cover_the_receive_area_the_artwork_and_the_subtitle_files`는 이제 양쪽이 같은 상수라서 실패할 수 없어요. 고치지 않고 두었고, [0113](0113-remaining-area-tests.md)에서 지워요.
- `/proc/net/route` 읽기는 이 티켓의 표에 없어서 건드리지 않았어요.

### 다른 점마다 정한 것

- **`.part`의 대소문자**: Transmission의 실제 이름 규칙인, 비지 않은 이름 뒤의 소문자 `.part` 하나로 맞췄어요. Transmission은 언제나 소문자로 쓰므로 대소문자를 가리지 않던 trss-library 쪽이 예외였어요. 그래서 trss-library는 이제 `X.PART`를 받는 중으로 보지 않아요. 동작 변경이라서 따로 커밋했어요(`6e9063e`, `fix(library): count only Transmission's lower-case .part as a download in progress`).
- **뿌리 위의 `..`**: 셋 모두 버려요. 폴더 잠금과 규칙 보관이 정규화하는 것은 절대 경로이고, POSIX에서 `/..`는 `/`이므로 `/../x`와 `/x`는 같은 폴더예요. `..`를 남기면 한 폴더에 두 철자가 생겨 잠금이 서로를 보지 못했어요. 표지의 상대 경로에는 원래 하던 가둠이에요. 따로 커밋했어요(`fix(core): treat /../x and /x as one folder in folder locks and rule paths`, 이 결과를 함께 담은 커밋).
  - 상대 경로가 들어오는 곳을 찾아봤어요. 수집 폴더와 보관 폴더는 설정 API와 가져오기 API에서 절대 경로 검사를 거치고, 규칙 폴더는 그 수집 폴더 아래에 붙어요. 수정본 기록의 폴더는 Transmission의 절대 경로예요.
  - 예전 `fold_bases` 마이그레이션 전에 저장된 값에만 앞에 `..`가 붙은 상대 경로가 있을 수 있어요. 그런 값이라도 잠금 키가 넓어질 뿐, 가리키는 폴더는 바뀌지 않아요.
- **폴더 검사의 문구**: 바꾸지 않았어요. 검사는 한 곳에서 하고, 문구는 부르는 곳이 정해요.
- **서버 브라우저의 파일 이름 다듬기**: 의도한 두 단계 설계라서 남겼어요.
  - trss-browser의 `safe_file_name`은 브라우저의 내려받기 폴더를 믿지 않기로 한 `466b393`에서 생겼어요. 경로의 마지막 조각만 남기고 점으로 시작하는 이름을 만들지 않아서, 컨테이너 안에서 job 폴더로 옮기는 파일을 지켜요. trss-subtitles의 `auth.rs`는 "내려받은 파일 이름은 점으로 시작하지 않는다"에 기대어 답 파일과 내려받은 파일을 가려요.
  - trss-jobs의 `safe_name`(`700b612`의 보이지 않는 글자 규칙 포함)은 이름이 받은 기록이나 배치의 이름이 될 때 다시 다듬어요(`runner.rs`의 받기, `upload.rs`). 브라우저를 거치지 않는 Drive, HTTP, 올리기의 이름도 여기서 다듬어요.
  - 두 규칙은 보통 이름에서도 달라요(브라우저는 마지막 조각, trss-jobs는 `/`를 `_`로). 하나로 바꾸면 디스크의 이름이 바뀌고, 크레이트 의존 방향(trss-jobs → trss-subtitles → trss-browser)도 맞지 않아요.
  - 표의 "4벌" 중 올리기와 표시 이름은 이미 `safe_name`을 불렀어요. WinPNG의 `same_name`은 다듬기가 아니라 견주기 위한 접기라서 두었어요.

### 남은 것

- 작업 실행기의 `Waited::File`이 지닌 이름은 브라우저가 다듬은 이름이고, 작업 사건의 내용에 trss-jobs의 보이지 않는 글자 규칙 없이 들어가요. 받은 기록의 이름은 그 뒤에 다듬어서, 영향은 사건 한 줄의 글자뿐이에요. 바꾸지 않았어요.

### 검증한 것

2026-10-08에 `cargo test --locked --workspace -j 4`로 확인했어요. 기존 테스트는 고치지 않았어요.

| 커밋 | 통과 | 실패 | 무시 |
| --- | ---: | ---: | ---: |
| 시작 | 2,832 | 0 | 14 |
| `2adf9e4` 작은 파일 도우미 | 2,837 | 0 | 14 |
| `c8d915d` 앱 데이터 폴더 이름 | 2,839 | 0 | 14 |
| `c5702f9` 경로 정규화 | 2,841 | 0 | 14 |
| `0b99139` 폴더 검사 | 2,846 | 0 | 14 |
| `6e9063e` `.part` | 2,847 | 0 | 14 |
| 뿌리 위의 `..`(이 결과를 담은 커밋) | 2,848 | 0 | 14 |

- 두 수정 커밋의 테스트는 고치기 전에 실패하고 고친 뒤 통과했어요.
- trss-library 자동 감시 폴더의 겹침 비교는 `0b99139`에 나중에 합쳤어요. 합친 뒤 `cargo test -p trss-library automatic`(19개)이 통과했고, 그 안의 테스트가 두 방향의 거절 문구를 확인해요.
- `0b99139` 뒤의 한 번에서 trss-browser의 `launcher::a_chromium_that_does_not_start_leaves_nothing_behind`가 한 번 실패했어요(200과 504). 따로 세 번과 workspace 다시 돌리기에서 통과했고, 이 티켓은 trss-browser를 바꾸지 않았어요.
