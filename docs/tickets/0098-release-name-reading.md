# 0098 릴리스 이름과 회차 읽기를 모아요

- 상태: 완료 (2026-10-08)
- 출처: [흩어진 개념 모으기](refactoring.md#흩어진-개념-모으기), [수집 받기 줄기의 재구성](refactoring.md#수집-받기-줄기의-재구성), [영상 회차 변환](../specs/collection.md#영상-회차-변환)
- 막는 티켓: [0090](../archive/tickets/5-deployed-verification/0090-release-name-corpus.md)(이름 묶음), [0091](../archive/tickets/5-deployed-verification/0091-trname-long-and-half-episodes.md)(trname 수정), [0092](0092-refactor-baseline.md)(기준값)

## 작업

2026-10-07에 릴리스 이름을 읽는 곳이 trss-collect 안에 3곳 있었고, trname이 따로 읽었어요.

- `revision`의 `Release::parse`와 `without_version`은 이름, 수정 번호(`NvM`), CRC를 읽어요.
- `subscriptions`의 `parse_release`와 `whole_episode`는 작품과 회차를 읽어요.
- `past_search/release`의 `read`는 위 둘을 합치고 정규식을 더해요.
- 읽는 범위가 서로 달라요. 확장자(`.[A-Za-z0-9]{2,4}`와 `mkv|mp4|avi|torrent`), 수정 번호의 자릿수(`v\d+`와 `v\d{1,2}`), 시즌 표기(`S\d{1,2}E`와 `S\d{1,3}E`), `03-03`을 묶음으로 볼지가 달라요.

trname의 회차 변환 계산은 trss-collect에 3벌 더 있어요(`judge`의 `folder_episode`, `range`, `episode_offset`의 "0과 1은 변환 없음").
trname이 쓴 이름(`<작품> SxxEyy.ext`)과 시즌 폴더를 읽는 곳도 trss-collect, trss-library, trss-transmission, trname의 4곳이고, 자릿수와 `S` 앞에 올 수 있는 글자가 달라요.

- 릴리스 이름 읽기는 trss-collect의 한 모듈로 모아, 하나의 결과 구조를 돌려줘요.
- trname이 쓴 이름과 시즌 폴더 읽기는 trss-core에 둬요. trss-transmission은 trss-core를 부를 수 있어요.
- 받은 영상의 회차를 예상할 때는 trname의 읽기와 변환을 그대로 써서, 예상과 실제 파일 이름이 같은 규칙에서 나와요.
  이 요구는 사용자 결정(2026-10-08)으로 바뀌었어요. 예상은 trss-collect의 읽기로 하고, 묶음 테스트가 예상과 실제 이름이 같은지 확인해요. 결과 절을 봐요.
- trname은 번호 없는 영상 이름(극장판, BD)에서 해상도나 CRC의 숫자를 회차로 읽어요(0090의 묶음에서 공개 피드 6개). trname의 마지막 규칙은 CLI로 손수 바꾸는 자막 이름(`nogame01.ass`)을 읽으므로 trname은 그대로 두고, trss-collect가 회차 없음이나 묶음으로 읽는 이름은 trname으로 바꾸지 않고 받은 이름을 남겨요(사용자 결정, 2026-10-07). 동작 변경이므로 정리 커밋과 따로 커밋해요.
- [0090](../archive/tickets/5-deployed-verification/0090-release-name-corpus.md)의 묶음 테스트가 읽기가 바뀌지 않았는지 지켜요. 모은 뒤 묶음의 어느 이름이라도 읽는 결과가 달라지면, 그것은 동작 변경이에요. 맞는 쪽을 정해 결과 절에 적고 정리 커밋과 따로 커밋해요.

## 완료 기준

- 위의 읽기가 각각 한 곳에 있고, 예전 복사본은 없어요.
- 묶음 테스트와 기존 테스트가 고치지 않고 통과해요.
- 묶음 테스트가 worker가 실제로 붙이는 이름을 견주고, trname이 번호 없는 영상 이름에서 숫자를 회차로 읽던 줄(2026-10-07에 6개)의 알려진 실패 표시가 없어요.
- 그 뒤 이름 읽기의 예제 테스트(2026-10-07에 약 35개)를 묶음 테스트로 정리해요. 이름을 확인하는 worker의 프로세스 테스트는 남겨요. 앞뒤 개수를 결과 절에 적어요.

## 결과

### 결정

- **회차 예상**: trss-collect의 읽기와 회차 변환으로 예상하고, 묶음 테스트가 영상 파일 이름마다 예상한 회차와 worker가 붙이는 이름의 회차가 같은지 확인해요(사용자 결정, 2026-10-08). 작업 절의 "trname의 읽기와 변환을 그대로" 대신이에요.
  - 지난 회차 검색이 판단하는 피드 제목은 대개 확장자가 없어 trname이 읽지 못해요. `.mkv`를 붙여 읽혀도 묶음의 제목 65개를 틀리게 읽었어요(`Beyblade X - Episode 130 [720p]` → 20).
  - 작품 폴더가 `<작품>/Season NN`이 아닌 규칙은 trname이 시즌을 정하지 못해 예상이 모두 사라져요.
  - 아래의 이름 규칙 뒤로는 묶음의 영상 파일 이름 168개에서 두 쪽이 모두 같아요. 165개는 둘 다 맞고, 코난 극장판 3개는 둘 다 30화로 같게 틀려요.
- **회차 없음·묶음 이름**: trname으로 바꾸지 않고 받은 이름을 남겨요(사용자 결정, 2026-10-07). [수집 명세의 영상 회차 변환](../specs/collection.md#영상-회차-변환)에 두 결정을 적었어요.
- 읽기마다 다르던 범위는 다음처럼 맞췄어요(리드가 정함). 묶음의 어느 이름도 읽기가 바뀌지 않는 쪽을 골랐어요.
  - 확장자는 `.[A-Za-z0-9]{2,4}`와 `torrent`를 합친 것이에요.
  - 수정 번호는 수정본 대체가 쓰던 `(?i)v\d{1,2}`예요. 수정본 대체는 파일을 지우므로 그 읽기를 바꾸지 않았어요.
  - 시즌 표기는 `S\d{1,2}E`이고, 회차 앞의 대시는 `\s+-\s+`예요.
  - `03-03`은 trss-collect 안에서 지금처럼 묶음이에요. trss-subtitles의 범위 읽기는 건드리지 않았어요.
- trss-core의 이름 읽기는 정규식을 써요. 엄격한 읽기 셋의 되돌아가기 경우를 손으로 옮기는 것보다 낫고, `regex`는 이미 workspace에 있어요.

### 바꾼 것

| 커밋 | 내용 |
| --- | --- |
| `6c82abd` refactor(core): gather the readers of trname-written names and season folders in trss_core::trname_names | trname이 쓴 이름과 시즌 폴더 읽기를 쓰임마다 하나씩 모았어요. `season_episode`(엄격, trss-web의 할 일과 작품 API, trss-collect의 지난 회차 검색), `is_trname_name`(trss-transmission의 `looks_renamed`), `listed_episode`(라이브러리의 너그러운 읽기, 자막은 따로 허락할 때만), `season_folder`(라이브러리의 시즌 폴더)예요. 회차 예상의 시즌 폴더는 trname의 `Directory`를 그대로 불러요. trss-transmission이 trss-core에 의존해요. |
| `d0403c2` refactor(collect): one implementation of the episode conversion's arithmetic | 회차 변환 계산을 `episode_offset`의 `shift`, `folder_episode`, `leaves_numbers` 하나로 모았어요. `range.rs`, `judge.rs`, `received_as`, `worth_offering`, `same_effect`, trss-web의 `matches!(previous, 0 \| 1)` 복사본을 지웠어요. |
| `d8b8d9d` refactor(collect): read a release name in one place, trss_collect::release_name | `ReleaseName::read`가 무리, 작품, 회차·묶음·회차 없음(`Kind`), stem, 수정 번호, CRC, 표기를 한 구조로 돌려줘요. `Release::parse`, `without_version`, `parse_release`, `whole_episode`, `past_search::release::read`를 지웠어요. `revision.rs`에는 CRC와 `FileIdentity`만 남아요. |
| `4202f22` fix(collect): read a bare four-digit episode | 대시 없는 네 자리 회차를 읽어요. 아래 "동작이 바뀐 것"을 봐요. |
| `9a1b352` fix(collect): read the extension, the revision mark, the season and the dash of a release name one way | 위의 맞춘 범위예요. |
| `41ca53d` fix(collect): keep the name of a torrent read as no episode or as a batch | 회차 없음·묶음 이름의 규칙이에요. |
| `33b5125` test(collect): read the example names of release_name from the corpus | 예제 테스트를 묶음 줄로 옮겼어요. |
| `test(collect): check that the predicted episode is the one in the name the worker gives`(이 결과를 담은 커밋) | 회차 예상과 실제 이름을 견주는 묶음 테스트예요. |

정리 커밋 셋(`6c82abd`, `d0403c2`, `d8b8d9d`)은 기존 테스트와 묶음 테스트를 고치지 않고 통과했어요. `d8b8d9d`는 묶음 이름을 바꿔 만든 18,384개의 이름으로 예전 읽기와 `ReleaseName::read`를 견줘 모든 칸이 같은 것을 확인했어요. `folder_episode`는 릴리스 0–1299(정수와 `.5`), 회차 변환 −1300–1300에서 예전 식과 같았어요. 이 두 확인의 도구는 저장소에 두지 않았어요.

### 동작이 바뀐 것

- **네 자리 회차**(`4202f22`): 묶음의 HatSubs `One Piece 1176`–`1180` 다섯 줄이 작품 `One Piece`, 회차 1176–1180으로 맞게 읽혀서 알려진 실패 표시를 지웠어요. 이 수정은 이름 규칙보다 먼저 들어가야 했어요. 그러지 않으면 trname이 맞게 바꾸던 이 다섯 이름을 회차 없음으로 읽어 바꾸지 않게 돼요.
  - 묶음 밖에서는 `Movie 2019 (BD 1080p)`, `Show 1080 (1080p)`처럼 이름 끝의 네 자리 숫자도 회차로 읽어요. trname은 전에도 이 숫자를 회차로 읽었어요.
- **읽기 범위 맞추기**(`9a1b352`): 묶음에서 바뀐 이름은 0개예요. 바뀐 입력은 테스트로 확인해요.
  - `.torrent`로 끝나는 제목의 CRC와 stem을 읽어요.
  - `Show - 05.webm`을 5화로 읽어요.
  - `Show Vol.12`의 작품을 `Show Vol`로 읽어요.
  - `05V2`를 5화의 수정 번호 2로 읽어요.
  - `05v123`은 회차도 수정 번호도 없는 이름이에요.
  - 공백이 여럿인 대시도 대시 표기로 읽어요.
- **회차 없음·묶음 이름**(`41ca53d`): 수집 주기와 `다시 받기`가 함께 쓰는 이름 콜백(`NameForTrname`)이 `Option`을 돌려줘요. `None`은 이름을 그대로 두라는 뜻이고, trname이 이름을 만들지 못한 경우와 구별해요.
  - 수집 주기는 새로 더한 토렌트(`RenameMode::Added`)도 남겨요. `다시 받기`는 `NAME_NOT_DERIVED` 메모를 남겨요.
  - trss-collect가 회차로 읽지만 trname이 이름을 만들지 못하는 이름은 수집 주기가 지금처럼 지워요. worker 테스트가 이 동작을 [0128](0128-keep-unnamed-cycle-torrent.md)까지 지켜요.
  - trname이 해상도나 CRC의 숫자를 회차로 읽던 6줄(One Piece Film Red, Chibi Maruko-chan 2줄, Ponyo, Kimi no Iro, Magic Tree House)의 알려진 실패 표시를 지웠어요.
  - 묶음 테스트는 이제 trname만이 아니라 worker가 붙이는 이름(`name_for_trname` 다음 trname)을 견줘요.

### 예제 테스트와 묶음

- 2026-10-07에 이름 읽기의 예제 테스트를 약 35개로 셌는데, `1fb4ede`에서 다시 세니 trss-collect에 19개였어요. 35개를 다시 셀 수는 없었어요.
- 동작 변경 커밋에서 5개를 더해 24개가 됐고, 묶음으로 옮긴 뒤 19개예요. 지운 5개는 작품 제목 안의 숫자, 회차와 수정 번호, 묶음, 회차 없음, 해상도·CRC 숫자의 예제예요. stem, CRC, 표기, 무리, 수정 표시를 뺀 이름을 확인하는 테스트는 남겼어요.
- 묶음은 766줄에서 794줄이 됐어요. 예제에만 있던 이름 28개를 출처 `example`로 더했어요.
- 이름을 확인하는 worker의 프로세스 테스트는 남겼어요. 새 이름 규칙의 worker 테스트 4개를 더했어요(수집 주기와 `다시 받기`).

### 남은 것

- 예제 테스트가 회차 종류만 보던 탓에 숨어 있던 작품 읽기의 실패 두 줄이 묶음에 드러났어요. `[Unofficial] Sono Bisque Doll (01-24) Unofficial Batch`와 `[SubsPlease] Sono Bisque Doll - 01~12 [Batch] (1080p)`의 작품 이름에 범위가 남아요. 이 티켓의 범위 밖이라 알려진 실패로 두었어요.
- `H.264`로 끝나는 제목은 지금처럼 stem에서 `.264`를 잃어요.
- 화면의 TypeScript 복사본(`web/src/screens/collect/rules/past-search/api.ts`의 `folderEpisode`)은 그대로예요.
- Erai `(V2)`는 [0115](0115-erai-magnet-revisions.md), 이름을 정하지 못한 토렌트의 제거는 [0128](0128-keep-unnamed-cycle-torrent.md)에서 다뤄요.

### 검증한 것

2026-10-08에 `cargo test --locked --workspace -j 4`로 확인했어요. 실패는 모두 0개, 무시는 모두 14개예요.

| 뒤 | 통과 |
| --- | ---: |
| 시작(`1fb4ede`) | 2,848 |
| `6c82abd`, `d0403c2`, `d8b8d9d`, `4202f22` | 2,848 |
| `9a1b352` | 2,852 |
| `41ca53d` | 2,856 |
| `33b5125` | 2,851 |
| 회차 예상 테스트 | 2,852 |

회차 예상 테스트는 예상 쪽의 회차 변환을 일부러 2로 바꾸자 영상 이름 168개 중 161개가 다르다며 실패했어요. 나머지 7개는 두 쪽 모두 회차가 없는 이름이에요.
