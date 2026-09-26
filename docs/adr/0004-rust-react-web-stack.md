# Rust 서버와 React 화면을 사용해요

기존 Rust·Tokio 기반 영상 처리 코드를 유지하고, 웹 서버는 axum, 화면은 React·TypeScript로 구현하기로 했어요.
작업 목록·자막 비교·승인·원격 인증 화면의 상태를 프런트엔드에서 구성하되, 작업과 파일 변경의 권위는 서버에 남겨요.
프런트엔드는 정적 번들로 빌드해 Rust 서버에서 제공하며, 운영용 Node 서버는 요구하지 않아요.

Svelte·TypeScript와 Rust 서버 HTML에 필요한 JavaScript만 더하는 대안도 비교했어요.
React 기반 UI 생태계를 사용하는 방향을 선택한 것이며, 다른 방식보다 성능이 우월하다고 검증한 결과는 아니에요.
[React의 앱 구성 안내](https://react.dev/learn/build-a-react-app-from-scratch)는 독립적인 프런트 빌드 구성을 설명해요.
세부 빌드 도구·UI 라이브러리와 내부 폴더 구조는 아직 정하지 않았어요.

웹과 작업은 [공통 라이브러리와 별도 바이너리](0006-separate-web-worker-binaries.md)로 구성하고, 같은 이미지의 별도 컨테이너에서 실행해요.
현재 요구는 [구현 명세](../specs/web-gui-subtitles.md)에 있고, 이 기록은 구현 완료를 뜻하지 않아요.
