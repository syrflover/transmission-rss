# 0058 실제 서버의 Transmission OOM과 자원 한도를 지켜봐요

- 상태: 취소 (2026-10-22, 지켜볼 일을 열린 관찰 목록으로 옮겼어요. 아래 "결과")
- 출처: [0009](../../tickets/1-app-owned-collection/0009-deploy-web-worker.md)의 남은 일에서 떼어 냈어요(사용자 결정, 2026-10-04)
- 막는 티켓: 없음

## 배경

0009에서 웹과 worker를 배포하다가 Transmission 데몬이 컨테이너 안에서 되풀이해 죽는 것을 찾았어요. 원인은 RHEL 9 커널 `5.14.0-687.36.1`부터 들어간 memcg 재시도 변경(RHEL-211058)이에요. Red Hat은 2026-10-01 [RHSA-2026:74438](https://access.redhat.com/errata/RHSA-2026:74438)로 낸 `687.54.1`에서 이를 고쳤어요(RHEL-269060). 원인과 조치는 [0009의 기록](../../tickets/1-app-owned-collection/0009-deploy-web-worker.md#이름과-라벨이-되돌아간-일-호스트-커널의-oom-강제-종료)에 있어요.

조치는 `transmission.slice`의 `MemoryHigh=384M`예요. 캐시를 384M 아래로 붙잡아 512M 한도에 닿지 않게 해요. Transmission의 512M 한도는 그대로 둬요(사용자 결정).

이 관찰과 자원 한도 재검토는 실제 서버를 며칠 운영해야 볼 수 있어요. 결과 목표 1의 완료 조건(cron 없이 같은 항목을 같은 폴더로 받는 것)에는 들지 않아서 0009에서 떼어 냈어요(사용자 결정, 2026-10-04).

## 작업

- 봇 토렌트 여럿을 며칠 받는 동안 Transmission 컨테이너의 `oom_kill`이 0에 머무는지 봐요. 2026-10-02 점검에서는 0이었지만, 그 사이 받은 것은 1.4GB 한 회차뿐이었어요. 2026-10-19 점검에서도 `oom_kill`은 0이었고 `transmission.slice`의 `memory.high`는 402653184(384M)였어요. 다만 컨테이너는 같은 날 12:27 UTC에 다시 시작했고 그 뒤로 쓴 양이 560KB 남짓이라, 컨테이너의 카운터만으로는 알 수 있는 게 적어요. slice의 `memory.events`는 slice가 활성화된 뒤 그 아래 컨테이너를 모두 합쳐 세는데, `high` 30455, `max` 0, `oom_kill` 0이었어요. 캐시가 384M를 30,455번 넘는 동안 512M 한도에는 한 번도 닿지 않았고, 죽은 적도 없다는 뜻이에요. slice가 활성화된 시각과 그 사이 받은 양은 확인하지 않았어요.
- 수정(RHEL-269060)이 들어간 `687.54.1` 이후의 커널로 올렸어요. 올린 뒤에도 `oom_kill`이 0인지 봐요. `transmission.slice`는 남겨 둬도 괜찮아요. 서버는 2026-10-19까지 `5.14.0-687.53.1.el9_8`이었고, 이 커널의 changelog에는 재시도 로직을 바꾼 `mm/memcg: refactor try_charge_memcg retry logic to use for loop`를 비롯한 RHEL-211058 항목 셋이 있었어요. 2026-10-19에 `5.14.0-687.58.1.el9_8`로 올리고 재부팅했어요. 새 커널의 changelog에는 이 리팩터링을 되돌린 `Revert "mm/memcg: refactor try_charge_memcg retry logic to use for loop"`(RHEL-269060)가 있고, 함께 들어왔던 sysctl 두 개를 되돌린 항목은 없어요. 재부팅 직후 slice의 `memory.events`는 `high`를 비롯해 모두 0이었어요. 받는 동안에도 `oom_kill`이 0인지는 첫 항목에서 이어서 봐요.
- `trss` 컨테이너의 메모리 한도를 다시 정해요. web과 worker가 두 컨테이너였을 때 한도는 웹 128M, worker 256M([0065](../../tickets/4-store-apply-and-replace/0065-archive-extraction.md)에서 압축 해제 때문에 128M에서 올렸어요)이었고, [0146](../../tickets/lighter-structure/0146-one-trss-process-and-container.md)이 둘을 합치며 두 한도를 더한 384M로 시작했어요. 2026-10-02 사용량은 worker 7.4MiB, web 3.9MiB로 한도의 6% 아래였어요. 2026-10-22에 0.8.0을 올리고 1분쯤 뒤 `docker stats`로 측정한 합친 프로세스의 사용량은 `trss` 14.26MiB로 한도의 4% 아래였고, 서버 브라우저 `trss-browser`는 13.86MiB로 한도 768M의 2% 아래였어요([0162](../../tickets/lighter-structure/0162-deploy-lighter-structure.md)). 큰 폴더 스캔과 표지 올리기 때의 최대치를 보고 정해요. 서버 브라우저([0039](../../tickets/3-subtitle-candidates-and-receiving/0039-browser-container-lifecycle.md))가 들어간 릴리스를 올리면 그 컨테이너(한도 768M)의 사용량도 함께 봐요.

## 완료 기준

| 관찰 | 기대 결과 |
| --- | --- |
| 봇 토렌트 여럿을 며칠 받음 | `memory.events`의 `oom_kill`이 0이고, 토렌트의 이름과 라벨이 되돌아가지 않아요. |
| 큰 폴더 스캔, 표지 올리기 | `trss`의 최대 메모리를 측정해요. 그 값으로 한도를 정해 compose에 반영해요. |

## 결과

2026-10-22에 이 티켓을 취소하고, 지켜볼 일 두 가지를 [열린 관찰 목록](../../../observations.md)의 [서버 Transmission의 OOM 강제 종료](../../../observations.md#서버-transmission의-oom-강제-종료)와 [`trss` 컨테이너의 메모리 한도](../../../observations.md#trss-컨테이너의-메모리-한도) 항목으로 옮겼어요(사용자 결정, 2026-10-22). 둘 다 서버를 며칠 운영해야 볼 수 있어서, 이 티켓은 끝나는 때가 정해지지 않은 관찰이었어요.

- 한 것: 2026-10-19에 서버의 커널을 수정(RHEL-269060)이 든 `5.14.0-687.58.1.el9_8`로 올렸어요. `transmission.slice`의 `MemoryHigh=384M`은 그대로 남아 있어요.
- 하지 않은 것: 받는 동안 `oom_kill`이 0에 머무는지 며칠 관찰하는 일과, `trss`의 최대 메모리를 측정해 한도를 다시 정하는 일이에요. 완료 기준의 두 줄 가운데 충족한 기록이 있는 것은 없어요.
