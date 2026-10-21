# 0151 서버의 미디어 파일시스템이 `RENAME_EXCHANGE`를 지원하는지 `trss-probe`로 확인해요

- 상태: 완료
- 출처: [구조 가볍게 하기 명세의 끊긴 자막 작업](lighter-structure.md#끊긴-자막-작업)의 교체, [명세의 가정](lighter-structure.md#가정)
- 막는 티켓: [0144](0144-lighter-structure-baseline.md)(기준값)

## 작업

[0153](0153-no-effect-records.md)은 교체를 `RENAME_EXCHANGE`로 새 파일과 지금 파일을 한 번에 맞바꿔 해요. 명세는 서버의 미디어 파일시스템이 이를 지원한다고 가정했지만 확인하지 않았어요. 지원하지 않으면 교체 방법을 다시 정해야 하므로, 0153보다 먼저 확인해요.

`trss-probe`는 이미 작품 폴더에서 `RENAME_NOREPLACE`를 확인해요. 같은 자리에 `RENAME_EXCHANGE` 확인을 더하고, `deploy/probe.sh`로 서버에서 돌려요. 이 스크립트는 로컬에서 빌드한 probe 바이너리를 서버에 복사해, 미디어를 쓰는 앱 컨테이너와 같은 사용자·한도·마운트의 일회용 컨테이너에서 돌려요.

서버에서 probe를 돌리는 것은 사용자가 승인해요(명세의 [티켓에서 받을 사용자 결정](lighter-structure.md#티켓에서-받을-사용자-결정)).

## 완료 기준

- [x] probe가 작품 폴더에서 파일 두 개를 `RENAME_EXCHANGE`로 맞바꾸고, 두 이름이 서로의 내용을 가졌는지 확인해요. 지원하지 않는 파일시스템에서는 `RENAME_NOREPLACE`처럼 그렇다고 알려요. 확인이 남긴 파일은 지워요.
- [x] 로컬 파일시스템에서 probe의 테스트가 이 확인을 돌려요.
- [x] 사용자 승인을 받아 서버(j4105)의 미디어 폴더에 있는 작품 폴더 하나에서 probe를 돌리고, 결과를 날짜, 작품 폴더, 파일시스템 종류와 함께 이 티켓의 결과에 적어요. 원본 출력은 `dev/local/measure/0151/`에 있어요.
- [x] 지원하면 [명세의 가정](lighter-structure.md#가정)에서 이 가정을 확인한 것으로 고쳐요. 지원하지 않으면 교체를 어떻게 할지 사용자 결정을 받아 명세와 0153을 고쳐요.
- [x] `deploy/probe.sh`의 설명이 새 확인을 적어요.

## 결과

### 서버 확인

2026-10-21 11:37 UTC에 사용자가 서버(j4105)에서 `deploy/probe.sh`로 probe를 돌렸어요. 서버의 미디어 파일시스템은 `RENAME_EXCHANGE`를 지원해요.

- 작품 폴더는 `Shows (current)/Chii Fuyo`였어요. 컨테이너 안에서는 `/downloads/Shows (current)/Chii Fuyo`이고, 이 폴더가 있는 미디어 마운트는 `/dev/sdb`의 xfs(`rw,relatime`)예요.
- 서버는 Rocky Linux 9.8, 커널 `5.14.0-687.58.1.el9_8`이었어요. probe는 배포된 0.7.0 이미지에서 1000:1000, 메모리 384m, CPU 0.5로 돌았어요.
- 59개 항목이 모두 통과했어요. 작품 폴더의 하위 폴더 파일과 바로 아래 파일을 `renameat2(RENAME_EXCHANGE)`로 맞바꿨고, 두 이름이 서로의 내용과 `장치:inode`를 가졌고, 두 폴더의 `fsync`도 됐어요.
- 정리 단계가 작품 폴더에 만든 `.trss-probe-*` 파일 둘과 시험 하위 폴더를 지웠어요.
- 원본 출력은 이 PC의 `dev/local/measure/0151/probe-j4105-2026-10-21.txt`에 있어요. 서버의 `~/trss/probe-0151-2026-10-21.txt`에도 같은 것이 있어요.

### 로컬 확인

2026-10-21에 probe가 작품 폴더에서 하는 일에 맞바꾸기를 더했어요. 시험 하위 폴더 안의 파일(새 파일 역할)과 작품 폴더 바로 아래의 파일(지금 파일 역할)을 `renameat2(RENAME_EXCHANGE)`로 맞바꾸고, 두 이름이 서로의 내용과 `장치:inode`를 가졌는지 본 뒤 두 폴더를 `fsync`해요. 맞바꿀 두 파일은 probe가 `.trss-probe-*` 이름으로 만들고 끝에 지워요. 맞바꾸기가 `EINVAL`이나 `EOPNOTSUPP`로 실패하면 파일시스템이 지원하지 않는다고 알려요. `deploy/probe.sh`의 설명과 `trss-probe --help`의 `--work` 설명에 이 확인을 적었어요.

- `trss-probe`의 테스트 25개가 통과했어요. 새 테스트 `an_exchange_puts_the_staged_file_in_place_and_the_old_one_in_the_subfolder`가 맞바꾼 뒤의 내용을 보고, 기존 `a_work_folder_gets_a_test_subfolder_that_is_removed_and_nothing_else_changes`가 작품 폴더에 남는 것이 없는지 봐요.
- `docker build --target probe-binary`로 만든 정적 실행 파일을 `probe.sh`로 돌렸어요. 임시 `.env`와 작품 폴더 하나를 둔 대역 폴더에서 이미지는 개발용 `local`, 메모리 검사는 끈 채(`--size-mib 0`)였어요. 59개 항목이 모두 통과했고, 만든 파일과 폴더는 모두 지워졌어요. 원본 출력은 이 PC의 `dev/local/measure/0151/probe-standin.txt`에 있어요.

이 확인으로 알 수 없는 것이에요.

- 대역 폴더는 이 PC의 `/tmp`라서 서버의 xfs가 맞바꾸기를 지원하는지는 서버에서 돌려야 알아요(2026-10-21).
- 지원하지 않는 파일시스템에서 알리는 경로는 돌려 보지 않았어요(2026-10-21). 그런 파일시스템을 이 PC에 쉽게 만들 수 없었어요.

### 지원하지 않을 때의 알림

위 한계 가운데 알리는 경로는 같은 날 Docker의 seccomp로 돌려 봤어요. 기본은 모두 허용하고, `renameat2`의 다섯째 인자(flags)가 `RENAME_EXCHANGE`(2)일 때만 오류를 돌려주는 프로필로, 위 로컬 확인과 같은 정적 실행 파일을 개발용 `local` 이미지의 대역 폴더에서 돌렸어요.

- `EINVAL`(22)이면 `renameat2(RENAME_EXCHANGE) of the two: Invalid argument (os error 22) (errno 22), the filesystem does not support RENAME_EXCHANGE`로 그 항목 하나만 실패했어요.
- `EOPNOTSUPP`(95)이면 같은 항목이 `Not supported (os error 95)`와 같은 안내로 실패했어요.
- 두 번 모두 56개 중 1개가 실패했고, 맞바꾸지 못한 두 파일과 시험 하위 폴더는 정리 단계가 지워 작품 폴더가 비었어요. `RENAME_NOREPLACE` 확인은 영향을 받지 않았어요.

프로필과 원본 출력은 이 PC의 `dev/local/measure/0151/`(`seccomp.json`, `probe-exchange-refused.txt`, `probe-exchange-refused-95.txt`)에 있어요. 진짜로 지원하지 않는 파일시스템에서는 돌려 보지 않았어요(2026-10-21).
