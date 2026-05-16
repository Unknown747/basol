#!/usr/bin/env bash
# ============================================================
#  Basol — One-click Install / Update for Ubuntu VPS
#  Usage:
#    curl -fsSL https://raw.githubusercontent.com/Unknown747/Baxsol/main/install.sh | bash
#  or, if already cloned:
#    bash install.sh
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

# ── Config ───────────────────────────────────────────────────
REPO_URL="https://github.com/Unknown747/Baxsol"
INSTALL_DIR="$HOME/basol"
SERVICE_NAME="basol"
BINARY_NAME="solana_analyzer"

# ── Banner ───────────────────────────────────────────────────
echo -e "${BOLD}"
echo "══════════════════════════════════════════"
echo "   Basol — Solana Memecoin Trading Bot    "
echo "   One-click Install / Update for Ubuntu  "
echo "══════════════════════════════════════════"
echo -e "${NC}"

# ── Detect mode (install vs update) ─────────────────────────
IS_UPDATE=false
if [[ -d "$INSTALL_DIR/.git" ]]; then
    IS_UPDATE=true
    info "Existing installation found at $INSTALL_DIR — running UPDATE"
else
    info "No installation found — running FRESH INSTALL"
fi

# ── 1. System dependencies ───────────────────────────────────
step "1/6  Checking system dependencies"

PKGS_NEEDED=()
for pkg in curl git build-essential pkg-config libssl-dev; do
    if ! dpkg -s "$pkg" &>/dev/null 2>&1; then
        PKGS_NEEDED+=("$pkg")
    fi
done

if [[ ${#PKGS_NEEDED[@]} -gt 0 ]]; then
    info "Installing: ${PKGS_NEEDED[*]}"
    sudo apt-get update -qq
    sudo apt-get install -y -qq "${PKGS_NEEDED[@]}"
fi
success "System dependencies ready"

# ── 2. Rust ──────────────────────────────────────────────────
step "2/6  Checking Rust toolchain"

if ! command -v cargo &>/dev/null; then
    info "Rust not found — installing via rustup"
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
    # shellcheck source=/dev/null
    source "$HOME/.cargo/env"
    success "Rust installed: $(rustc --version)"
else
    info "Rust found: $(rustc --version)"
    rustup update stable -q
    success "Rust toolchain up to date"
fi

# Ensure cargo is in PATH for the rest of this script
export PATH="$HOME/.cargo/bin:$PATH"

# ── 3. Clone or update repo ──────────────────────────────────
step "3/6  Fetching latest code"

if [[ "$IS_UPDATE" == true ]]; then
    cd "$INSTALL_DIR"
    git pull --ff-only origin main
    success "Code updated"
else
    git clone "$REPO_URL" "$INSTALL_DIR"
    cd "$INSTALL_DIR"
    success "Repository cloned to $INSTALL_DIR"
fi

# ── 4. Configure .env ────────────────────────────────────────
step "4/6  Configuration"

ENV_FILE="$INSTALL_DIR/.env"

prompt_required() {
    local varname="$1"
    local prompt_text="$2"
    local current_val=""

    # Check if already set in .env
    if [[ -f "$ENV_FILE" ]]; then
        current_val=$(grep -E "^${varname}=" "$ENV_FILE" 2>/dev/null | cut -d= -f2- || true)
    fi

    if [[ -n "$current_val" && "$current_val" != *"your_"* && "$current_val" != *"_here"* ]]; then
        info "$varname already configured — skipping"
        return
    fi

    while true; do
        echo -en "${YELLOW}  → $prompt_text:${NC} "
        read -r value
        if [[ -n "$value" ]]; then
            # Write or update in .env
            if grep -q "^${varname}=" "$ENV_FILE" 2>/dev/null; then
                sed -i "s|^${varname}=.*|${varname}=${value}|" "$ENV_FILE"
            else
                echo "${varname}=${value}" >> "$ENV_FILE"
            fi
            break
        else
            warn "Value cannot be empty. Try again."
        fi
    done
}

prompt_optional() {
    local varname="$1"
    local prompt_text="$2"
    local default_val="$3"
    local current_val=""

    if [[ -f "$ENV_FILE" ]]; then
        current_val=$(grep -E "^${varname}=" "$ENV_FILE" 2>/dev/null | cut -d= -f2- || true)
    fi

    if [[ -n "$current_val" ]]; then
        return
    fi

    echo -en "${YELLOW}  → $prompt_text${NC} [${default_val}]: "
    read -r value
    value="${value:-$default_val}"

    if grep -q "^${varname}=" "$ENV_FILE" 2>/dev/null; then
        sed -i "s|^${varname}=.*|${varname}=${value}|" "$ENV_FILE"
    else
        echo "${varname}=${value}" >> "$ENV_FILE"
    fi
}

# Copy template if .env doesn't exist yet
if [[ ! -f "$ENV_FILE" ]]; then
    cp "$INSTALL_DIR/.env.example" "$ENV_FILE"
    info ".env created from template"
fi

echo ""
echo -e "${BOLD}Required API keys:${NC}"
prompt_required "HELIUS_API_KEY"      "Helius API key (get free at https://helius.dev)"
prompt_required "TELEGRAM_BOT_TOKEN"  "Telegram Bot Token (from @BotFather)"
prompt_required "TELEGRAM_CHAT_ID"    "Telegram Chat ID"

echo ""
echo -e "${BOLD}Wallet (only needed if TRADING_ENABLED=true):${NC}"
WALLET_VAL=""
if [[ -f "$ENV_FILE" ]]; then
    WALLET_VAL=$(grep -E "^WALLET_PRIVATE_KEY=" "$ENV_FILE" 2>/dev/null | cut -d= -f2- || true)
fi
if [[ -z "$WALLET_VAL" || "$WALLET_VAL" == *"your_"* ]]; then
    echo -en "${YELLOW}  → Wallet private key (leave blank to skip / paper trading only):${NC} "
    read -r wallet_val
    if [[ -n "$wallet_val" ]]; then
        if grep -q "^WALLET_PRIVATE_KEY=" "$ENV_FILE" 2>/dev/null; then
            sed -i "s|^WALLET_PRIVATE_KEY=.*|WALLET_PRIVATE_KEY=${wallet_val}|" "$ENV_FILE"
        else
            echo "WALLET_PRIVATE_KEY=${wallet_val}" >> "$ENV_FILE"
        fi
    fi
fi

echo ""
echo -e "${BOLD}Trading mode:${NC}"
prompt_optional "TRADING_ENABLED"       "Enable live trading? (true/false)" "false"
prompt_optional "PAPER_TRADING_ENABLED" "Enable paper trading? (true/false)" "true"
prompt_optional "PAPER_BALANCE_SOL"     "Paper trading virtual balance (SOL)" "1.0"

success ".env configured"

# ── 5. Build release binary ──────────────────────────────────
step "5/6  Building release binary (this takes 1-3 minutes on first build)"

cd "$INSTALL_DIR"
cargo build --release 2>&1 | grep -E "^(   Compiling|    Finished|error)" || true

BINARY_PATH="$INSTALL_DIR/target/release/$BINARY_NAME"
if [[ ! -f "$BINARY_PATH" ]]; then
    # Run again without filter to show full error
    cargo build --release
    error "Build failed — see errors above"
fi

success "Binary built: $BINARY_PATH"

# ── 6. Systemd service ───────────────────────────────────────
step "6/6  Setting up systemd service"

SERVICE_FILE="/etc/systemd/system/${SERVICE_NAME}.service"

sudo tee "$SERVICE_FILE" > /dev/null <<EOF
[Unit]
Description=Basol Solana Memecoin Trading Bot
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=$USER
WorkingDirectory=$INSTALL_DIR
EnvironmentFile=$ENV_FILE
ExecStart=$BINARY_PATH
Restart=on-failure
RestartSec=10
StandardOutput=journal
StandardError=journal
SyslogIdentifier=$SERVICE_NAME

[Install]
WantedBy=multi-user.target
EOF

sudo systemctl daemon-reload
sudo systemctl enable "$SERVICE_NAME" --quiet

# Restart if running, start if not
if sudo systemctl is-active --quiet "$SERVICE_NAME"; then
    sudo systemctl restart "$SERVICE_NAME"
    success "Service restarted"
else
    sudo systemctl start "$SERVICE_NAME"
    success "Service started"
fi

# ── Done ─────────────────────────────────────────────────────
echo ""
echo -e "${BOLD}${GREEN}══════════════════════════════════════════${NC}"
echo -e "${BOLD}${GREEN}  Basol is running!                       ${NC}"
echo -e "${BOLD}${GREEN}══════════════════════════════════════════${NC}"
echo ""
echo -e "  ${BOLD}Useful commands:${NC}"
echo -e "  ${CYAN}sudo systemctl status $SERVICE_NAME${NC}      — check status"
echo -e "  ${CYAN}sudo journalctl -u $SERVICE_NAME -f${NC}      — live logs"
echo -e "  ${CYAN}sudo systemctl stop $SERVICE_NAME${NC}        — stop bot"
echo -e "  ${CYAN}sudo systemctl restart $SERVICE_NAME${NC}     — restart bot"
echo -e "  ${CYAN}bash $INSTALL_DIR/install.sh${NC}             — update to latest"
echo ""
echo -e "  ${BOLD}Secrets (API keys):${NC}   $ENV_FILE"
echo -e "  ${BOLD}Strategy config:${NC}      $INSTALL_DIR/config.env   ← edit ini untuk ubah TP/SL/skor"
echo -e "  ${BOLD}Logs:${NC}                 sudo journalctl -u $SERVICE_NAME -n 50"
echo ""
warn "Setelah install pertama, cek dan sesuaikan: nano $INSTALL_DIR/config.env"
echo ""
