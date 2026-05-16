#!/usr/bin/env bash
# ============================================================
#  Basol — Auto Update Script for Ubuntu VPS
#  Usage:
#    bash update.sh            — pull latest + rebuild + restart
#    bash update.sh --no-restart  — pull + rebuild, skip restart
#    bash update.sh --status      — show service status only
# ============================================================
set -euo pipefail

# ── Colours ─────────────────────────────────────────────────
RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'
BLUE='\033[0;34m'; CYAN='\033[0;36m'; BOLD='\033[1m'; NC='\033[0m'

info()    { echo -e "${BLUE}[INFO]${NC}  $*"; }
success() { echo -e "${GREEN}[OK]${NC}    $*"; }
warn()    { echo -e "${YELLOW}[WARN]${NC}  $*"; }
error()   { echo -e "${RED}[ERROR]${NC} $*" >&2; exit 1; }
step()    { echo -e "\n${BOLD}${CYAN}▶ $*${NC}"; }

# ── Config (must match install.sh) ──────────────────────────
INSTALL_DIR="$HOME/basol"
SERVICE_NAME="basol"
BINARY_NAME="solana_analyzer"
BINARY_PATH="$INSTALL_DIR/target/release/$BINARY_NAME"

# ── Flags ───────────────────────────────────────────────────
NO_RESTART=false
STATUS_ONLY=false

for arg in "$@"; do
    case "$arg" in
        --no-restart) NO_RESTART=true ;;
        --status)     STATUS_ONLY=true ;;
        --help|-h)
            echo "Usage: bash update.sh [--no-restart] [--status] [--help]"
            echo "  (no flags)     Pull latest code, rebuild, restart service"
            echo "  --no-restart   Pull + rebuild without restarting the service"
            echo "  --status       Show service status and last 20 log lines only"
            exit 0
            ;;
        *) warn "Unknown flag: $arg — ignored" ;;
    esac
done

# ── Banner ───────────────────────────────────────────────────
echo -e "${BOLD}"
echo "══════════════════════════════════════════"
echo "   Basol — Auto Update for Ubuntu VPS    "
echo "══════════════════════════════════════════"
echo -e "${NC}"

# ── Status-only mode ────────────────────────────────────────
if [[ "$STATUS_ONLY" == true ]]; then
    echo -e "${BOLD}Service status:${NC}"
    sudo systemctl status "$SERVICE_NAME" --no-pager --lines=20 || true
    echo ""
    echo -e "${BOLD}Last 20 log lines:${NC}"
    sudo journalctl -u "$SERVICE_NAME" -n 20 --no-pager || true
    exit 0
fi

# ── Guard: must be an existing installation ──────────────────
if [[ ! -d "$INSTALL_DIR/.git" ]]; then
    error "No Basol installation found at $INSTALL_DIR\nRun install.sh first: bash install.sh"
fi

# ── 1. Record current commit for changelog ───────────────────
step "1/4  Checking for updates"

cd "$INSTALL_DIR"
export PATH="$HOME/.cargo/bin:$PATH"

BEFORE_HASH=$(git rev-parse --short HEAD)
CURRENT_BRANCH=$(git rev-parse --abbrev-ref HEAD)
info "Current: $BEFORE_HASH on branch $CURRENT_BRANCH"

# Fetch without merging first so we can show what's new
git fetch --quiet origin main

REMOTE_HASH=$(git rev-parse --short origin/main)

if [[ "$BEFORE_HASH" == "$REMOTE_HASH" ]]; then
    success "Already up to date ($BEFORE_HASH) — nothing to pull"
    echo ""
    # Still offer to rebuild in case binary is missing
    if [[ ! -f "$BINARY_PATH" ]]; then
        warn "Binary missing at $BINARY_PATH — will rebuild"
    else
        echo -e "  Binary  : ${CYAN}$BINARY_PATH${NC} (up to date)"
        echo -e "  Service : run ${CYAN}bash update.sh --status${NC} to check"
        echo ""
        exit 0
    fi
else
    info "Updates available — pulling ($BEFORE_HASH → $REMOTE_HASH)"
    git pull --ff-only origin main
    AFTER_HASH=$(git rev-parse --short HEAD)
    success "Code updated to $AFTER_HASH"
    echo ""
    echo -e "${BOLD}Changes pulled:${NC}"
    git log --oneline "$BEFORE_HASH".."$AFTER_HASH" 2>/dev/null || true
    echo ""
fi

# ── 2. Verify Rust toolchain ────────────────────────────────
step "2/4  Checking Rust toolchain"

if ! command -v cargo &>/dev/null; then
    error "cargo not found in PATH\nRe-run install.sh or: source \$HOME/.cargo/env"
fi
success "Rust $(rustc --version 2>/dev/null | cut -d' ' -f2) ready"

# ── 3. Build release binary ─────────────────────────────────
step "3/4  Building release binary"

START_TS=$(date +%s)

# Show only meaningful lines; rerun unfiltered on failure
set +e
BUILD_OUTPUT=$(cargo build --release 2>&1)
BUILD_EXIT=$?
set -e

if [[ $BUILD_EXIT -ne 0 ]]; then
    echo "$BUILD_OUTPUT"
    error "Build failed — see errors above. No changes applied to running service."
fi

END_TS=$(date +%s)
ELAPSED=$(( END_TS - START_TS ))

# Show only the summary lines from a successful build
echo "$BUILD_OUTPUT" | grep -E "^(   Compiling|    Finished|warning:)" | tail -5 || true

if [[ ! -f "$BINARY_PATH" ]]; then
    echo "$BUILD_OUTPUT"
    error "Binary not found after build — unexpected error"
fi

success "Binary built in ${ELAPSED}s — $BINARY_PATH"

# ── 4. Restart / report ─────────────────────────────────────
step "4/4  Applying update"

if [[ "$NO_RESTART" == true ]]; then
    warn "Skipping service restart (--no-restart flag set)"
    warn "Run:  sudo systemctl restart $SERVICE_NAME   to apply manually"
else
    if sudo systemctl is-active --quiet "$SERVICE_NAME" 2>/dev/null; then
        sudo systemctl restart "$SERVICE_NAME"
        # Brief pause so the new process has time to initialize
        sleep 2
        if sudo systemctl is-active --quiet "$SERVICE_NAME"; then
            success "Service '$SERVICE_NAME' restarted and running"
        else
            warn "Service restarted but may have failed — check logs below"
        fi
    elif sudo systemctl is-enabled --quiet "$SERVICE_NAME" 2>/dev/null; then
        warn "Service '$SERVICE_NAME' was not running — starting it now"
        sudo systemctl start "$SERVICE_NAME"
        sleep 2
        success "Service '$SERVICE_NAME' started"
    else
        warn "Service '$SERVICE_NAME' not found or not enabled"
        warn "Run install.sh first to register the service, or start manually:"
        warn "  $BINARY_PATH"
    fi
fi

# ── Done ─────────────────────────────────────────────────────
echo ""
echo -e "${BOLD}${GREEN}══════════════════════════════════════════${NC}"
echo -e "${BOLD}${GREEN}  Update complete!                        ${NC}"
echo -e "${BOLD}${GREEN}══════════════════════════════════════════${NC}"
echo ""
echo -e "  ${BOLD}Version:${NC}  $REMOTE_HASH"
echo -e "  ${BOLD}Binary:${NC}   $BINARY_PATH"
echo -e "  ${BOLD}Config:${NC}   $INSTALL_DIR/.env"
echo ""
echo -e "  ${BOLD}Useful commands:${NC}"
echo -e "  ${CYAN}sudo systemctl status $SERVICE_NAME${NC}       — check status"
echo -e "  ${CYAN}sudo journalctl -u $SERVICE_NAME -f${NC}       — live logs (Ctrl+C to exit)"
echo -e "  ${CYAN}sudo systemctl stop $SERVICE_NAME${NC}         — stop bot"
echo -e "  ${CYAN}bash $INSTALL_DIR/update.sh --status${NC}      — status + last 20 lines"
echo ""

# Show last few log lines so user can see startup immediately
echo -e "${BOLD}Last log lines:${NC}"
sudo journalctl -u "$SERVICE_NAME" -n 10 --no-pager 2>/dev/null || \
    info "(Service not running via systemd — no logs available)"
echo ""
