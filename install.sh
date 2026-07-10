#!/bin/bash
set -euo pipefail
IFS=$'\n\t'

# ── REM CLI Installer ──────────────────────────────────────────────────────
# Usage: curl -fsSL https://raw.githubusercontent.com/csy20/rem-cli/main/install.sh | bash
#
# Detects OS and architecture, downloads the matching binary from GitHub Releases,
# installs to ~/.local/bin/, and adds it to PATH if needed.
#
# Optional env vars:
#   VERSION=v0.4.0   pin a release tag (default: latest)
#   REPO=owner/name  override the GitHub repo (default: csy20/rem-cli)
#   INSTALL_DIR=...  install destination (default: ~/.local/bin)

REPO="${REPO:-csy20/rem-cli}"
VERSION="${VERSION:-latest}"
INSTALL_DIR="${INSTALL_DIR:-${HOME}/.local/bin}"
BINARY="rem"
BOLD="\033[1m"
GREEN="\033[32m"
YELLOW="\033[33m"
DIM="\033[2m"
RED="\033[31m"
RESET="\033[0m"

info()  { echo -e "  ${GREEN}✓${RESET} $*"; }
warn()  { echo -e "  ${YELLOW}!${RESET} $*"; }
header() { echo -e "\n${BOLD}┃ REM Installer${RESET} ${DIM}───────────────────────────${RESET}\n"; }
step()  { echo -e "  ${DIM}│${RESET} $*"; }

abort() {
    echo -e "  ${RED}✗${RESET} Error: $*" >&2
    exit 1
}

need_cmd() {
    command -v "$1" >/dev/null 2>&1 || abort "required command not found: $1"
}

header
need_cmd curl
need_cmd uname
need_cmd mktemp

# ── Detect platform ──────────────────────────────────────────────────────────
step "detecting platform..."

OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
ARCH="$(uname -m)"

case "$OS" in
    linux)  PLATFORM_OS="linux" ;;
    darwin) PLATFORM_OS="macos" ;;
    *)      abort "unsupported OS: $OS (only Linux and macOS are supported)" ;;
esac

case "$ARCH" in
    x86_64|amd64)   PLATFORM_ARCH="x86_64" ;;
    aarch64|arm64)  PLATFORM_ARCH="aarch64" ;;
    *)              abort "unsupported architecture: $ARCH" ;;
esac

TARGET="${PLATFORM_ARCH}-${PLATFORM_OS}"
step "platform: ${BOLD}${TARGET}${RESET}"

# ── Get latest version if not specified ───────────────────────────────────────
# Asset naming (must match .github/workflows/release.yml):
#   rem-x86_64-linux | rem-aarch64-linux | rem-x86_64-macos | rem-aarch64-macos
# Archives (optional): same name with .tar.gz containing a binary named "rem".
if [ "$VERSION" = "latest" ]; then
    step "fetching latest release..."
    # Avoid curl -f so we can print a clear message on 404 (no releases yet).
    API_URL="https://api.github.com/repos/${REPO}/releases/latest"
    HTTP_BODY="$(mktemp)"
    HTTP_CODE="$(curl -sSL -o "$HTTP_BODY" -w "%{http_code}" \
        -H "Accept: application/vnd.github+json" \
        -H "User-Agent: rem-installer" \
        "$API_URL" || true)"

    if [ "$HTTP_CODE" != "200" ]; then
        rm -f "$HTTP_BODY"
        abort "no GitHub release found for ${REPO} (HTTP ${HTTP_CODE}).
  Publish a release by pushing a version tag, e.g.:
    git tag v0.4.0 && git push origin v0.4.0
  Or install from source:
    git clone https://github.com/${REPO}.git && cd rem-cli && cargo install --path ."
    fi

    VERSION="$(sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$HTTP_BODY" | head -1)"
    rm -f "$HTTP_BODY"

    if [ -z "$VERSION" ]; then
        abort "could not parse latest version from GitHub API for ${REPO}"
    fi
fi
step "version: ${BOLD}${VERSION}${RESET}"

# ── Download binary ──────────────────────────────────────────────────────────
# Prefer a plain binary asset; fall back to .tar.gz if that is what the release ships.
ASSET_PLAIN="rem-${TARGET}"
ASSET_TARBALL="rem-${TARGET}.tar.gz"
BASE_URL="https://github.com/${REPO}/releases/download/${VERSION}"

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT
cd "$TMP_DIR"

download_asset() {
    local name="$1"
    local url="${BASE_URL}/${name}"
    step "trying ${name}..."
    if curl -fsSL --progress-bar "$url" -o "$name"; then
        return 0
    fi
    return 1
}

DOWNLOADED=""
if download_asset "$ASSET_PLAIN"; then
    DOWNLOADED="$ASSET_PLAIN"
elif download_asset "$ASSET_TARBALL"; then
    DOWNLOADED="$ASSET_TARBALL"
else
    abort "failed to download a binary for ${TARGET}.
  Looked for:
    ${BASE_URL}/${ASSET_PLAIN}
    ${BASE_URL}/${ASSET_TARBALL}
  Check that the release assets exist and match your platform."
fi

step "downloaded ${BOLD}${DOWNLOADED}${RESET}"

# Normalize to a file named "rem" in TMP_DIR
if [[ "$DOWNLOADED" == *.tar.gz ]]; then
    tar xzf "$DOWNLOADED"
    # Accept either "rem" or the platform-named binary inside the archive
    if [ -f rem ]; then
        :
    elif [ -f "$ASSET_PLAIN" ]; then
        mv "$ASSET_PLAIN" rem
    else
        # Take the first executable-looking file
        CANDIDATE="$(find . -maxdepth 2 -type f \( -name 'rem' -o -name 'rem-*' \) | head -1)"
        [ -n "$CANDIDATE" ] || abort "archive did not contain a rem binary"
        mv "$CANDIDATE" rem
    fi
else
    mv "$DOWNLOADED" rem
fi

chmod +x rem
# Basic sanity: file should be non-empty
[ -s rem ] || abort "downloaded binary is empty"

# ── Install ──────────────────────────────────────────────────────────────────
mkdir -p "$INSTALL_DIR"
if ! cp rem "$INSTALL_DIR/${BINARY}" 2>/dev/null; then
    # Fallback when cp fails (e.g. busy binary on some systems)
    mv rem "$INSTALL_DIR/${BINARY}" || abort "failed to install binary to ${INSTALL_DIR}/${BINARY}"
fi
chmod +x "$INSTALL_DIR/${BINARY}"
info "installed to ${BOLD}${INSTALL_DIR}/${BINARY}${RESET}"

# ── Add to PATH if needed ────────────────────────────────────────────────────
SHELL_NAME="$(basename "${SHELL:-bash}")"
SHELL_RC=""

case "$SHELL_NAME" in
    bash) SHELL_RC="${HOME}/.bashrc" ;;
    zsh)  SHELL_RC="${HOME}/.zshrc" ;;
    fish) SHELL_RC="${HOME}/.config/fish/config.fish" ;;
esac

path_has_install_dir() {
    case ":${PATH}:" in
        *":${INSTALL_DIR}:"*) return 0 ;;
        *) return 1 ;;
    esac
}

if [ -n "$SHELL_RC" ] && ! path_has_install_dir; then
    # Avoid duplicating the PATH line on re-install
    if [ -f "$SHELL_RC" ] && grep -F "$INSTALL_DIR" "$SHELL_RC" >/dev/null 2>&1; then
        warn "${BOLD}${INSTALL_DIR}${RESET} already referenced in ${SHELL_RC} (restart shell to pick it up)"
    else
        mkdir -p "$(dirname "$SHELL_RC")"
        if [ "$SHELL_NAME" = "fish" ]; then
            echo "fish_add_path $INSTALL_DIR" >> "$SHELL_RC"
        else
            echo "export PATH=\"${INSTALL_DIR}:\$PATH\"" >> "$SHELL_RC"
        fi
        warn "added ${BOLD}${INSTALL_DIR}${RESET} to ${SHELL_RC}"
        warn "restart your shell or run: ${BOLD}source ${SHELL_RC}${RESET}"
    fi
fi

# ── Done ─────────────────────────────────────────────────────────────────────
echo ""
info "REM CLI installed successfully!"
echo ""
echo "  Run:  ${BOLD}rem${RESET}       — start interactive chat"
echo "  Run:  ${BOLD}rem ask \"...\"${RESET}  — ask a coding question"
echo "  Run:  ${BOLD}rem new <name>${RESET} — scaffold a project"
echo ""
echo "  ${DIM}Requires Ollama: https://ollama.com${RESET}"
echo "  ${DIM}Recommended: ollama pull qwen2.5-coder:1.5b${RESET}"
echo ""

# ── Ollama environment hints ────────────────────────────────────────────────
if command -v ollama &>/dev/null; then
    warn "For low-RAM machines (4–6GB), set these env vars:"
    echo ""
    echo "  export OLLAMA_FLASH_ATTENTION=1    # 30-50% KV cache RAM savings"
    echo "  export OLLAMA_KV_CACHE_TYPE=q8_0   # half precision KV cache"
    echo "  export OLLAMA_MMAP=1               # mmap model load"
    echo "  export OLLAMA_MAX_LOADED_MODELS=1  # keep one model loaded"
    echo ""
fi
