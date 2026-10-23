# 0011 0.8.0 피드백의 수정을 배포하고 서버에서 확인해요

- 상태: 진행 중 (2026-10-23, 마이그레이션 확인을 마쳤어요. 아래 "결과")
- 출처: [0.8.0을 써 보고 고칠 것](spec.md), [readme의 Releasing](../../../readme.md#releasing)
- 막는 티켓: [0001](0001-unconnected-subscription.md), [0002](0002-archive-a-work-from-its-detail.md), [0003](0003-archive-deletes-the-rule.md), [0004](0004-library-folder-choice.md), [0005](0005-library-list-remembers.md), [0006](0006-schedule-search-every-weekday.md), [0007](0007-rule-anissia-link.md), [0008](0008-subscription-cover.md), [0009](0009-reread-when-the-app-returns.md), [0010](0010-every-screen-listens.md)

## 작업

0001–0010을 담은 릴리스를 실제 서버(j4105)에 올리고, 사용자가 0.8.0에서 겪은 것이 서버에서 고쳐졌는지 봐요. 릴리스 태그, 이미지 게시, 배포는 사용자가 승인한 뒤에 해요.

[0003](0003-archive-deletes-the-rule.md)의 마이그레이션은 `보관됨` 규칙을 지워서 되돌릴 수 없어요. 그래서 올리기 전에 서버 DB의 새 복사본으로 마이그레이션을 확인하고([readme의 Releasing](../../../readme.md#releasing)), [0001](0001-unconnected-subscription.md)의 진단이 끝났는지 다시 봐요.

개발 환경에서 볼 수 없는 것은 서버와 사용자의 기기에서 봐요. iOS 홈 화면 앱의 확인은 사용자의 휴대폰이 필요해서 사용자에게 부탁해요.

- iOS 홈 화면 앱에서 할 일을 연 채 다른 앱에 갔다가 돌아왔을 때, 그 사이 바뀐 할 일이 보이는지(1번)
- iOS 홈 화면 앱과 PC의 Firefox에서 `작품 찾기`의 글자를 모두 지워도 포커스가 남는지(2번)
- iOS 홈 화면 앱에서 이번 주 편성의 표지가 보이는지(7번)

## 완료 기준

- [x] 올리기 전에 서버 DB의 새 복사본에서 마이그레이션 뒤 `보관됨` 규칙이 없고, 지운 규칙의 수집 이력과 시즌 연결이 남은 것을 확인해 결과에 적어요. 지운 개수와 남은 개수도 적어요.
- [ ] 새 릴리스가 서버에서 돌고, 다음 확인을 날짜와 버전과 함께 결과에 적어요. 보지 못한 것은 까닭과 함께 적어요.
  - 라이브러리의 `전체·수집·보관` 고르기와 `보관` 배지, 새로고침 뒤에도 남는 정렬·보기·필터·폴더 고르기
  - 작품 상세의 `보관 폴더로 옮기기` 하나와, 규칙 상세나 보관 제안의 `보관` 하나
  - 구독 화면의 `작품 찾기`가 다른 요일의 작품을 요일과 함께 찾는 것
  - 일반 규칙의 `Anissia 연결`의 `전체 목록` 탭
  - 첫 화 전의 구독 카드에 보이는 표지. 새 구독이 없으면 다음 분기 구독 때 봐요.
  - 위의 iOS 홈 화면 앱과 Firefox의 확인
- [ ] 0001–0010이 고친 검증 표의 행에 서버에서 본 것을 더해요.
- [ ] 이 묶음의 마지막 티켓이므로, `stora:completing-specs`로 [명세](spec.md)의 완료 기준을 티켓의 결과로 확인하고, 피드백 열한 가지가 모두 다뤄졌는지 명세에 적어요.

## 결과

### 마이그레이션 확인

2026-10-23에 사용자가 `dev/compose.sh pull --force`로 받은 서버 DB의 새 사본(0.8.0, `user_version` 66)을, 1c77bc14의 `trss_core::Db::open_blocking`만 부르는 작은 프로그램으로 워커 없이 마이그레이션했어요. `user_version`은 66에서 69가 됐고 33.87 ms 걸렸어요. `보관됨` 규칙 25개가 지워졌고 `활성` 규칙 7개는 그대로 남았어요. 마이그레이션 뒤 `rules`의 `state`는 `active`와 `paused`만 받아요.

| 표 | 전 | 뒤 | 지운 규칙의 것 |
| --- | --- | --- | --- |
| `rules` | 32 | 7 | 25 → 0 (일반 규칙 24, 시즌에 붙지 않은 구독 1) |
| `rule_subscriptions` | 6 | 5 | 1 → 0 |
| `rule_started` | 32 | 7 | 25 → 0 |
| `archive_suggestion_kept` | 0 | 0 | 0 |
| `history_items` | 677 | 677 | 16 → 16 |
| `seasons` | 146 | 146 | |
| `season_anissia` | 18 | 18 | 지운 규칙의 작품에 걸린 것은 전에도 0 |

지운 구독은 [0001](0001-unconnected-subscription.md)이 진단한 `정반대의 너와 나 2기`(규칙 `d45f0159-b9c2-40e7-b510-73b92c2ef8e7`)이고, 그 규칙이 받은 수집 이력 1개도 남았어요. 0001의 진단은 2026-10-22의 서버 DB 사본으로 이미 마쳤어요. 그 밖의 표는 행 수가 같고, 새 표 `subscription_artwork`는 비어 있어요. `PRAGMA integrity_check`는 `ok`, `PRAGMA foreign_key_check`는 0행이었어요. 원본 기록과 비교 스크립트는 이 PC의 `dev/local/measure/feedback-0.8.0-0011/`에 있어요.
