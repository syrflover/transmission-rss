# 0091 trname이 세 자리 회차와 .5 회차를 맞게 읽어요

- 상태: 대기
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
