# AI Assistant for Tokamak Appchain Platform

## Overview
웹 플랫폼(localhost:3000)에 플로팅 AI 챗봇을 추가하여 사용자가 앱체인에 대해 질문하고 안내를 받을 수 있도록 한다.

## UI Design
- **위치**: 화면 우하단 플로팅 버튼 (말풍선 아이콘)
- **클릭 시**: 채팅 패널 열림 (400px 너비, 화면 하단 고정)
- **닫기**: X 버튼 또는 외부 클릭
- **스타일**: 기존 UI 톤 (흰색 배경, 파란 액센트, rounded)

## Features

### Phase 1: Basic Q&A
- "앱체인이 뭐야?" — 일반 안내
- "ZK-DEX vs EVM L2 차이?" — 프로그램 비교
- "L2 만드는 방법" — Launch L2 가이드
- "어떤 앱체인이 인기 있어?" — Explore 데이터 기반 추천

### Phase 2: Context-Aware
- 현재 페이지 컨텍스트 인식 (Explore, Store, Detail 등)
- 앱체인 상세 페이지에서 "이 체인 설명해줘" → 해당 앱체인 정보 기반 응답
- 에러 발생 시 도움말 자동 제안

### Phase 3: Actions (Optional)
- "ZK-DEX로 L2 만들어줘" → Launch L2 페이지로 이동 + 자동 설정
- "이 앱체인 북마크해줘" → API 호출

## Backend

### API Endpoint
```
POST /api/ai/chat
Body: { message: string, context?: { page: string, appchainId?: string } }
Response: { reply: string }
```

### AI Provider (기존 환경변수 활용)
```env
TOKAMAK_AI_PROVIDER=openai      # OpenAI-compatible API
TOKAMAK_AI_BASE_URL=https://api.ai.tokamak.network
TOKAMAK_AI_API_KEY=sk-...
TOKAMAK_AI_MODEL=qwen3-235b
```

### System Prompt (예시)
```
You are the Tokamak Appchain Assistant. You help users:
- Understand appchains and L2 technology
- Navigate the platform (Explore, Store, Launch L2)
- Choose the right program (EVM L2, ZK-DEX, Tokamon)
- Troubleshoot deployment issues

Be concise, friendly, and use Korean or English based on the user's language.
Current page context: {context}
```

### RAG (Optional)
- 플랫폼 문서 (`platform/docs/`) 임베딩
- 앱체인 메타데이터 (이름, 설명, 해시태그) 검색
- 프로그램 설명 검색

## Client Components

### Files to Create
- `components/ai-assistant.tsx` — 플로팅 버튼 + 채팅 패널
- `lib/ai-api.ts` — AI API 호출 함수

### Files to Modify
- `app/layout.tsx` — `<AiAssistant />` 추가
- `lib/api.ts` — `aiApi.chat()` 추가

### Server Files to Create
- `routes/ai.js` — `/api/ai/chat` 엔드포인트
- `lib/ai-client.js` — OpenAI-compatible API 래퍼

## Dependencies
- 추가 패키지 없음 (OpenAI-compatible API는 fetch로 호출)
- 메신저 앱의 AI 구현 참고: `crates/desktop-app/ui/src/components/AiChat.tsx`

## Reference
- 메신저 앱 AI 설정: `TOKAMAK_AI_*` 환경변수
- 메신저 앱 AI 채팅: `crates/desktop-app/ui/src/components/AiChat.tsx`
- 플랫폼 클라이언트 .env.local에 이미 AI 환경변수 설정됨

## Branch
별도 브랜치에서 구현: `feat/ai-assistant`
