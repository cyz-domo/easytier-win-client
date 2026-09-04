#!/usr/bin/env bash
# Local release build for EasyTier Windows Client on Linux.
#
# SCOPE LIMIT: the app is Windows-first. On Linux this script verifies the
# toolchain and produces installable packages (deb / AppImage / rpm per Tauri
# bundle config) of the *shell only* — the Windows service, named-pipe IPC and
# EasyTier core/drivers do not exist on Linux, so those features are absent.
# The packages are useful for build validation and UI preview, not production.
#
# Usage:
#   ./scripts/build-release.sh              # check env + build (host arch)
#   ./scripts/build-release.sh --fix        # apt-get install missing system deps
#   ./scripts/build-release.sh --skip-deps  # skip npm ci
#   OUT_DIR=../out ./scripts/build-release.sh

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

OUT_DIR="${OUT_DIR:-dist}"
FIX=0
SKIP_DEPS=0
FAILURES=()

for arg in "$@"; do
  case "$arg" in
    --fix) FIX=1 ;;
    --skip-deps) SKIP_DEPS=1 ;;
    -h|--help)
      sed -n '1,18p' "$0"
      exit 0
      ;;
    *)
      echo "Unknown option: $arg" >&2
      exit 2
      ;;
  esac
done

log()   { printf '\033[36m== %s ==\033[0m\n' "$*"; }
pass()  { printf '  \033[32m[OK]\033[0m %s\n' "$*"; }
fail()  { printf '  \033[31m[X]\033[0m %s\n' "$*"; FAILURES+=("$1"); }
warn()  { printf '  \033[33m[!]\033[0m %s\n' "$*"; }

APT_MISSING=()

check_node() {
  if command -v node >/dev/null 2>&1; then
    major="$(node --version | sed 's/^v//' | cut -d. -f1)"
    if [ "$major" -ge 18 ]; then pass "Node.js $(node --version)"; else fail "Node.js >= 18 required, found $(node --version)"; fi
  else
    fail "Node.js not found" ; APT_MISSING+=("nodejs npm")
  fi
}

check_rust() {
  if command -v cargo >/dev/null 2>&1; then
    pass "Cargo $(cargo --version | cut -d' ' -f2)"
  else
    fail "Rust/Cargo not found — install from https://rustup.rs"
  fi
}

# Tauri Linux system dependencies (Debian/Ubuntu names).
check_system_libs() {
  local libs=(
    libwebkit2gtk-4.1-dev
    libgtk-3-dev
    libayatana-appindicator3-dev
    librsvg2-dev
    libssl-dev
    build-essential
    curl
    file
  )
  for lib in "${libs[@]}"; do
    if dpkg-query -W -f='${Status}' "$lib" 2>/dev/null | grep -q "install ok installed"; then
      pass "$lib"
    else
      fail "system package missing: $lib"
      APT_MISSING+=("$lib")
    fi
  done
}

log "EasyTier Win Client — Linux build (shell-only packages)"

if [ "$(uname -s)" != "Linux" ]; then
  echo "This script targets Linux. On Windows use scripts/build-release.ps1." >&2
  exit 1
fi

if command -v apt-get >/dev/null 2>&1; then
  check_node; check_rust; check_system_libs
else
  warn "Non-apt distribution: system package checks skipped; verify webkit2gtk-4.1/gtk3/appindicator manually."
  check_node; check_rust
fi

if [ "${#FAILURES[@]}" -gt 0 ]; then
  echo ""
  echo "Environment check failed with ${#FAILURES[@]} problem(s)."
  if [ "$FIX" -eq 1 ] && [ "${#APT_MISSING[@]}" -gt 0 ] && command -v apt-get >/dev/null 2>&1; then
    echo "Installing missing packages: ${APT_MISSING[*]}"
    sudo apt-get update -y
    sudo apt-get install -y "${APT_MISSING[@]}"
  else
    echo "Re-run with --fix to install missing apt packages (Debian/Ubuntu)."
  fi
  if ! command -v cargo >/dev/null 2>&1; then exit 1; fi
fi

if [ "$SKIP_DEPS" -eq 0 ]; then
  log "Installing frontend dependencies"
  npm ci
fi

log "Running codec tests"
npm run test:codec

log "Building (release; several minutes)"
if [ "${EASYTIER_SKIP_SERVICE:-0}" = "1" ]; then
  warn "Skipping Windows service binary on Linux (EASYTIER_SKIP_SERVICE=1)."
else
  if cargo build --release --manifest-path src-tauri/Cargo.toml --bin easytier-service; then
    pass "service binary built (host compatibility check)"
  else
    echo "service build failed; Linux package build aborted" >&2
    exit 1
  fi
fi
npm exec tauri -- build

log "Collecting artifacts into $OUT_DIR/"
mkdir -p "$OUT_DIR"
find src-tauri/target/release/bundle -type f \( -name '*.deb' -o -name '*.AppImage' -o -name '*.rpm' \) -exec cp -v {} "$OUT_DIR/" \;

echo ""
log "Build complete"
ls -lh "$OUT_DIR"
warn "Reminder: Linux packages are build-validation shells — service/TUN/core features are Windows-only."
