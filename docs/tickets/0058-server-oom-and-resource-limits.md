# 0058 실제 서버의 Transmission OOM과 자원 한도를 지켜봐요

- 상태: 진행 중
- 출처: [0009](../archive/tickets/1-app-owned-collection/0009-deploy-web-worker.md)의 남은 일에서 떼어 냈어요(사용자 결정, 2026-10-04)
- 막는 티켓: 없음

## 배경

0009에서 웹과 worker를 배포하다가 Transmission 데몬이 컨테이너 안에서 되풀이해 죽는 것을 찾았어요. 원인은 RHEL 9 커널 `5.14.0-687.36.1`부터 들어간 memcg 재시도 변경(RHEL-211058)이에요. Red Hat은 2026-10-01 [RHSA-2026:74438](https://access.redhat.com/errata/RHSA-2026:74438)로 낸 `687.54.1`에서 이를 고쳤어요(RHEL-269060). 원인과 조치는 [0009의 기록](../archive/tickets/1-app-owned-collection/0009-deploy-web-worker.md#이름과-라벨이-되돌아간-일-호스트-커널의-oom-강제-종료)에 있어요.

조치는 `transmission.slice`의 `MemoryHigh=384M`예요. 캐시를 384M 아래로 붙잡아 512M 한도에 닿지 않게 해요. Transmission의 512M 한도는 그대로 둬요(사용자 결정).

이 관찰과 자원 한도 재검토는 실제 서버를 며칠 운영해야 볼 수 있어요. 결과 목표 1의 완료 조건(cron 없이 같은 항목을 같은 폴더로 받는 것)에는 들지 않아서 0009에서 떼어 냈어요(사용자 결정, 2026-10-04).

## 할 일

- 봇 토렌트 여럿을 며칠 받는 동안 Transmission 컨테이너의 `oom_kill`이 0에 머무는지 봐요. 2026-10-02 점검에서는 0이었지만, 그 사이 받은 것은 1.4GB 한 회차뿐이었어요. 2026-10-19 점검에서도 `oom_kill`은 0이었고 `transmission.slice`의 `memory.high`는 402653184(384M)였어요. 다만 컨테이너는 같은 날 12:27 UTC에 다시 시작했고 그 뒤로 쓴 양이 560KB 남짓이라, 컨테이너의 카운터만으로는 알 수 있는 게 적어요. slice의 `memory.events`는 slice가 활성화된 뒤 그 아래 컨테이너를 모두 합쳐 세는데, `high` 30455, `max` 0, `oom_kill` 0이었어요. 캐시가 384M를 30,455번 넘는 동안 512M 한도에는 한 번도 닿지 않았고, 죽은 적도 없다는 뜻이에요. slice가 활성화된 시각과 그 사이 받은 양은 확인하지 않았어요.
- 수정(RHEL-269060)이 들어간 `687.54.1` 이후의 커널로 올려요. 올린 뒤에도 `oom_kill`이 0인지 봐요. `transmission.slice`는 남겨 둬도 괜찮아요. 서버의 커널은 2026-10-02와 2026-10-19 모두 `5.14.0-687.53.1.el9_8`이었어요. 2026-10-19에 본 이 커널의 changelog에는 재시도 로직을 바꾼 `mm/memcg: refactor try_charge_memcg retry logic to use for loop`를 비롯한 RHEL-211058 항목 셋이 있고 CentOS Stream의 되돌림인 RHEL-255363은 없었어요. 수정 전의 커널이에요. 같은 날 서버 저장소의 최신 커널은 `5.14.0-687.58.1.el9_8`이었어요. `dnf repoquery --changelogs`로 본 그 커널의 changelog에서 RHEL-211058과 RHEL-255363을 찾았을 때는 아무것도 나오지 않았고, RHEL-269060으로는 찾지 않았어요. 그래서 올린 뒤 `rpm -q --changelog`로 RHEL-269060이 있는지 봐요.
- 웹(128M)과 worker(256M, [0065](../archive/tickets/4-store-apply-and-replace/0065-archive-extraction.md)에서 압축 해제 때문에 128M에서 올렸어요)의 메모리 한도를 다시 정해요. 2026-10-02 사용량은 worker 7.4MiB, web 3.9MiB로 한도의 6% 아래였어요. 큰 폴더 스캔과 표지 올리기 때의 최대치를 보고 정해요. 서버 브라우저([0039](../archive/tickets/3-subtitle-candidates-and-receiving/0039-browser-container-lifecycle.md))가 들어간 릴리스를 올리면 그 컨테이너(한도 768M)의 사용량도 함께 봐요.

## 완료 기준

| 관찰 | 기대 결과 |
| --- | --- |
| 봇 토렌트 여럿을 며칠 받음 | `memory.events`의 `oom_kill`이 0이고, 토렌트의 이름과 라벨이 되돌아가지 않아요. |
| 큰 폴더 스캔, 표지 올리기 | 웹과 worker의 최대 메모리를 재요. 그 값으로 한도를 정해 compose에 반영해요. |
