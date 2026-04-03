#!/bin/bash
# setup.sh — One-command setup for Minutes after cloning the repo.
#
# Usage:
#   git clone -b shipd https://github.com/searchformyusername/minutes.git
#   cd minutes
#   ./scripts/setup.sh
#
# What it does:
#   1. Checks/installs system dependencies (Rust, cmake, ffmpeg)
#   2. Builds the CLI binary with diarization support
#   3. Downloads the whisper transcription model
#   4. Downloads speaker diarization models
#   5. Verifies everything works
#
# Options:
#   --model <name>   Whisper model to download (default: medium)
#                    Options: tiny, base, small, medium, large-v3
#   --skip-deps      Skip system dependency checks
#   --skip-build     Skip building (only download models)

set -euo pipefail

# ── Defaults ─────────────────────────────────────────────────
MODEL="medium"
SKIP_DEPS=false
SKIP_BUILD=false

for arg in "$@"; do
    case "$arg" in
        --model)   shift; MODEL="${1:-medium}"; shift ;;
        --model=*) MODEL="${arg#*=}" ;;
        --skip-deps)  SKIP_DEPS=true ;;
        --skip-build) SKIP_BUILD=true ;;
        --help|-h)
            echo "Usage: ./scripts/setup.sh [--model small|base|tiny|medium|large-v3] [--skip-deps] [--skip-build]"
            exit 0
            ;;
    esac
done

# ── Colors ───────────────────────────────────────────────────
GREEN='\033[1;32m'
YELLOW='\033[1;33m'
RED='\033[1;31m'
CYAN='\033[1;36m'
DIM='\033[2m'
RESET='\033[0m'

step() { echo -e "\n${CYAN}=== $1 ===${RESET}"; }
ok()   { echo -e "  ${GREEN}✓${RESET} $1"; }
warn() { echo -e "  ${YELLOW}!${RESET} $1"; }
fail() { echo -e "  ${RED}✗${RESET} $1"; }

# ── Step 1: System dependencies ──────────────────────────────
if [ "$SKIP_DEPS" = false ]; then
    step "Checking system dependencies"

    # Rust
    if command -v cargo &>/dev/null; then
        ok "Rust $(cargo --version | cut -d' ' -f2)"
    else
        fail "Rust not found"
        echo "    Install: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
        echo "    Then restart your shell and re-run this script."
        exit 1
    fi

    # cmake (needed by whisper-rs build)
    if command -v cmake &>/dev/null; then
        ok "cmake $(cmake --version | head -1 | cut -d' ' -f3)"
    else
        warn "cmake not found — installing..."
        if command -v brew &>/dev/null; then
            brew install cmake
            ok "cmake installed"
        elif command -v apt-get &>/dev/null; then
            sudo apt-get install -y cmake
            ok "cmake installed"
        else
            fail "cmake not found and no package manager detected"
            echo "    Install cmake manually: https://cmake.org/download/"
            exit 1
        fi
    fi

    # ffmpeg (critical for audio decoding quality)
    if command -v ffmpeg &>/dev/null; then
        ok "ffmpeg $(ffmpeg -version 2>&1 | head -1 | cut -d' ' -f3)"
    else
        warn "ffmpeg not found — installing..."
        if command -v brew &>/dev/null; then
            brew install ffmpeg
            ok "ffmpeg installed"
        elif command -v apt-get &>/dev/null; then
            sudo apt-get install -y ffmpeg
            ok "ffmpeg installed"
        else
            warn "ffmpeg not found. Audio quality may be degraded for m4a/mp3 files."
            echo "    Install: brew install ffmpeg (macOS) / apt install ffmpeg (Linux)"
        fi
    fi
fi

# ── Step 2: Build the CLI ────────────────────────────────────
if [ "$SKIP_BUILD" = false ]; then
    step "Building Minutes CLI (this takes a few minutes on first build)"

    # macOS needs C++ include path for whisper.cpp compilation
    if [ "$(uname)" = "Darwin" ]; then
        SDK_PATH=$(xcrun --show-sdk-path 2>/dev/null || true)
        if [ -n "$SDK_PATH" ]; then
            export CXXFLAGS="-I${SDK_PATH}/usr/include/c++/v1"
        fi
    fi

    cargo install --path crates/cli --features diarize

    if command -v minutes &>/dev/null; then
        ok "minutes CLI installed ($(which minutes))"
    else
        fail "minutes binary not found on PATH"
        echo "    Make sure ~/.cargo/bin is in your PATH:"
        echo "    export PATH=\"\$HOME/.cargo/bin:\$PATH\""
        exit 1
    fi
fi

# ── Step 3: Download whisper model ───────────────────────────
step "Downloading whisper model ($MODEL)"
minutes setup --model "$MODEL"
ok "Whisper $MODEL model ready"

# ── Step 4: Download diarization models ──────────────────────
step "Downloading speaker diarization models"
minutes setup --diarization
ok "Diarization models ready"

# ── Step 5: Configure model in config.toml ───────────────────
step "Configuring Minutes"
CONFIG_DIR="$HOME/.config/minutes"
CONFIG_FILE="$CONFIG_DIR/config.toml"
mkdir -p "$CONFIG_DIR"

cat > "$CONFIG_FILE" << TOML
[transcription]
model = "$MODEL"

[diarization]
engine = "pyannote-rs"
threshold = 0.3
TOML
ok "Created $CONFIG_FILE"
echo -e "  ${DIM}  model = $MODEL${RESET}"
echo -e "  ${DIM}  diarization = pyannote-rs (threshold 0.3)${RESET}"

# ── Step 6: Verify ───────────────────────────────────────────
step "Verifying setup"

echo -e "  ${DIM}Running health check...${RESET}"
if minutes health 2>&1 | grep -qi "error\|fail"; then
    warn "Health check reported issues (see above). This may be fine for first setup."
else
    ok "Health check passed"
fi

# ── Done ─────────────────────────────────────────────────────
echo ""
echo -e "${GREEN}=== Setup complete! ===${RESET}"
echo ""
echo "  Quick start:"
echo -e "    ${CYAN}minutes devices${RESET}              # list available microphones"
echo -e "    ${CYAN}minutes enroll${RESET}               # enroll your voice (15s)"
echo -e "    ${CYAN}minutes record --title \"My Meeting\"${RESET}  # start recording"
echo -e "    ${CYAN}Ctrl-C${RESET}                       # stop and process"
echo -e "    ${CYAN}cat ~/meetings/*.md${RESET}           # read the transcript"
echo ""
echo -e "  Enroll other speakers for auto-identification:"
echo -e "    ${CYAN}minutes enroll --name \"Priya\"${RESET}"
echo -e "    ${CYAN}minutes enroll --name \"Meena\" --file ~/audio.m4a${RESET}"
echo ""
echo -e "  See all commands: ${CYAN}minutes --help${RESET}"
