# 0058 실제 서버의 Transmission OOM과 자원 한도를 지켜봐요

- 상태: 진행 중
- 출처: [0009](../archive/tickets/1-app-owned-collection/0009-deploy-web-worker.md)의 남은 일에서 떼어 냈어요(사용자 결정, 2026-10-04)
- 막는 티켓: 없음

## 배경

0009에서 웹과 worker를 배포하다가 Transmission 데몬이 컨테이너 안에서 되풀이해 죽는 것을 찾았어요. 원인은 호스트 커널(RHEL 9 `5.14.0-687.41.1`부터)에 들어간 memcg 재시도 변경(RHEL-211058)이에요. 원인과 조치는 [0009의 기록](../archive/tickets/1-app-owned-collection/0009-deploy-web-worker.md#이름과-라벨이-되돌아간-일-호스트-커널의-oom-강제-종료)에 있어요.

조치는 `transmission.slice`의 `MemoryHigh=384M`예요. 캐시를 384M 아래로 붙잡아 512M 한도에 닿지 않게 해요. Transmission의 512M 한도는 그대로 둬요(사용자 결정).

이 관찰과 자원 한도 재검토는 실제 서버를 며칠 운영해야 볼 수 있어요. 결과 목표 1의 완료 조건(cron 없이 같은 항목을 같은 폴더로 받는 것)에는 들지 않아서 0009에서 떼어 냈어요(사용자 결정, 2026-10-04).

## 할 일

- 봇 토렌트 여럿을 며칠 받는 동안 Transmission 컨테이너의 `oom_kill`이 0에 머무는지 봐요. 2026-10-02 점검에서는 0이었지만, 그 사이 받은 것은 1.4GB 한 회차뿐이었어요.
- 되돌림(RHEL-255363)이 들어간 RHEL 9 커널이 나오면 올려요. 올린 뒤에도 `oom_kill`이 0인지 봐요. `transmission.slice`는 남겨 둬도 괜찮아요. 2026-10-02 서버의 커널은 `5.14.0-687.53.1.el9_8`이었어요. 이 커널에 되돌림이 들어갔는지는 확인하지 않았어요.
- 웹(128M)과 worker(256M, [0065](../archive/tickets/4-store-apply-and-replace/0065-archive-extraction.md)에서 압축 해제 때문에 128M에서 올렸어요)의 메모리 한도를 다시 정해요. 2026-10-02 사용량은 worker 7.4MiB, web 3.9MiB로 한도의 6% 아래였어요. 큰 폴더 스캔과 표지 올리기 때의 최대치를 보고 정해요. 서버 브라우저([0039](../archive/tickets/3-subtitle-candidates-and-receiving/0039-browser-container-lifecycle.md))가 들어간 릴리스를 올리면 그 컨테이너(한도 768M)의 사용량도 함께 봐요.

## 완료 기준

| 관찰 | 기대 결과 |
| --- | --- |
| 봇 토렌트 여럿을 며칠 받음 | `memory.events`의 `oom_kill`이 0이고, 토렌트의 이름과 라벨이 되돌아가지 않아요. |
| 큰 폴더 스캔, 표지 올리기 | 웹과 worker의 최대 메모리를 재요. 그 값으로 한도를 정해 compose에 반영해요. |
