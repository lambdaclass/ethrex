# Tokamak Appchain Desktop App - Build Guide

Tauri 2.x 기반 데스크탑 앱을 macOS / Windows 실행파일로 빌드하는 방법을 설명합니다.

---

## 사전 요구사항

### 공통

| 도구 | 버전 | 설치 |
|------|------|------|
| Rust | 1.77.2+ | [rustup.rs](https://rustup.rs) |
| Node.js | 20+ | [nodejs.org](https://nodejs.org) |
| pnpm | 9+ | `npm install -g pnpm` |

### macOS 추가 요구사항

- **Xcode Command Line Tools**
  ```bash
  xcode-select --install
  ```

### Windows 추가 요구사항

- **Visual Studio Build Tools** (C++ 데스크톱 개발 워크로드 포함)
  - [다운로드](https://visualstudio.microsoft.com/visual-studio-build-tools/)
  - 설치 시 "C++를 사용한 데스크톱 개발" 체크
- **WebView2 Runtime** (Windows 10/11에는 기본 포함)
  - [다운로드](https://developer.microsoft.com/en-us/microsoft-edge/webview2/)

---

## 로컬 빌드

### 1. 프론트엔드 의존성 설치

```bash
cd crates/desktop-app/ui
pnpm install
```

### 2. 빌드 실행

```bash
pnpm tauri build
```

빌드가 완료되면 `src-tauri/target/release/bundle/` 아래에 결과물이 생성됩니다.

### 빌드 결과물

| 플랫폼 | 파일 | 설명 |
|--------|------|------|
| macOS | `dmg/Tokamak Appchain_0.1.0_aarch64.dmg` | 배포용 디스크 이미지 |
| macOS | `macos/Tokamak Appchain.app` | macOS 앱 번들 |
| Windows | `msi/Tokamak Appchain_0.1.0_x64_en-US.msi` | MSI 설치 프로그램 |
| Windows | `nsis/Tokamak Appchain_0.1.0_x64-setup.exe` | NSIS 설치 프로그램 |

### Apple Silicon / Intel 지정 빌드 (macOS)

```bash
# Apple Silicon (M1/M2/M3/M4)
pnpm tauri build --target aarch64-apple-darwin

# Intel Mac
pnpm tauri build --target x86_64-apple-darwin

# Universal Binary (양쪽 모두 지원)
rustup target add x86_64-apple-darwin
pnpm tauri build --target universal-apple-darwin
```

---

## 디버그 빌드

릴리스 최적화 없이 빠르게 빌드하려면:

```bash
pnpm tauri build --debug
```

결과물은 `src-tauri/target/debug/bundle/`에 생성됩니다.

---

## 앱 아이콘 변경

앱 아이콘을 변경하려면 1024x1024 PNG 이미지를 준비한 후:

```bash
pnpm tauri icon /path/to/app-icon.png
```

이 명령은 `src-tauri/icons/` 폴더에 모든 플랫폼용 아이콘 파일을 자동 생성합니다.

---

## GitHub Actions CI/CD (크로스 플랫폼 자동 빌드)

macOS에서 Windows를, Windows에서 macOS를 빌드할 수 없습니다(크로스 컴파일 미지원).
양쪽 플랫폼 모두 빌드하려면 GitHub Actions를 사용합니다.

`.github/workflows/build-desktop.yml` 파일을 프로젝트 루트에 생성합니다:

```yaml
name: Build Desktop App

on:
  push:
    tags: ['v*']
  workflow_dispatch:

permissions:
  contents: write

jobs:
  build:
    strategy:
      fail-fast: false
      matrix:
        include:
          - platform: macos-latest
            target: aarch64-apple-darwin
            label: macOS-ARM64
          - platform: macos-latest
            target: x86_64-apple-darwin
            label: macOS-x64
          - platform: windows-latest
            target: x86_64-pc-windows-msvc
            label: Windows-x64

    runs-on: ${{ matrix.platform }}
    name: Build (${{ matrix.label }})

    steps:
      - uses: actions/checkout@v4

      - uses: pnpm/action-setup@v4
        with:
          version: 9

      - uses: actions/setup-node@v4
        with:
          node-version: 20
          cache: pnpm
          cache-dependency-path: crates/desktop-app/ui/pnpm-lock.yaml

      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: ${{ matrix.target }}

      - name: Rust cache
        uses: swatinem/rust-cache@v2
        with:
          workspaces: crates/desktop-app/ui/src-tauri -> target

      - name: Install frontend dependencies
        run: pnpm install
        working-directory: crates/desktop-app/ui

      - name: Build Tauri app
        uses: tauri-apps/tauri-action@v0
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
        with:
          projectPath: crates/desktop-app/ui
          args: --target ${{ matrix.target }}

      - name: Upload artifacts
        uses: actions/upload-artifact@v4
        with:
          name: tokamak-appchain-${{ matrix.label }}
          path: |
            crates/desktop-app/ui/src-tauri/target/${{ matrix.target }}/release/bundle/**/*.dmg
            crates/desktop-app/ui/src-tauri/target/${{ matrix.target }}/release/bundle/**/*.app
            crates/desktop-app/ui/src-tauri/target/${{ matrix.target }}/release/bundle/**/*.msi
            crates/desktop-app/ui/src-tauri/target/${{ matrix.target }}/release/bundle/**/*.exe
```

### 사용 방법

- **태그 푸시 시 자동 빌드:** `git tag v0.1.0 && git push --tags`
- **수동 실행:** GitHub 리포지토리 > Actions 탭 > "Build Desktop App" > "Run workflow"
- **결과 다운로드:** Actions 실행 완료 후 Artifacts 섹션에서 다운로드

---

## macOS 코드 서명 (배포 시 필요)

서명 없이 빌드한 앱은 macOS Gatekeeper에 의해 차단됩니다. 배포용으로는 코드 서명이 필요합니다.

### 개발/테스트 시 서명 없이 실행

빌드한 `.app`을 직접 실행할 때 "확인되지 않은 개발자" 경고가 뜨면:

```bash
# 방법 1: 시스템 설정 > 개인 정보 보호 및 보안 > "확인 없이 열기"
# 방법 2: 터미널에서 quarantine 속성 제거
xattr -cr "src-tauri/target/release/bundle/macos/Tokamak Appchain.app"
```

### 배포용 서명 (Apple Developer 계정 필요)

GitHub Actions에서 서명하려면 다음 secrets를 설정합니다:

| Secret | 설명 |
|--------|------|
| `APPLE_CERTIFICATE` | .p12 인증서를 base64 인코딩한 값 |
| `APPLE_CERTIFICATE_PASSWORD` | 인증서 비밀번호 |
| `APPLE_SIGNING_IDENTITY` | 예: `Developer ID Application: Your Name (TEAMID)` |
| `APPLE_ID` | Apple ID 이메일 |
| `APPLE_PASSWORD` | 앱 전용 비밀번호 |
| `APPLE_TEAM_ID` | Apple Developer 팀 ID |

---

## 트러블슈팅

### `pnpm tauri build` 실패 시

```bash
# Rust 툴체인 업데이트
rustup update stable

# 프론트엔드 재설치
rm -rf node_modules && pnpm install

# Rust 빌드 캐시 정리
cd src-tauri && cargo clean
```

### Windows에서 WebView2 관련 오류

WebView2 Runtime이 설치되어 있는지 확인하고, 없으면 수동으로 설치합니다.

### macOS에서 "xcrun: error" 발생

```bash
xcode-select --install
sudo xcode-select --reset
```
