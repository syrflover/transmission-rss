# 0042 Blogger·Google Drive의 개별 자막과 회차 ZIP을 받아요

- 상태: 대기
- 출처: [출처별 다운로드](../specs/subtitles.md#출처별-다운로드)(Blogger·Google Drive 개별 자막, 회차 ZIP), [Blogger·Google Drive 조사](../brainstorm/web-gui-subtitles.md#bloggergoogle-drive)
- 막는 티켓: [0038](0038-receive-result-tistory.md), [0039](0039-browser-container-lifecycle.md)(0033이 브라우저 경로가 필요하다고 정하면)

## 작업

Blogger 게시물에서 연결된 Google Drive 파일을 받아요. 게시물에 개별 회차 파일과 회차 ZIP이 함께 있을 수 있으므로, 후보의 회차와 사용자가 고른 범위에 맞는 파일을 고르는 규칙은 0033의 관찰을 따라요.
회차 ZIP에서 고른 범위만 적용하는 일은 결과 목표 4의 묶음 분석이고, 이 티켓은 두 종류를 각각 수신 영역까지 받는 것이에요.
개별 파일 수신의 성공으로 ZIP 처리를 완료로 보지 않아요.

## 완료 기준

| 입력·상태 | 기대 결과 |
| --- | --- |
| 개별 자막 링크가 있는 게시물(가짜 출처) | 고른 회차의 파일만 받아요. |
| 회차 ZIP 링크가 있는 게시물(가짜 출처) | ZIP을 받고, 작업 상세에 묶음으로 받았다고 보여요. |
| Drive가 파일 대신 확인 페이지나 오류 HTML을 줌 | 받음이 아니라 분류된 실패예요. |
| 실제 게시물(개별 하나, ZIP 하나) | 실제 서버에서 받은 두 파일의 크기·형식을 확인한 결과가 있어요. |
