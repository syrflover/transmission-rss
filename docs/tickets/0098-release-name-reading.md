# 0098 릴리스 이름과 회차 읽기를 모아요

- 상태: 대기
- 출처: [흩어진 개념 모으기](refactoring.md#흩어진-개념-모으기), [수집 받기 줄기의 재구성](refactoring.md#수집-받기-줄기의-재구성), [영상 회차 변환](../specs/collection.md#영상-회차-변환)
- 막는 티켓: [0090](0090-release-name-corpus.md)(이름 묶음), [0091](0091-trname-long-and-half-episodes.md)(trname 수정), [0092](0092-refactor-baseline.md)(기준값)

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
- trname은 번호 없는 영상 이름(극장판, BD)에서 해상도나 CRC의 숫자를 회차로 읽어요(0090의 묶음에서 공개 피드 6개). trname의 마지막 규칙은 CLI로 손수 바꾸는 자막 이름(`nogame01.ass`)을 읽으므로 trname은 그대로 두고, trss-collect가 회차 없음이나 묶음으로 읽는 이름은 trname으로 바꾸지 않고 받은 이름을 남겨요(사용자 결정, 2026-10-07). 동작 변경이므로 정리 커밋과 따로 커밋해요.
- [0090](0090-release-name-corpus.md)의 묶음 테스트가 읽기가 바뀌지 않았는지 지켜요. 모은 뒤 묶음의 어느 이름이라도 읽는 결과가 달라지면, 그것은 동작 변경이에요. 맞는 쪽을 정해 결과 절에 적고 정리 커밋과 따로 커밋해요.

## 완료 기준

- 위의 읽기가 각각 한 곳에 있고, 예전 복사본은 없어요.
- 묶음 테스트와 기존 테스트가 고치지 않고 통과해요.
- 묶음 테스트가 worker가 실제로 붙이는 이름을 견주고, trname이 번호 없는 영상 이름에서 숫자를 회차로 읽던 줄(2026-10-07에 6개)의 알려진 실패 표시가 없어요.
- 그 뒤 이름 읽기의 예제 테스트(2026-10-07에 약 35개)를 묶음 테스트로 정리해요. 이름을 확인하는 worker의 프로세스 테스트는 남겨요. 앞뒤 개수를 결과 절에 적어요.
