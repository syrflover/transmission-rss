# 열린 관찰

끝난 변경이 남긴 관찰 가운데, 실제 서버를 쓰는 동안 계속 지켜봐야 하는 것과 나중에 생길 환경에서만 확인할 수 있는 것을 모아요. 끝난 명세와 티켓은 다시 열지 않아서, 거기에만 남은 관찰은 다시 확인되지 않기 때문이에요.

항목마다 지켜볼 것, 드러나는 모습, 결과가 바꿀 것, 출처를 적으세요. 관찰한 것이 생기면 날짜를 붙여 그 항목에 더하고, 지켜보던 것을 뒷받침하든 거스르든 적으세요. 항목은 사용자가 결론을 내리거나 그만 보기로 할 때만 지우고, 그 결론은 지우는 커밋의 메시지에 적으세요.

## 다시 시작할 때 끊긴 명령과 자막 작업

- 지켜볼 것: 서버의 `trss`가 다시 시작하거나 업그레이드될 때, 끝나지 않은 명령을 다시 실행하고 시도를 한 번으로 세는지, 단계를 실행하던 자막 작업이 `중단됨`이 되고 `다시 시작`하면 끊긴 단계부터 이어 가는지예요. `중단됨`이 얼마나 자주 생기는지도 봐요.
- 드러나는 모습: 시작 로그의 `Subtitle jobs: N left running by an earlier process are interrupted` 줄, 할 일 화면 `자막 작업`의 `중단됨` 배지, `commands` 표의 `attempts`예요.
- 결과가 바꿀 것: `중단됨`이 자주 생겨 사람이 다시 시작하는 일이 짐이 되면 [끊긴 작업 ADR](adr/0020-interrupted-jobs-wait-for-a-restart.md)의 결정을 다시 봐요. 다시 실행한 명령이 틀린 일을 하면 명령을 다시 실행하는 규칙을 고쳐요.
- 출처: [구조 가볍게 하기 명세의 명령](archive/tickets/lighter-structure/lighter-structure.md#명령)과 [끊긴 자막 작업](archive/tickets/lighter-structure/lighter-structure.md#끊긴-자막-작업). 테스트는 끊긴 상태를 DB에 만들어 확인했고, 0.8.0으로 올리기 20분쯤 전의 서버 DB 사본에는 끝나지 않은 명령과 실행 중인 작업이 없었어요(2026-10-22).

## 수집 주기 하나가 보내는 바뀜의 수

- 지켜볼 것: 수집 주기는 본 항목마다 `history` 바뀜을 하나씩 보내요. 바뀜 채널은 듣는 쪽마다 1,024개까지 쌓아 두므로, 피드의 항목이 늘어 한 번에 그보다 많이 보내면 화면이 `lagged`를 받아요.
- 드러나는 모습: `dev/measure/record-events.py`로 서버의 `GET /api/events`를 기록했을 때 `lagged` 이벤트가 오거나, 1초 안에 온 바뀜이 1,024개에 가까워져요. 화면은 `lagged`를 받으면 보이는 것을 모두 다시 읽어서 틀리지는 않지만 비싸요.
- 결과가 바꿀 것: 서버가 바뀜을 모아 보내거나 채널을 키워요([바뀜 알림 ADR](adr/0021-change-events-from-the-db-connection.md)).
- 출처: [0164의 개발 환경에서 측정한 이벤트 수](archive/tickets/lighter-structure/0164-db-change-events.md#개발-환경에서-측정한-이벤트-수)
- 2026-10-22 15:07–15:18 서버(0.8.0)에서 11분 동안 바뀜 511개(`history` 500개, `collect_status` 8개, `anissia` 3개)가 왔어요. 1초에 가장 많이 온 것은 주기가 끝날 때의 251개였고, `lagged`는 없었어요.

## 시계만으로 바뀌는 상태의 이벤트

- 지켜볼 것: 마감 서비스가 수집 멈춤, 방영 시각, 서울 자정처럼 시계만으로 바뀌는 때에 `collect_status`를 보내는지예요. 서비스는 가장 이른 때까지 tokio 시간으로 자고, 한 시간을 넘게 자지 않아요. 서버 시계가 바뀌거나 tokio의 시간과 벽시계가 어긋나면 그때를 늦게 알릴 수 있어요.
- 드러나는 모습: 방영 시각이 지났는데 편성 카드가 `방영 전`으로 남거나, 서울 날짜가 바뀌었는데 `오늘`이 옮겨 가지 않다가 새로고침하면 바뀌어요.
- 결과가 바꿀 것: 서비스가 시계를 다시 보는 간격이나 잠드는 방식을 고쳐요.
- 출처: [0156의 확인하지 않은 것](archive/tickets/lighter-structure/0156-events-for-covers-schedule-status.md#확인하지-않은-것). 테스트는 멈춘 tokio 시간에서만 자요.
- 2026-10-22 개발 환경에서 방영 시각을 15:33으로 바꿔 두자, 쓰기 없이 15:33:00.002에 `collect_status`가 왔어요.

## 서버의 Transmission을 거친 일괄 보관

- 지켜볼 것: 서버에서 `모두 보관`·`N개 보관`을 처음 할 때, 서버가 규칙 순서대로 보관하고 탭을 숨겨도 남은 규칙을 이어 가는지예요. 규칙이 4주 조용해져 보관 제안이 생겨야 할 수 있어요.
- 드러나는 모습: 일괄 보관이 끝까지 가지 않고 멈추거나, 다시 보인 탭이 끝난 규칙 수를 틀리게 보여요.
- 결과가 바꿀 것: `trss` 크레이트의 `archive_batch` 실행을 고쳐요.
- 출처: [0163의 개발 환경에서 확인](archive/tickets/lighter-structure/0163-batch-archive-on-server.md#개발-환경에서-확인). 개발 환경의 Transmission에서만 봤어요.

## reverse proxy를 거친 이벤트

- 지켜볼 것: 서버의 웹 앞에 reverse proxy를 두면, 이벤트 스트림이 쌓이지 않고 곧바로 화면에 닿는지예요. 지금 서버는 LAN 주소와 포트로 바로 열어서 확인할 수 없어요.
- 드러나는 모습: 명령의 결과가 몇 초 늦게 보이거나, 화면이 45초마다 연결을 새로 열어요.
- 결과가 바꿀 것: readme에 proxy의 버퍼링과 압축을 끄는 설정을 적어요. 서버는 `X-Accel-Buffering: no`를 보내고 압축하지 않아요.
- 출처: [0154의 이벤트 경로](archive/tickets/lighter-structure/0154-change-events-and-command-reads.md#이벤트-경로)

## 다른 파일시스템의 미디어

- 지켜볼 것: 미디어를 다른 파일시스템(NFS, SMB, 대소문자를 가리지 않는 파일시스템)으로 옮길 때, 교체에 꼭 필요한 `RENAME_EXCHANGE`와 대소문자만 다른 이름 바꾸기가 되는지예요. 지금은 서버(j4105)의 xfs 작품 폴더에서만 확인했어요.
- 드러나는 모습: 옮기기 전에 새 마운트에서 `deploy/probe.sh`를 돌리면 검사가 실패해요. 옮긴 뒤에는 자막 교체가 모두 실패해요.
- 결과가 바꿀 것: 교체를 다른 방법으로 할지 사용자 결정을 받아요.
- 출처: [0151](archive/tickets/lighter-structure/0151-probe-rename-exchange.md)과 [0153의 바뀐 동작](archive/tickets/lighter-structure/0153-no-effect-records.md#바뀐-동작)

## 서버 Transmission의 OOM 강제 종료

- 지켜볼 것: 봇 토렌트 여럿을 며칠 받는 동안 서버(j4105)의 Transmission이 `oom_kill`로 죽지 않는지, 토렌트의 이름과 라벨이 되돌아가지 않는지예요. RHEL 9 커널 `5.14.0-687.36.1`부터 `687.53.1`까지는 쓰는 중인 `transmission-daemon`을 한도에서 죽였어요(RHEL-211058). 서버는 2026-10-19에 수정(RHEL-269060)이 든 `5.14.0-687.58.1.el9_8`로 올렸고, `transmission.slice`의 `MemoryHigh=384M`도 남아 있어요.
- 드러나는 모습: `transmission.slice`와 그 아래 컨테이너의 `memory.events`에서 `oom_kill`이나 `max`가 0보다 커지거나, 토렌트의 이름과 라벨이 되돌아가요.
- 결과가 바꿀 것: 0이 아니면 커널 수정이 듣지 않는 것이라 원인을 다시 찾아요. Transmission의 512M 한도는 그대로 두기로 했어요(사용자 결정).
- 출처: [server-resource-limits/0058](archive/specs/2026-10-04-server-resource-limits/0058-server-oom-and-resource-limits.md)과 [1-app-owned-collection/0009의 OOM 강제 종료 기록](archive/tickets/1-app-owned-collection/0009-deploy-web-worker.md#이름과-라벨이-되돌아간-일-호스트-커널의-oom-강제-종료)
- 2026-10-02 점검에서 컨테이너의 `oom_kill`은 0이었어요. 그 사이 받은 것은 1.4GB 한 회차뿐이었어요.
- 2026-10-19 커널을 올리기 전 slice의 `memory.events`는 `high` 30455, `max` 0, `oom_kill` 0이었어요. 캐시가 384M를 넘는 동안 512M 한도에는 닿지 않았어요. slice가 활성화된 시각과 그 사이 받은 양은 확인하지 않았어요. 재부팅 직후에는 모두 0이었어요.

## `trss` 컨테이너의 메모리 한도

- 지켜볼 것: 큰 폴더를 스캔하거나 표지를 올릴 때 `trss`가 쓰는 최대 메모리예요. 한도 384M는 web(128M)과 worker(256M)가 두 컨테이너였을 때의 한도를 더해 시작한 값이고, 사용량을 보고 정한 값이 아니에요. 서버 브라우저 `trss-browser`(한도 768M)도 함께 봐요.
- 드러나는 모습: 서버에서 `docker stats`로 본 `trss`와 `trss-browser`의 메모리 사용량이에요.
- 결과가 바꿀 것: 측정한 최대치로 두 한도를 다시 정해 compose에 반영해요.
- 출처: [server-resource-limits/0058](archive/specs/2026-10-04-server-resource-limits/0058-server-oom-and-resource-limits.md)과 [lighter-structure/0146](archive/tickets/lighter-structure/0146-one-trss-process-and-container.md)
- 2026-10-02 서버에서 worker는 7.4MiB, web은 3.9MiB를 썼어요.
- 2026-10-22 0.8.0을 올리고 1분쯤 뒤 `trss`는 14.26MiB, `trss-browser`는 13.86MiB였어요([lighter-structure/0162](archive/tickets/lighter-structure/0162-deploy-lighter-structure.md)).
