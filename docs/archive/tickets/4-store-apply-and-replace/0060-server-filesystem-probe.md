# 0060 실제 서버의 미디어 디스크에서 파일 쓰기를 확인해요

- 상태: 완료 (2026-10-07, 실제 서버에서 확인했어요. 아래 "서버 결과")
- 출처: [체크포인트와 중단 복구](../../../specs/jobs.md#체크포인트와-중단-복구)의 실제 파일시스템 검증, [보관본과 적용본](../../../specs/subtitles.md#보관본과-적용본)의 임시 파일 위치, [구현 경계](../../../specs/web-app.md#구현-경계와-실행-순서)의 실행 사용자
- 막는 티켓: 없음

## 작업

확인할 일이에요. 보관·적용이 기대는 파일 동작을 실제 서버(j4105)의 미디어 디스크에서 작은 탐침으로 확인해요. 실제 배포는 목표 5예요.

- 탐침은 worker와 같은 이미지, 같은 bind mount(`/downloads`)와 앱 데이터 폴더(`/data`), 같은 메모리 한도(256M, [0065](0065-archive-extraction.md)에서 128M에서 올렸어요)로 띄운 일회용 컨테이너에서 uid·gid 1000:1000으로 돌려요.
  운영 중인 작품 폴더가 아니라 미디어 디스크 안의 시험 폴더에 쓰고, 끝나면 지워요.
- 확인할 것이에요.
  - 시험 폴더가 있는 파일시스템의 종류(xfs로 알고 있어요)와 마운트 옵션
  - 같은 파일시스템에서 `renameat2(RENAME_NOREPLACE)`가 되고, 목적지가 있으면 `EEXIST`로 거절하는지
  - `/data`에서 `/downloads`로의 이름 바꾸기가 `EXDEV`로 실패하는지. 보관·적용의 임시 파일을 `.trss/` 안에 두는 까닭이에요.
  - 파일과 폴더의 `fsync`, 이름 바꾸기 뒤에도 `장치:inode`가 그대로인지
  - 1000:1000으로 만든 파일·폴더의 소유와 권한, Transmission이 만든 작품 폴더 안에 `.trss/`를 만들 수 있는지
- worker의 memcg 안에서 수백 MiB 파일을 읽고 쓰는 동안 `memory.current`의 최대치, 페이지 캐시, `memory.events`의 `oom_kill`을 재요.
  [0058](../../../tickets/0058-server-oom-and-resource-limits.md)의 커널 문제(memcg 재시도)가 worker에도 닿는지 보려는 거예요.
- 탐침은 저장소에 남겨 다시 돌릴 수 있게 해요. 서버에서 돌릴 명령은 사용자가 실행하고, 저는 결과를 받아 기록해요.

## 완료 기준

| 관찰 | 기대 결과 |
| --- | --- |
| 탐침 실행 기록 | 서버 커널, 파일시스템 종류, 마운트 옵션과 항목별 결과가 날짜와 함께 이 티켓에 있어요. |
| 이름 바꾸기 | 같은 파일시스템에서는 덮어쓰지 않는 이름 바꾸기가 되고 목적지가 있으면 거절돼요. 다른 파일시스템이면 실패해요. 결과가 다르면 임시 파일의 위치를 다시 정하고 명세를 고쳐요. |
| 1000:1000 | 만든 파일·폴더의 소유가 1000:1000이고, Transmission이 만든 작품 폴더에 `.trss/`를 만들 수 있어요. |
| worker 한도의 memcg | 큰 파일을 읽고 쓰는 동안의 최대 `memory.current`와 `oom_kill` 수가 있어요. `oom_kill`이 생기면 0058에 함께 적고 worker의 한도를 다시 정해요. |

## 만든 도구

- **탐침** `crates/trss-probe`는 `rustix`만 쓰는 정적 실행 파일(`trss-probe`)이에요. 앞으로 나올 릴리스의 이미지에는 `/usr/local/bin/trss-probe`로 들어가고, 지금 서버에 있는 0.5.0 이미지에는 없어서 아래처럼 바이너리를 따로 만들어 마운트해요.
- **실행 스크립트** `deploy/probe.sh`는 compose 폴더의 `.env`에서 `MEDIA_DIR`·`TRSS_DATA_DIR`·`TRSS_VERSION`·`TRSS_UID`·`TRSS_GID`를 읽어, 서버의 이미지로 일회용 컨테이너(`--user 1000:1000 --memory 256m --memory-swap 256m --cpus 0.25 --network none`)를 띄워요. worker와 같은 마운트(`/downloads`, `/data`)를 쓰고 탐침 바이너리는 읽기 전용으로 마운트해요.
- **빌드** `Dockerfile`의 `probe-binary` 대상이 탐침과 압축 해제의 자식 프로그램(`trss-extract`, [0065](0065-archive-extraction.md)의 `--unpack`에 써요)을 빌드해서 정적 실행 파일 둘을 내보내요.
- 탐침이 하는 일은 `trss-probe --help`와 모듈 설명에 있어요. 만든 폴더·파일은 모두 `.trss-probe-<pid>-<번호>`라는 이름이고 끝에 지워요. 그 이름이 아닌 것은 만들지도 지우지도 않아요. 작품 폴더(`--work`)에는 시험 하위 폴더 하나와 거기서 옮겨 올린 파일 하나만 만들고 지워요. 숨김 이름이라 라이브러리 검색(`discovery`)이 작품으로 읽지 않아요.

| 확인 | 탐침의 항목 |
| --- | --- |
| 파일시스템 종류·마운트 옵션 | `/proc/self/mountinfo`에서 `/downloads`·`/data`·작품 폴더가 놓인 마운트(종류, 장치, 마운트 옵션, 슈퍼블록 옵션)와 `st_dev`, 커널 버전 |
| 덮어쓰지 않는 이름 바꾸기 | 같은 마운트 안에서 `renameat2(RENAME_NOREPLACE)`가 되고, 있는 파일·폴더 위로는 `EEXIST`로 거절하며 아무것도 바꾸지 않는지(`/downloads`와 `/data` 각각) |
| `EXDEV` | `/data`의 파일을 `/downloads`로 옮기면 실패하는지. 두 폴더의 마운트 ID가 다르면 `EXDEV`가 나와야 해서 항목이 실패 판정이고, 같은 마운트면 결과만 알려 줘요. |
| `fsync`, `장치:inode` | 파일과 폴더의 `fsync`, 이름 바꾸기 앞뒤의 `장치:inode` |
| 소유와 권한 | 만든 파일·폴더의 소유가 `--owner`(기본 1000:1000)인지, 모드가 무엇인지 |
| 작품 폴더 | 주어진 작품 폴더 안에 시험 하위 폴더(`.trss/` 대신)를 만들고, 거기서 파일을 쓰고 `fsync`한 뒤 폴더 위로 `RENAME_NOREPLACE`로 옮겨 올릴 수 있는지, 소유는 무엇인지 |
| worker 한도의 memcg | 300MiB(`--size-mib`) 파일을 쓰고 `fsync`하고, `posix_fadvise(DONTNEED)`로 캐시를 버린 뒤 읽으며 바이트를 비교해요. 그동안 5ms마다 `memory.current`·`memory.stat`(`file`·`anon`·`file_dirty`·`file_writeback`)의 최대치를 재고, `memory.events`를 앞뒤로 보여 줘요. `oom_kill`이 늘면 실패 판정이에요. |

## 실행 방법

사용자가 실행해요. `[내 PC]`는 이 저장소가 있는 PC에서, `[서버]`는 서버(j4105, `~/trss`)에서 실행하는 명령이에요.

1. `[내 PC]` 저장소에서 정적 실행 파일을 빌드해요. 몇 분 걸리고 `probe-out/trss-probe`와 `probe-out/trss-extract`가 생겨요. `trss-extract`는 압축 해제 측정([0065](0065-archive-extraction.md#실제-서버에서-재는-방법))에만 써요.
   ```sh
   docker build --target probe-binary --output type=local,dest=probe-out .
   ```
2. `[내 PC]` 실행 파일과 스크립트를 서버의 compose 폴더로 복사해요.
   ```sh
   scp probe-out/trss-probe deploy/probe.sh j4105:~/trss/
   ```
3. `[서버]` 확인에 쓸 작품 폴더를 변수에 담아요.
   작품 폴더는 감시 폴더인 수집 폴더와 보관 폴더 바로 아래의 폴더이고, worker가 `.trss/`를 만들고 적용본을 쓰는 곳이에요. 그래서 수집 폴더에서 하나, 보관 폴더에서 하나를 골라요. 안의 파일은 건드리지 않아요.
   `$MEDIA_DIR/downloads/`는 Transmission의 기본 저장 폴더이고 감시 폴더가 아니라서 쓰지 않아요.
   `COLLECT`와 `ARCHIVE`는 앱의 수집 설정에 있는 두 폴더에서 `/downloads/`를 뺀 이름이에요. 아래 값은 개발 환경에 가져온 서버 DB의 설정이에요(2026-10-07 확인).
   명령은 `.env`에서 `MEDIA_DIR`를 읽고, 두 폴더에서 숨김이 아닌 첫 폴더를 하나씩 골라요. 파일 하나뿐인 torrent는 폴더 없이 수집 폴더에 바로 놓이므로 고르지 않아요. 마지막 줄은 두 감시 폴더와 고른 작품 폴더의 소유와 권한을 보여 줘요.
   다른 폴더를 쓰려면 `WORK1`이나 `WORK2`에 그 이름을 다시 넣어요. 변수는 그 터미널에만 있으니 4·5단계도 같은 터미널에서 실행해요.
   ```sh
   cd ~/trss
   MEDIA=$(grep '^MEDIA_DIR=' .env | tail -n 1 | cut -d= -f2- | tr -d '"')
   COLLECT='Shows (current)'
   ARCHIVE='Shows'
   WORK1=$(ls -p "$MEDIA/$COLLECT" | grep '/$' | head -n 1 | tr -d /)
   WORK2=$(ls -p "$MEDIA/$ARCHIVE" | grep '/$' | head -n 1 | tr -d /)
   ls -ld "$MEDIA/$COLLECT" "$MEDIA/$COLLECT/$WORK1" "$MEDIA/$ARCHIVE" "$MEDIA/$ARCHIVE/$WORK2"
   ```
   컨테이너 안에서는 `$MEDIA_DIR`가 `/downloads`라서, 같은 폴더가 `/downloads/$COLLECT/$WORK1`로 보여요.
4. `[서버]` 탐침을 돌리고 출력을 파일에도 남겨요. 도중에 컨테이너가 죽으면 출력이 중간에서 끊겨요.
   ```sh
   chmod +x probe.sh trss-probe
   ./probe.sh -- --work "/downloads/$COLLECT/$WORK1" --work "/downloads/$ARCHIVE/$WORK2" 2>&1 | tee probe-$(date +%F).txt
   ```
   더 확인할 작품 폴더가 있으면 `--work`를 더 붙여요.
5. `[서버]` 출력에 `make the test folder`의 `Permission denied`가 있으면 미디어 디스크의 루트에 1000:1000이 쓰지 못하는 거예요. 그 결과도 기록할 값이니 그대로 두고, 시험 폴더를 수집 폴더 안에 만들도록 `--media`를 더해 다시 돌려요. worker가 작품 폴더를 옮기는 곳이 수집 폴더와 보관 폴더라서, 루트보다 이쪽이 운영과 같은 조건이에요.
   ```sh
   ./probe.sh -- --media "/downloads/$COLLECT" --work "/downloads/$COLLECT/$WORK1" --work "/downloads/$ARCHIVE/$WORK2" 2>&1 | tee probe-$(date +%F)-b.txt
   ```
6. 3단계 `ls -ld`의 출력과 `probe-<날짜>.txt`의 내용을 저에게 붙여 주세요. 제가 날짜·커널·파일시스템과 항목별 결과를 이 티켓에 적고, 결과가 다르면 임시 파일의 위치와 명세를 다시 정해요.

알아 둘 것이에요.

- `/data`는 서버의 앱 데이터 폴더가 아니라 `TRSS_DATA_DIR` 옆에 새로 만든 임시 폴더(`.trss-probe-data.XXXXXX`, 같은 디스크)이고 스크립트가 끝에 지워요. 서버의 앱 데이터 폴더는 [0062](0062-run-as-media-user.md)로 소유를 옮기기 전까지 root 소유라 1000:1000으로 쓸 수 없어요. 옮긴 뒤에 진짜 폴더로 돌리려면 `--data-dir "$HOME/trss/data"`처럼 호스트 경로를 줘요. 두 폴더 모두 컨테이너에서는 서로 다른 bind mount라서 `/data`에서 `/downloads`로의 이름 바꾸기는 어느 쪽이든 `EXDEV`여야 해요.
- 컨테이너 하나만 256M 한도로 띄워요. 서버의 다른 컨테이너를 멈추지 않아도 돼요. 다만 Transmission이 한창 받는 동안에는 디스크를 같이 써서 속도 수치가 낮게 나올 수 있어요. 수치(MiB/s)가 아니라 `oom_kill`과 최대 `memory.current`가 확인할 값이에요.
- 한도에 닿아 컨테이너가 죽으면 출력이 `writing 300 MiB …`에서 끊겨요. 그때는 `docker ps -a`와 `docker inspect <컨테이너> --format '{{.State.OOMKilled}}'`, `dmesg`의 OOM 줄을 함께 알려 주세요.
- 도중에 `Ctrl-C`로 끊으면 `.trss-probe-*` 폴더가 남을 수 있어요. 미디어 디스크와 작품 폴더 안의 그 이름만 지우면 돼요.
- 스크립트는 `--env`로 다른 `.env`를, `--image`로 다른 이미지를, `--memory`로 다른 한도를 받아요(`./probe.sh --help`).

## 서버 결과 (2026-10-07, j4105)

사용자가 서버에서 3·4단계를 실행했어요(11:33 UTC). 탐침은 2026-10-05에 `probe-binary`로 빌드한 실행 파일(`trss-probe` 0.1.0)이고, 그 뒤 탐침과 `trss-archive`의 코드는 바뀌지 않았어요. 64개 항목이 모두 통과했어요.

| 조건 | 값 |
| --- | --- |
| 호스트 | Rocky Linux 9.8, 커널 `5.14.0-687.53.1.el9_8.x86_64`, Docker 29.8.1 |
| 컨테이너 | 서버의 `0.5.0` 이미지에 탐침을 읽기 전용으로 마운트했어요. 1000:1000, 메모리 256M(swap 같음), CPU 0.25예요. |
| 미디어(`/downloads`) | 호스트 `/data/storage1/media`, `/dev/sdb`의 xfs, 장치 8:16. 마운트 옵션 `rw,relatime`, 슈퍼블록 옵션 `rw,seclabel,attr2,inode64,logbufs=8,logbsize=32k,noquota` |
| 데이터(`/data`) | 앱 데이터 폴더 옆의 임시 폴더. `/dev/mapper/rl_j4105-home`의 xfs, 장치 253:2, 옵션은 미디어와 같아요. |
| 작품 폴더 | 수집 폴더의 `All Works Maid`(소유 1000:1000, 모드 0755)와 보관 폴더의 `Bocchi the Rock!`(1000:1000, 0775). 둘 다 미디어와 같은 마운트예요. |

| 확인 | 결과 |
| --- | --- |
| 미디어 루트의 시험 폴더 | `/downloads` 바로 아래에 만들 수 있었어요. 그래서 5단계는 필요 없었어요. 파일과 폴더의 `fsync`가 되고, 만든 파일은 1000:1000 `0644`, 폴더는 1000:1000 `0755`였어요. |
| 덮어쓰지 않는 이름 바꾸기 | 미디어와 데이터 양쪽에서 빈 이름으로는 되고, 있는 파일 위로는 `EEXIST`(17)로 거절했어요. 거절 뒤 두 파일과 처음 파일의 내용이 그대로였어요. 폴더를 있는 폴더나 파일 위로 옮기는 것도 `EEXIST`였고, 빈 이름으로는 됐어요. |
| `장치:inode` | 이름 바꾸기와 하위 폴더에서 위로 옮기기의 앞뒤에서 그대로였어요. |
| `EXDEV` | `/data`에서 `/downloads`로의 이름 바꾸기가 `EXDEV`(18)로 실패했어요. 두 마운트는 장치도 달라요. |
| 작품 폴더 | 두 작품 폴더 모두 안에 하위 폴더(`.trss/` 대신)와 파일을 만들어 `fsync`하고, 파일을 작품 폴더로 옮겨 올렸어요. 만든 하위 폴더와 파일은 1000:1000이었어요. |
| 256M memcg | 300MiB를 쓰고 `fsync`하는 데 2.9초(103MiB/s), 캐시를 버리고 읽어 비교하는 데 3.5초(86MiB/s)였어요. 최대 `memory.current`는 256.0MiB(한도)였어요. 쓰는 동안의 최대는 `file` 254.0MiB, `anon` 1.1MiB, `file_dirty` 33.8MiB, `file_writeback` 45.4MiB였어요. `memory.events`의 `max`는 374번 늘었고 `oom`과 `oom_kill`은 0이었어요. |
| 정리 | 만든 시험 폴더 여섯 개를 모두 지웠어요. |

읽은 것이에요.

- 보관·적용이 기대는 파일 동작은 서버의 xfs에서도 개발 환경과 같았어요. 같은 파일시스템의 `RENAME_NOREPLACE`, 다른 마운트 사이의 `EXDEV`, `fsync`, 식별 유지가 모두 기대대로예요. 그래서 임시 파일을 `.trss/` 안에 두는 명세는 그대로 둬요.
- Transmission이 만든 작품 폴더는 이미 1000:1000 소유예요. 1000:1000으로 도는 worker가 그 안에 `.trss/`를 만들 수 있어요.
- 이 커널에도 [0009](../1-app-owned-collection/0009-deploy-web-worker.md#이름과-라벨이-되돌아간-일-호스트-커널의-oom-강제-종료)에서 Transmission을 죽인 memcg 재시도 변경이 있어요. 이번에는 한도에 374번 닿았지만 `oom_kill`은 없었어요. 그래서 [0058](../../../tickets/0058-server-oom-and-resource-limits.md)에 적을 일은 없고 worker의 한도도 그대로 둬요.
  다만 탐침은 한 프로세스가 차례로 쓰고 읽었어요. 0009의 강제 종료는 비운 자리를 다른 쓰기가 먼저 가져가는 동안 났어요. 그래서 여러 곳이 함께 쓰는 경우에도 같을지는 이 결과로 알 수 없어요. 압축 해제처럼 자식 프로세스와 함께 쓰는 경우는 [0065](0065-archive-extraction.md)의 서버 측정이 봐요.

이 결과로 알 수 없는 것이에요.

- 3단계 `ls -ld`의 출력은 받지 않았어요. 그래서 두 감시 폴더(`Shows (current)`, `Shows`)의 소유와 모드는 모르는 채예요. 규칙 보관은 작품 폴더를 한 감시 폴더에서 다른 감시 폴더로 옮기므로, 1000:1000이 두 폴더에 쓸 수 있어야 해요. 미디어 루트는 쓸 수 있었어요. 0081의 1000:1000 전환 절차에 미디어 폴더 권한 확인이 있어요.
- `/data`는 서버의 앱 데이터 폴더가 아니라 같은 디스크의 임시 폴더였어요. 앱 데이터 폴더의 소유는 0081에서 옮겨요.
- 속도는 그때 Transmission이 디스크를 함께 썼는지 모르므로 참고값이에요.
- 재생기가 NFS로 읽을 때의 모습은 이 탐침이 보지 않아요([0061](0061-infuse-placement-check.md)).

## 로컬 시험 (2026-10-04, 개발 PC, 서버 결과가 아니에요)

탐침과 `deploy/probe.sh`가 돌아가는지만 개발 PC에서 봤어요. 커널은 `7.2.8-arch1-2`이고 파일시스템은 btrfs(`compress=zstd:3`)예요. 서버(Rocky Linux 9의 xfs, `5.14` 커널)의 동작은 이 시험에서 알 수 없어요.

- 이 브랜치에서 `probe-binary`로 빌드한 실행 파일을 `file`로 보면 `static-pie linked`이고 크기는 약 750KiB예요.
- 임시 `.env`(`MEDIA_DIR=./media`, `TRSS_DATA_DIR=./data`, `TRSS_VERSION=0062`)와 임시 작품 폴더 하나로 `./probe.sh -- --work "/downloads/downloads/Work A"`를 돌렸어요. 컨테이너는 `--user 1000:1000 --memory 128m --memory-swap 128m`이었고 이미지는 이 브랜치에서 만든 것이에요.
- 56개 항목이 모두 통과했고 종료 상태는 0이었어요. `/data`에서 `/downloads`로의 이름 바꾸기는 두 마운트 ID가 달라 `EXDEV`였어요(`st_dev`는 같은 `0:47`이었으니 같은 파일시스템이어도 마운트가 다르면 `EXDEV`예요). 만든 소유는 1000:1000이었어요.
- 128M에서 300MiB를 쓰고 읽는 동안 최대 `memory.current`는 128.0MiB(한도), 페이지 캐시(`file`)는 최대 약 126MiB, `anon`은 약 2MiB였고 `memory.events`의 `max`는 1,386번 늘고 `oom_kill`은 0이었어요. 이 PC의 커널에서는 한도에 닿아도 캐시를 줄이며 끝났다는 뜻이에요.
- 실패 경로도 봤어요. root 소유의 `/data` 폴더와 없는 작품 폴더를 주자 해당 항목이 `[FAIL]`이고 종료 상태가 1이었어요.
- 이 시험으로 알 수 없는 것은 실제 xfs의 동작, Transmission이 만든 폴더의 소유와 모드, 서버 커널에서의 `memory.events`예요. 서버에서 돌려야 알 수 있어요.
