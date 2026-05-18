#!/usr/bin/env bash
# ============================================================
#  Basol — Update Script
#  Mendukung dua environment: Replit dan Ubuntu VPS (systemd)
#
#  Usage:
#    bash update.sh              — pull latest + rebuild + restart
#    bash update.sh --no-restart — pull + rebuild, skip restart
#    bash update.sh --status     — show status only
#    bash update.sh --help       — show help
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

# ── Detect environment ───────────────────────────────────────
IS_REPLIT=false
if [[ -n "${REPL_ID:-}" ]] || [[ -n "${REPLIT_DEV_DOMAIN:-}" ]]; then
    IS_REPLIT=true
fi

# ── Config (VPS mode) ────────────────────────────────────────
INSTALL_DIR="${BASOL_DIR:-$HOME/basol}"
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
            echo ""
            echo "  (no flags)     Pull latest code from GitHub, rebuild, restart bot"
            echo "  --no-restart   Pull + rebuild without restarting"
            echo "  --status       Show bot status and last 20 log lines"
            echo ""
            if [[ "$IS_REPLIT" == true ]]; then
                echo "  Running in: Replit environment"
                echo "  After update: restart the 'Basol Scanner' workflow manually"
            else
                echo "  Running in: VPS / server environment (systemd)"
            fi
            exit 0
            ;;
        *) warn "Unknown flag: $arg — ignored" ;;
    esac
done

# ── Banner ───────────────────────────────────────────────────
echo -e "${BOLD}"
echo "══════════════════════════════════════════"
if [[ "$IS_REPLIT" == true ]]; then
echo "   Basol — Update Script (Replit)         "
else
echo "   Basol — Update Script (VPS)            "
fi
echo "══════════════════════════════════════════"
echo -e "${NC}"

# ── Status-only mode ────────────────────────────────────────
if [[ "$STATUS_ONLY" == true ]]; then
    if [[ "$IS_REPLIT" == true ]]; then
        info "Running in Replit — check workflow status in the Replit panel"
        BINARY="./target/debug/$BINARY_NAME"
        [[ -f "./target/release/$BINARY_NAME" ]] && BINARY="./target/release/$BINARY_NAME"
        if [[ -f "$BINARY" ]]; then
            success "Binary found: $BINARY"
        else
            warn "Binary not built yet — run: cargo build"
        fi
        info "Current commit: $(git rev-parse --short HEAD 2>/dev/null || echo 'unknown')"
    else
        echo -e "${BOLD}Service status:${NC}"
        sudo systemctl status "$SERVICE_NAME" --no-pager --lines=20 || true
        echo ""
        echo -e "${BOLD}Last 20 log lines:${NC}"
        sudo journalctl -u "$SERVICE_NAME" -n 20 --no-pager || true
    fi
    exit 0
fi

# ── Set working directory ────────────────────────────────────
if [[ "$IS_REPLIT" == true ]]; then
    # In Replit, we're already in the project directory
    WORK_DIR="$(pwd)"
else
    if [[ ! -d "$INSTALL_DIR/.git" ]]; then
        error "No Basol installation found at $INSTALL_DIR\nRun install.sh first: bash install.sh"
    fi
    WORK_DIR="$INSTALL_DIR"
fi

cd "$WORK_DIR"
export PATH="$HOME/.cargo/bin:$PATH"

# ── 1. Git pull ──────────────────────────────────────────────
step "1/3  Pulling latest code from GitHub"

if [[ ! -d ".git" ]]; then
    error "Not a git repository. Clone the repo first:\n  git clone https://github.com/Unknown747/Baxsol ."
fi

BEFORE_HASH=$(git rev-parse --short HEAD 2>/dev/null || echo "unknown")
CURRENT_BRANCH=$(git rev-parse --abbrev-ref HEAD 2>/dev/null || echo "main")
info "Current: $BEFORE_HASH on branch $CURRENT_BRANCH"

# Auto-detect remote tracking branch
REMOTE_BRANCH=$(git rev-parse --abbrev-ref --symbolic-full-name @{u} 2>/dev/null \
    || echo "origin/$CURRENT_BRANCH")
REMOTE_NAME="${REMOTE_BRANCH%%/*}"
BRANCH_NAME="${REMOTE_BRANCH#*/}"
info "Remote: $REMOTE_BRANCH"

# Fetch without merging to compare
git fetch --quiet "$REMOTE_NAME" "$BRANCH_NAME" 2>/dev/null || {
    warn "Could not reach remote — check internet connection"
    warn "Continuing with local build..."
}

REMOTE_HASH=$(git rev-parse --short "$REMOTE_BRANCH" 2>/dev/null || echo "$BEFORE_HASH")

# Protect runtime files from being overwritten by git
RUNTIME_FILES=("bot_data.json" "paper_state.json" "config.env")
for f in "${RUNTIME_FILES[@]}"; do
    if git ls-files --error-unmatch "$f" &>/dev/null 2>&1; then
        git rm --cached -r --quiet "$f" 2>/dev/null || true
        info "Un-tracked from git: $f (file kept on disk)"
    fi
done

# Untrack entire target/ directory — build artifacts must never be in git
# This is the root cause of "target/.rustc_info.json: needs merge" conflicts
if git ls-files --error-unmatch "target/" &>/dev/null 2>&1 || \
   [[ -n "$(git ls-files target/ 2>/dev/null)" ]]; then
    git rm --cached -r --quiet target/ 2>/dev/null || true
    info "Un-tracked from git: target/ (build artifacts — kept on disk)"
fi

if [[ "$BEFORE_HASH" == "$REMOTE_HASH" ]]; then
    success "Already up to date ($BEFORE_HASH)"
    echo ""
    info "No new commits — rebuilding anyway to apply any local changes"
else
    info "Updates found: $BEFORE_HASH → $REMOTE_HASH"

    # Stash local changes so pull never aborts
    STASH_MSG="update.sh auto-stash $(date +%s)"
    STASHED=false
    if ! git diff --quiet || ! git diff --cached --quiet; then
        git stash push --quiet -m "$STASH_MSG"
        STASHED=true
        info "Local changes stashed temporarily"
    fi

    git pull --ff-only "$REMOTE_NAME" "$BRANCH_NAME"

    # Restore stash — keep local runtime files on conflict
    if [[ "$STASHED" == true ]]; then
        git stash pop --quiet 2>/dev/null || {
            warn "Stash pop had conflicts — keeping your local config files as-is"
            for f in "${RUNTIME_FILES[@]}"; do
                git checkout --theirs -- "$f" 2>/dev/null || true
            done
            git stash drop --quiet 2>/dev/null || true
        }
        info "Local changes restored"
    fi

    AFTER_HASH=$(git rev-parse --short HEAD)
    success "Updated to $AFTER_HASH"
    echo ""
    echo -e "${BOLD}Changes pulled:${NC}"
    git log --oneline "$BEFORE_HASH".."$AFTER_HASH" 2>/dev/null || true
    echo ""
fi

# Ensure config.env exists — create from example if missing
if [[ ! -f "config.env" ]] && [[ -f "config.env.example" ]]; then
    cp config.env.example config.env
    warn "config.env not found — created from config.env.example"
    warn "Review and edit config.env before running the bot"
fi

# ── 2. Build ─────────────────────────────────────────────────
step "2/3  Building"

if ! command -v cargo &>/dev/null; then
    error "cargo not found. Install Rust: https://rustup.rs"
fi
info "Rust $(rustc --version 2>/dev/null | cut -d' ' -f2)"

START_TS=$(date +%s)
set +e
if [[ "$IS_REPLIT" == true ]]; then
    # Debug build is faster and sufficient for Replit
    BUILD_OUTPUT=$(cargo build 2>&1)
else
    BUILD_OUTPUT=$(cargo build --release 2>&1)
fi
BUILD_EXIT=$?
set -e

if [[ $BUILD_EXIT -ne 0 ]]; then
    echo "$BUILD_OUTPUT"
    error "Build failed — see errors above. No changes applied."
fi

END_TS=$(date +%s)
ELAPSED=$(( END_TS - START_TS ))
echo "$BUILD_OUTPUT" | grep -E "^(   Compiling|    Finished|warning:)" | tail -5 || true
success "Build complete in ${ELAPSED}s"

# ── 3. Restart ───────────────────────────────────────────────
step "3/3  Restart"

if [[ "$NO_RESTART" == true ]]; then
    warn "Skipping restart (--no-restart)"
elif [[ "$IS_REPLIT" == true ]]; then
    echo ""
    echo -e "${BOLD}${YELLOW}Replit detected — manual restart required:${NC}"
    echo -e "  1. Di panel Replit, cari workflow ${CYAN}\"Basol Scanner\"${NC}"
    echo -e "  2. Klik ${CYAN}Stop${NC} lalu ${CYAN}Start${NC} (atau klik tombol Restart)"
    echo -e "  3. Bot akan mulai dengan kode terbaru"
    echo ""
    echo -e "  Atau jalankan langsung di terminal:"
    echo -e "  ${CYAN}cargo run${NC}"
else
    # VPS: restart via systemd
    if sudo systemctl is-active --quiet "$SERVICE_NAME" 2>/dev/null; then
        sudo systemctl restart "$SERVICE_NAME"
        sleep 2
        if sudo systemctl is-active --quiet "$SERVICE_NAME"; then
            success "Service '$SERVICE_NAME' restarted and running"
        else
            warn "Service restarted but may have failed — check logs below"
        fi
    elif sudo systemctl is-enabled --quiet "$SERVICE_NAME" 2>/dev/null; then
        warn "Service was not running — starting now"
        sudo systemctl start "$SERVICE_NAME"
        sleep 2
        success "Service '$SERVICE_NAME' started"
    else
        warn "Service '$SERVICE_NAME' not found or not enabled"
        warn "Run install.sh first, or start manually:"
        warn "  $BINARY_PATH"
    fi
fi

# ── Done ─────────────────────────────────────────────────────
echo ""
echo -e "${BOLD}${GREEN}══════════════════════════════════════════${NC}"
echo -e "${BOLD}${GREEN}  Update complete!                        ${NC}"
echo -e "${BOLD}${GREEN}══════════════════════════════════════════${NC}"
echo ""
echo -e "  ${BOLD}Version:${NC}  $(git rev-parse --short HEAD 2>/dev/null || echo 'unknown')"
echo -e "  ${BOLD}Config:${NC}   config.env"
echo ""

if [[ "$IS_REPLIT" != true ]]; then
    echo -e "  ${BOLD}Useful commands:${NC}"
    echo -e "  ${CYAN}sudo systemctl status $SERVICE_NAME${NC}        — check status"
    echo -e "  ${CYAN}sudo journalctl -u $SERVICE_NAME -f${NC}        — live logs"
    echo -e "  ${CYAN}sudo systemctl stop $SERVICE_NAME${NC}          — stop bot"
    echo -e "  ${CYAN}bash update.sh --status${NC}                    — status + last 20 lines"
    echo ""
    echo -e "${BOLD}Last log lines:${NC}"
    sudo journalctl -u "$SERVICE_NAME" -n 10 --no-pager 2>/dev/null || \
        info "(Service not running via systemd)"
fi
echo ""
