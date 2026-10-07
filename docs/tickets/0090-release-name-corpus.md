# 0090 실제 릴리스 이름 묶음을 만들어요

- 상태: 완료 (2026-10-07)
- 출처: [테스트 나눔 ADR](../adr/0015-test-a-rule-once-in-its-crate.md), [영상 회차 변환](../specs/collection.md#영상-회차-변환), [영상 수정본의 대체](../specs/collection.md#영상-수정본의-대체)
- 막는 티켓: [0080](0080-server-db-migration-check.md)(서버 DB 복사본)

## 작업

ADR 0015는 릴리스 이름과 파일 이름의 해석을 실제 입력 묶음으로 테스트하기로 했어요.
2026-10-07 구조 조사에서 이 묶음이 배포 전에도 필요해졌어요. trss는 받은 영상의 이름을 trname으로 바꾸는데, trname이 세 자리 회차와 `.5` 회차를 잘못 읽어요([0091](0091-trname-long-and-half-episodes.md)).
지금 테스트의 입력은 거의 두 자리 회차라서 이 문제가 보이지 않았어요.

- 이름은 두 곳에서 모아요. 0080의 서버 DB 복사본에 있는 수집 이력, 그리고 공개 피드예요.
- 저장소가 공개돼 있으므로 수집 이력에서는 이름만 넣어요. 주소, 해시, 폴더, 규칙은 넣지 않아요.
- 이름마다 기대하는 작품 이름, 회차, 수정 번호를 함께 적어요. trname과 trss-collect의 읽기가 같으면 그 값을 기대값으로 삼아요. 다르면 제가 이름을 보고 정하고, 정하지 못한 이름은 사용자에게 물어요.
- 묶음을 도는 테스트 하나가 trname과 trss-collect의 읽기를 기대값과 견줘요. 지금 틀리는 이름은 알려진 실패로 표시해 두고, 0091과 뒤의 리팩터링에서 고칠 때 표시를 지워요.

## 완료 기준

- 묶음 파일이 저장소에 있고, 이름에 주소, 해시, 폴더, 규칙이 없어요. 수집 이력에서 온 이름과 공개 피드에서 온 이름의 개수를 결과 절에 적어요.
- 묶음 테스트가 workspace 테스트에서 돌아요. 기대값과 다른 읽기는 알려진 실패 목록에 있고, 그 밖의 이름은 모두 통과해요.
- 결과 절에 읽기가 틀리는 이름을 종류별로 묶어 적어요. 세 자리 회차, `.5` 회차, CRC를 회차로 읽음처럼 나누고, 수집 이력에 실제로 있었던 것인지도 적어요.
- 수집 이력에서 trname이 잘못 읽은 이름으로 실제 파일 이름을 바꾼 기록이 있으면 그 목록을 결과 절에 적어요. 이미 바뀐 파일을 고칠지는 사용자 결정을 받아요.

## 결과

### 만든 것 (2026-10-07)

- 묶음은 `crates/trss-collect/tests/fixtures/release-names.tsv`에 있어요. 이름 766개에 각각 출처, 작품 이름, 회차(`12`, `12.5`, `batch 1-12`, 없음 `-`), 수정 번호, 알려진 실패를 적었어요.
- trss-collect의 `release_names::every_release_name_reads_as_it_means_but_the_known_failures`가 묶음을 돌아요. 이 크레이트의 읽기(`parse_release`의 작품, `past_search::release::read`의 회차와 수정 번호)와, worker가 영상 파일 이름을 바꿀 때 trname이 읽는 회차(`derived_name`, `<작품>/Season 01`, 변환 없음)를 기대값과 견줘요. trname은 영상 파일 이름(`.mkv`, `.mp4`)에만 견줘요. 다른 이름은 피드 제목이라 trname에 가지 않아요.
- 기대값과 다른 읽기가 알려진 실패 목록과 다르면 실패해요. 새로 틀린 읽기와, 고쳐졌는데 표시가 남은 읽기 둘 다예요.
- 무시된 테스트 `print_lines`는 새 이름의 줄을 이 파일의 꼴로 찍어요. 이 크레이트의 읽기를 기대값으로 두므로, 줄마다 이름을 보고 확인해야 해요.

### 모은 이름

| 출처 | 개수 | 내용 |
| --- | ---: | --- |
| 수집 이력 | 368 | [0080](0080-server-db-migration-check.md)의 서버 DB 복사본에 있는 `history_items`의 서로 다른 제목 전부. Erai-raws 채널 256개, SubsPlease 112개예요. |
| 공개 피드 | 398 | 2026-10-07에 읽은 SubsPlease RSS(1080p)와 nyaa.si RSS예요. nyaa는 최신 목록과 `one piece 1080p`, `detective conan`, `Ioroid`, `Beatrice-Raws`, `".5 (1080p)"` 검색이에요. 수집 이력과 겹치는 이름은 수집 이력으로 셌어요. |

- 수집 이력에서는 제목만 넣었어요. 주소, 해시, 폴더, 규칙은 없어요. 묶음 파일에 `http`, `magnet:`, 40자리 해시, `/downloads`가 없는 것을 확인했어요.
- nyaa의 `Moozzi2` 검색 48개는 뺐어요. 대부분 자막·폰트 묶음(`.zip`, `.7z`)과 다른 그룹의 BD 묶음이라, 영상 파일 이름이 하나도 없었어요.
- 뜻을 이름만으로 정할 수 없는 5개는 뺐어요(사용자 결정, 2026-10-07). Beatrice-Raws의 `_rev`, `_rev2`, `_rev3` 4개는 수정 번호가 몇인지 알 수 없어요. `Yu-Gi-Oh! Go Rush!! Episode 52 - 67 [English Dub]`은 52–67화 묶음인지 알 수 없어요.
- 세 자리 이상 회차는 146개(수집 이력 11), `.5` 회차는 46개(수집 이력 1)예요.

### 기대값을 정한 방법

trss-collect의 읽기에서 출발해서, 이름을 보고 틀렸다고 판단한 것만 고쳤어요.
영상 파일 이름 159개 중 두 읽기의 회차가 다른 34개는 모두 이름을 보고 정했어요.
두 읽기가 같아도 틀린 것이 있었어요. 명탐정 코난 극장판 `Detective Conan - The Counterfeit Case of Ultra 30`은 둘 다 30화로 읽지만, 번호 없는 이름으로 정했어요.
작품 이름은 이름이 쓴 작품 제목이에요(규칙의 일치 문구). 그룹 태그, 회차, 해상도 같은 기술 정보는 빼고, 시즌 표기(`2nd Season`, `S2`)와 제목 안의 괄호(`(2026)`, `(Hokori)!`)는 남겨요.

### 읽기가 틀리는 이름

알려진 실패가 있는 이름은 101개(수집 이력 6, 공개 피드 95)예요.

| 읽기 | 종류 | 수집 이력 | 공개 피드 | 예 |
| --- | --- | ---: | ---: | --- |
| trname | 세·네 자리 회차를 앞 두 자리로 읽음 | 4 | 19 | `[SubsPlease] One Piece - 1180 (1080p) [97492A4D].mkv` → 11 |
| trname | 번호 없는 이름(극장판, BD)에서 해상도나 CRC의 숫자를 회차로 읽음 | 0 | 6 | `[Beatrice-Raws] Magic Tree House [BDRip 1920x1080 HEVC FLAC].mkv` → 80, `… [2F959F75].mkv` → 75 |
| trname | 제목 안의 숫자를 회차로 읽음 | 1 | 2 | `Detective Conan - The Counterfeit Case of Ultra 30` → 30 |
| trss-collect | 제목 안의 숫자(극장판 부제, 시즌 번호)를 회차로 읽음 | 1 | 8 | 위의 코난 극장판, `[Beatrice-Raws] Yuru Yuri 2 [BDRip …]` → 2, `[HuangSubs] Slay the Gods Season 2 (1080p)` → 2 |
| trss-collect | Erai-raws의 `(V2)` 수정 표시를 읽지 못함 | 1 | 0 | `[Magnet] Kusuriya no Hitorigoto 3rd Season - 01 (V2) […]` → 수정 번호 1 |
| trss-collect | `EP1180` 표기의 회차를 읽지 못하고, 작품 이름에 나머지가 다 들어감 | 0 | 26 | `[ToonsHub] One Piece EP1180 1080p CR WEB-DL …` |
| trss-collect | 대시 없는 네 자리 회차를 읽지 못함 | 0 | 5 | `[HatSubs] One Piece 1180 (WEB 1080p) [C2F305AA].mkv` |
| trss-collect | `第01话` 표기의 회차를 읽지 못함 | 0 | 1 | `[Doomdos] - … - 第01话 - […]` |
| trss-collect | `작품 - S01E01`의 ` -`가 작품 이름에 남음 | 0 | 16 | `[DKB] Chii Fuyo - S01E01 […]` → `Chii Fuyo -` |
| trss-collect | 이름 앞의 `- `가 작품 이름에 남음 | 0 | 8 | `[Doomdos] - Mononoke - 01 […]` → `- Mononoke` |
| trss-collect | `Episode`가 작품 이름에 남음 | 0 | 2 | `Beyblade X - Episode 130 […]` → `Beyblade X - Episode` |
| trss-collect | 해상도 같은 기술 정보가 작품 이름에 남음 | 0 | 2 | `[PrinceOtaku] … (2024) WEB.AMZN 1080p [Dual Audio]` |
| trss-collect | 괄호 없는 범위(`01-26`)를 묶음으로 읽지 못함 | 0 | 2 | `Detective Conan Movies 01-26 (Only subs) […]` |

코난 극장판의 SubsPlease 이름 3개는 trname과 trss-collect 둘 다 틀려서 두 줄에 모두 셌어요.
`.5` 회차의 영상 파일 이름 45개는 모두 SubsPlease 이름이고, trname이 모두 맞게 읽어요. [0091](0091-trname-long-and-half-episodes.md)의 `[Group] Show - 05.5 (1080p).mkv`처럼 SubsPlease가 아닌 `.5` 이름은 묶음에 없어요.

### 잘못 바뀐 파일 이름

없었어요.

- 수집 이력에서 받은 항목은 5개예요. 그중 trname이 틀리게 읽는 이름은 코난 극장판 하나뿐이에요. 이 항목은 한 번 받기로 `Season NN`이 아닌 폴더에 받아서, trname이 이름을 바꾸지 않는 경로였어요.
- 서버 미디어 폴더의 파일 이름(0080의 복사본)에 세 자리 회차 작품의 폴더가 없고, `S01E80`처럼 해상도에서 나온 회차 이름도 없었어요.

### 검증한 것

- 2026-10-07 `cargo test -j 4 -p trss-collect --lib release_names`에서 묶음 테스트가 통과했어요(766개, 0.70 s).
- 알려진 실패 표시 하나(`One Piece - 1180`의 `trname`)를 지우고, 맞게 읽는 이름 하나의 기대 회차를 바꾸자 테스트가 그 두 이름을 이름과 읽기와 함께 보여주며 실패했어요.

### 남은 일

- trname의 세 종류는 [0091](0091-trname-long-and-half-episodes.md)에서 고쳐요.
- trss-collect의 종류는 맡은 티켓이 없어요. [0098](0098-release-name-reading.md)은 읽기를 동작 그대로 모으는 일이에요. 이 중 Erai-raws의 `(V2)`는 사용자가 쓰는 채널에서 나왔어요. 규칙에 맞는 작품의 `(V2)` 항목은 수정본으로 읽히지 않아서, [영상 수정본의 대체](../specs/collection.md#영상-수정본의-대체)의 판정(이름에 CRC가 없는 수정본은 `버전 미상`)을 거치지 않아요. 수집 이력의 그 항목은 규칙에 맞지 않았어요(`no_match`).
