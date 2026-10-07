# 0105 회차 대응의 계산과 까닭 문구를 trss-library로 옮겨요

- 상태: 대기
- 출처: [자막의 회차 대응](../specs/library.md#자막의-회차-대응), [자막 작업 줄기의 재구성](refactoring.md#자막-작업-줄기의-재구성), [ADR 0015](../adr/0015-test-a-rule-once-in-its-crate.md)
- 막는 티켓: [0095](0095-episode-text-key-in-core.md)(회차 텍스트의 키)

## 작업

회차 대응(Episode Mapping)의 순수 계산(시즌 회차 구하기, 정한 이동값, 예외)은 trss-jobs의 `mapping`에 있어요. ADR 0015는 회차 대응의 규칙을 trss-library의 것으로 적었어요.
대응한 회차를 배치 대상이나 까닭 문구로 바꾸는 일은 2026-10-07에 trss-jobs의 5곳에 있었어요. 회차 배치(`place/episode`), 묶음 배치의 두 곳(`package`), 재배치(`relocate`), 기록의 범위 검사(`records`)예요.
같은 조건을 다른 말로 적어요. "시즌의 회차로 옮기지 못했어요", "정하지 못했어요", "다른 시즌의 파일로 보여요"가 그 예이고, `1화부터`와 `1–N화`도 4곳에서 따로 만들어요.
trss-collect의 Anissia 자막 목록은 trss-jobs의 대응을 볼 수 없어서 회차 이동을 직접 더해요.

- 대응의 순수 계산과 까닭 문구 만들기를 trss-library로 옮기고, trss-jobs와 trss-collect가 그것을 불러요.
- 화면과 웹 테스트가 문구를 글자 그대로 확인하므로, 문구는 지금 글자를 지켜요. 같은 조건의 다른 문구를 하나로 맞추면 화면이 바뀌므로 따로 커밋하고 결과 절에 적어요.

## 완료 기준

- 회차 대응의 계산과 까닭 문구가 trss-library에 하나씩 있고, trss-jobs의 5곳과 trss-collect가 그것을 불러요.
- 기존 테스트가 고치지 않고 통과해요.
