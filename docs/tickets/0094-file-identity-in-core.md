# 0094 같은 파일 알아보기를 trss-core 하나로 모아요

- 상태: 대기
- 출처: [흩어진 개념 모으기](refactoring.md#흩어진-개념-모으기), [ADR 0016](../adr/0016-shared-parts-in-core.md)
- 막는 티켓: [0092](0092-refactor-baseline.md)(기준값)

## 작업

"기록한 파일과 지금 파일이 같은 파일인가"를 6곳에서 따로 판단해요. 2026-10-07에 본 곳이에요.

- trss-jobs의 `area`(`object_of`, `same_object`). 저장 형식은 `dev:ino`이고, 예전 생성 시각 꼬리 `:<ns>`도 읽어요. 배치, 교체, 재배치, 받기, 올리기가 써요.
- trss-collect의 `revision`(`FileIdentity`). 저장 형식은 `dev:ino:len:mtime:ctime`이고, 수정본 기록과 회차 변환 되돌리기가 써요. `revisions`의 `owner_of`와 `same_folder`는 같은 순간의 두 stat을 장치와 inode로 견줘요.
- trss-library의 표지(`found_at`). `artwork_files`의 `dev`와 `ino` 열에 저장해요.

재부팅 뒤 장치 번호가 바뀌는 문제와 musl에서 생성 시각을 읽지 못하는 문제를 세 곳이 따로 고쳤어요(`a2f2fa6`, `7eff742`, `c050eba`, `aa2944e`, `6c175f1`).
inode만 견준다는 규칙과, 다른 파일 시스템의 같은 inode를 같은 파일로 볼 수 있는 남은 위험은 사용자 결정이에요(2026-10-06·07). 이 규칙을 trss-core의 한 부품이 지켜요.

- 세 저장 형식을 모두 읽어서 마이그레이션 없이 바꿔요.
- 내용 해시, CRC, 크기, 수정 시각, ctime 같은 추가 검사는 지금 그 검사를 하는 기능 크레이트에 남아요. 견주는 항목이 곳마다 다른 것(trss-jobs의 배치 영상은 수정 시각만, trss-collect는 ctime까지)은 동작이므로 그대로 둬요.
- trss-library의 감시 폴더 발견에 있는 `FileIdentity`(크기와 수정 시각)는 다른 개념이에요. 저장 값은 그대로 두고, 같은 이름이 헷갈리지 않게 타입 이름만 바꿔요.

## 완료 기준

- 같은 파일 판단의 구현이 trss-core에 하나 있고, trss-jobs, trss-collect, trss-library는 그것을 불러요. 예전 판단 함수는 없어요.
- trss-core에 세 저장 형식과 예전 생성 시각 꼬리를 모두 읽는 테스트, 장치 번호만 다른 기록을 같은 파일로 보는 테스트가 있어요.
- 기존 테스트가 고치지 않고 통과해요. 그 뒤 재마운트 테스트 11개 중 inode 규칙만 다시 확인하는 것은 [ADR 0015](../adr/0015-test-a-rule-once-in-its-crate.md)대로 정리하고, 재시작 경로를 확인하는 것은 남겨요. 앞뒤 개수를 결과 절에 적어요.
