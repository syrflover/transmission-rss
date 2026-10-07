# 0091 trname이 세 자리 회차와 .5 회차를 맞게 읽어요

- 상태: 완료 (2026-10-07)
- 출처: [영상 회차 변환](../specs/collection.md#영상-회차-변환), [0090](0090-release-name-corpus.md)
- 막는 티켓: [0090](0090-release-name-corpus.md)(실제 릴리스 이름 묶음)

## 작업

trss는 받은 영상의 이름을 trname(`github.com/syrflover/trname`, 사용자가 만든 crate)으로 `<작품> S01E05.mkv` 꼴로 바꿔요.
trss는 이름에서 수정 번호만 지우고 나머지를 그대로 trname에 넘겨요.
2026-10-07 trname `6169808`(trss가 고정한 판)을 직접 돌려 보니 다음처럼 읽었어요.

| 넘긴 이름 | trname이 만든 이름 |
| --- | --- |
| `[SubsPlease] One Piece - 1000 (1080p) [ABCD1234].mkv` | `One Piece S01E10.mkv` |
| `[SubsPlease] One Piece - 1001 (1080p) [ABCD1234].mkv` | `One Piece S01E10.mkv` |
| `[Moozzi2] Show - 105 (BD).mkv` | `Show S01E10.mkv` |
| `[Group] Show - 05.5 (1080p).mkv` | `Show S01E80.mkv` |
| `[Group] Show - 105 (1080p).mkv` | `Show S01E105.mkv` |

SubsPlease와 Moozzi2의 규칙은 회차를 두 자리까지만 읽어요. 어느 규칙에도 맞지 않는 이름은 이름 안의 아무 두 자리 숫자를 회차로 잡아서, `1080p`의 `80`을 읽어요.
지난 회차 검색의 판정은 trss-collect의 읽기로 회차를 예상하므로(`past_search/judge.rs`), 이런 이름에서는 예상한 회차와 실제 파일 이름이 달라질 수 있어요.

사용자가 trname을 고쳐도 된다고 했어요(2026-10-07). 개발 PC의 `~/Developments/trname`에서 고쳐요.

- trname은 사용자가 손으로 쓰는 CLI이기도 해요. trname에 있던 테스트는 그대로 통과해야 해요.
- trss는 trname을 git rev로 고정해 받아요. 배포 이미지도 GitHub에서 받으므로, 고친 trname을 GitHub에 push하고 trss의 rev를 올려야 해요. push는 원격 쓰기라서 사용자 승인을 받고 해요.

## 완료 기준

- 0090의 묶음에서 trname의 읽기가 틀렸던 세 자리 회차와 `.5` 회차 이름이 기대값과 같아요. 그 이름들의 알려진 실패 표시를 지웠어요.
- 위 표의 다섯 이름이 trname의 테스트에 있고, 각각 `S01E1000`, `S01E1001`, `S01E105`, `S01E05.5`, `S01E105`가 돼요.
- 바로 고치지 못한 종류는 결과 절의 남은 일에 까닭과 함께 있어요.
- trss가 고친 trname의 rev를 가리키고, workspace 테스트가 통과해요.

## 결과

### 만든 것 (2026-10-07)

- trname `ee46f04`(`fix: read three- and four-digit and half episodes`)에서 SubsPlease, Moozzi2, Ioroid의 규칙과 정리된 이름(`SxxEyy`)의 규칙이 회차를 네 자리까지 읽어요. 다른 그룹의 `[그룹] 작품 - 05.5` 꼴 규칙은 `.5` 회차를 읽어요. 사용자 승인을 받아 GitHub master에 push했어요.
- trss의 trname rev를 `6169808`에서 `ee46f04`로 올렸어요. `Cargo.lock`은 trname 줄만 바뀌었어요. 사이의 trname 커밋 3개(`8acd837`–`6b494b1`)는 CLI(`main.rs`)와 readme만 바꿨어요.
- trss-transmission의 `looks_renamed`와 worker 테스트 하나의 주석이 trname의 두 자리 제한을 까닭으로 들고 있어서 고쳤어요. `looks_renamed`는 제목의 대소문자가 다른 이름 때문에 계속 필요해요.

### 검증한 것

| 완료 기준 | 근거 |
| --- | --- |
| 묶음의 세 자리 회차와 `.5` 회차 이름이 기대값과 같음 | 로컬 trname으로 묶음 테스트를 돌리자, 알려진 실패 중 세·네 자리 회차 23개(수집 이력 4)만 기대값대로 읽혔고 다른 줄은 바뀌지 않았어요. 그 23줄의 표시를 지웠어요. `.5` 회차는 묶음에서 틀리게 읽힌 이름이 없었어요([0090](0090-release-name-corpus.md)). |
| 다섯 이름이 trname의 테스트에 있음 | trname의 `test_trname_long_and_half_episodes`가 다섯 이름과, Ioroid의 세 자리 회차, 정리된 이름 `Show S01E105.mkv`·`One Piece S01E1000.mkv`를 확인해요. 고치기 전의 규칙으로는 첫 이름부터 `One Piece S01E10.mkv`로 실패했어요. trname의 기존 테스트 2개도 통과해요. |
| trss가 고친 rev를 가리키고 workspace 테스트가 통과함 | 2026-10-07 `cargo test --locked -j 4 --workspace`에서 2,755개가 통과했고 14개가 무시됐어요(2분 51초, 다른 빌드가 함께 돌아 load 6–8). |

### 남은 일

- 번호 없는 영상 이름(극장판, BD)에서 해상도나 CRC의 숫자를 회차로 읽는 것(묶음의 공개 피드 6개)은 고치지 않았어요. trname의 마지막 규칙은 이름 안의 아무 숫자나 회차로 잡아서, CLI로 손수 이름을 바꾸는 자막(`nogame01.ass`, `프리렌 1.ass`)을 읽어요. 괄호 안의 숫자를 빼면 `Show [01].ass`처럼 괄호 안에만 회차가 있는 이름을 못 읽게 돼요. 그래서 trss 쪽에서, trss-collect가 회차 없음이나 묶음으로 읽는 이름은 trname으로 바꾸지 않기로 하고 [0098](0098-release-name-reading.md)에서 해요(사용자 결정, 2026-10-07).
- 제목 안의 숫자(`Detective Conan - The Counterfeit Case of Ultra 30`)는 이름만으로 극장판인지 알 수 없어서 알려진 실패로 남겨요.
