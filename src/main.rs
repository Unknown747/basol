// ============================================================
// Solana Token Analysis Bot - Auto Buy/Auto Sell
// ============================================================

mod positions;
mod wallet;
mod strategy;
mod sell_strategy;
mod paper_trading;
mod backtest;

use positions::Position;
use wallet::WalletManager;
use strategy::{TradingConfig, BuySignal, BuyDecision, evaluate_buy_signal, log_buy_decision};
use sell_strategy::{SellDecision, evaluate_all_positions, format_buy_notification, format_sell_notification};
use paper_trading::{
    PaperConfig, PaperTradingState,
    format_paper_buy_notification, format_paper_sell_notification,
    format_paper_report, save_paper_state, load_paper_state,
};

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};
use tokio::time::sleep;
use serde::{Deserialize, Serialize};
use reqwest::Client;
use chrono::{DateTime, Utc, Datelike, Timelike};
use std::fs;

// ============================================================
// CONFIG - Bot configuration constants
// ============================================================
const SCAN_INTERVAL_SECS: u64          = 15;   // 30→15: 2x faster new token discovery
const PROFIT_CHECK_INTERVAL_SECS: u64  = 300;
const SELL_CHECK_INTERVAL_SECS: u64    = 20;   // 60→20: 3x faster SL/TP trigger, minimize rug pull gap
const SOL_PRICE_UPDATE_INTERVAL_SECS: u64 = 300; // Refresh SOL price every 5 minutes
const SAVE_INTERVAL_MINS: i64     = 10;
const MAX_TOKEN_AGE_HOURS: i64    = 6;
// Score thresholds are derived from TradingConfig::min_score_to_buy (read from MIN_SCORE_TO_BUY env)
// New tokens (<6h old) get a 5-point lower threshold to compensate for less available data.
// Do NOT hardcode these — always use self.config.min_score_to_buy at call site.
const MAX_DAILY_ALERTS: u32       = 15;

// ============================================================
// STRUCTURES - DexScreener
// ============================================================

#[derive(Debug, Deserialize, Clone)]
struct DexToken {
    #[serde(rename = "chainId")]
    chain_id: String,
    #[serde(rename = "tokenAddress")]
    token_address: String,
    name: Option<String>,
    symbol: Option<String>,
    description: Option<String>,
    #[serde(rename = "imageUrl")]
    image_url: Option<String>,
    #[serde(rename = "createdAt")]
    created_at: Option<serde_json::Value>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize, Clone)]
struct PairData {
    #[serde(rename = "chainId")]
    chain_id: String,
    #[serde(rename = "dexId")]
    dex_id: String,
    url: String,
    #[serde(rename = "pairAddress")]
    pair_address: String,
    #[serde(rename = "baseToken")]
    base_token: TokenBasicInfo,
    #[serde(rename = "priceUsd")]
    price_usd: Option<String>,
    #[serde(rename = "pairCreatedAt")]
    pair_created_at: Option<i64>,
    liquidity: Option<LiquidityInfo>,
    volume: Option<VolumeInfo>,
    #[serde(rename = "priceChange")]
    price_change: Option<PriceChangeInfo>,
    fdv: Option<f64>,
    #[serde(rename = "marketCap")]
    market_cap: Option<f64>,
    txns: Option<TxnsInfo>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize, Clone)]
struct TokenBasicInfo {
    address: String,
    name: String,
    symbol: String,
}

#[derive(Debug, Deserialize, Clone)]
struct LiquidityInfo {
    usd: Option<f64>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize, Clone)]
struct VolumeInfo {
    #[serde(rename = "h24")]
    h24: Option<f64>,
    #[serde(rename = "h1")]
    h1: Option<f64>,
    #[serde(rename = "m5")]
    m5: Option<f64>,
}

#[derive(Debug, Deserialize, Clone)]
struct PriceChangeInfo {
    #[serde(rename = "m5")]
    m5: Option<f64>,
    #[serde(rename = "h1")]
    h1: Option<f64>,
    #[serde(rename = "h6")]
    h6: Option<f64>,
    #[serde(rename = "h24")]
    h24: Option<f64>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize, Clone)]
struct TxnsInfo {
    #[serde(rename = "h1")]
    h1: Option<TxnData>,
    #[serde(rename = "m5")]
    m5: Option<TxnData>,
}

#[derive(Debug, Deserialize, Clone)]
struct TxnData {
    buys: Option<u32>,
    sells: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct PairResponse {
    pairs: Option<Vec<PairData>>,
}

// ============================================================
// STRUCTURES - Helius API
// ============================================================

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct HeliusTokenInfo {
    #[serde(rename = "mintAuthority")]
    mint_authority: Option<String>,
    #[serde(rename = "freezeAuthority")]
    freeze_authority: Option<String>,
    decimals: Option<u8>,
    supply: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct HeliusTokenHolder {
    address: String,
    amount: String,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct HeliusTransaction {
    signature: String,
    timestamp: i64,
    #[serde(rename = "feePayer")]
    fee_payer: Option<String>,
    #[serde(rename = "tokenTransfers")]
    token_transfers: Option<Vec<TokenTransfer>>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize, Clone)]
struct TokenTransfer {
    #[serde(rename = "fromUserAccount")]
    from_user_account: Option<String>,
    #[serde(rename = "toUserAccount")]
    to_user_account: Option<String>,
    #[serde(rename = "tokenAmount")]
    token_amount: f64,
    mint: Option<String>,
}

// ============================================================
// STRUCTURES - Analysis Results
// ============================================================

#[allow(dead_code)]
#[derive(Debug, Clone)]
struct HolderAnalysis {
    total_holders: u32,
    sniper_count: u32,
    sniper_percentage: f64,
    top10_concentration: f64,
    gini_coefficient: f64,
    bundled_wallets_detected: bool,
    developer_sold_percentage: f64,
    score: f64,
    flags: Vec<String>,
    signals: Vec<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
struct LiquidityAnalysis {
    total_usd: f64,
    lp_burned: bool,
    lp_locked: bool,
    lock_duration_months: Option<f64>,
    independent_providers: u32,
    price_impact_5k: f64,
    price_impact_10k: f64,
    price_impact_25k: f64,
    score: f64,
    flags: Vec<String>,
    signals: Vec<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
struct WhaleAnalysis {
    smart_wallets_entered: u32,
    accumulation_pattern: bool,
    distribution_signs: bool,
    cold_storage_transfers: u32,
    largest_single_buy_usd: f64, // NOTE: holds raw token units, not USD (field name is a misnomer)
    score: f64,
    signals: Vec<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
struct TechnicalAnalysis {
    momentum_5m: f64,
    momentum_1h: f64,
    momentum_6h: f64,
    momentum_24h: f64,
    volume_24h: f64,
    buy_pressure_ratio: f64,
    pattern_detected: Option<String>,
    support_level: Option<f64>,
    resistance_level: Option<f64>,
    score: f64,
    signals: Vec<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
struct ContractSecurity {
    mint_authority_revoked: bool,
    freeze_authority_revoked: bool,
    /// true  = Helius returned data we could parse
    /// false = Helius call failed / token not yet indexed
    metadata_available: bool,
    transfer_fee_percent: f64,
    honeypot_risk: bool,
    score: f64,
    flags: Vec<String>,
    signals: Vec<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
struct SocialAnalysis {
    has_twitter: bool,
    has_telegram: bool,
    social_hype_score: f64,
    score: f64,
    signals: Vec<String>,
}

#[derive(Debug, Clone)]
struct TokenLifecycle {
    age_minutes: i64,
    phase: LifecyclePhase,
    entry_timing_score: f64,
    risk_reward_ratio: f64,
}

#[derive(Debug, Clone, PartialEq)]
enum LifecyclePhase {
    Launch,
    FirstDip,
    Accumulation,
    Breakout,
    Mature,
}

impl LifecyclePhase {
    fn to_label(&self) -> &str {
        match self {
            LifecyclePhase::Launch       => "⚡ Launch Phase (high risk)",
            LifecyclePhase::FirstDip     => "📉 First Dip (opportunity)",
            LifecyclePhase::Accumulation => "🟢 Accumulation Phase (ideal entry)",
            LifecyclePhase::Breakout     => "🔥 Breakout Phase (late entry)",
            LifecyclePhase::Mature       => "⬜ Mature (too late)",
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
struct FullTokenAnalysis {
    token_address: String,
    symbol: String,
    name: String,
    image_url: Option<String>,
    price_usd: Option<f64>,
    market_cap: Option<f64>,
    dex_urls: Vec<String>,
    total_score: f64,
    confidence_level: ConfidenceLevel,
    potential_multiplier: String,
    holder_analysis: HolderAnalysis,
    liquidity_analysis: LiquidityAnalysis,
    whale_analysis: WhaleAnalysis,
    technical_analysis: TechnicalAnalysis,
    contract_security: ContractSecurity,
    social_analysis: SocialAnalysis,
    lifecycle: TokenLifecycle,
    alert_level: AlertLevel,
    all_red_flags: Vec<String>,
    top_signals: Vec<String>,
}

#[derive(Debug, Clone)]
enum ConfidenceLevel {
    VeryHigh,
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone)]
enum AlertLevel {
    Legendary,
    Golden,
    Excellent,
    Normal,
}

impl AlertLevel {
    fn to_header(&self) -> &str {
        match self {
            AlertLevel::Legendary => "👑 **LEGENDARY OPPORTUNITY** 👑",
            AlertLevel::Golden    => "💎 **GOLDEN OPPORTUNITY** 💎",
            AlertLevel::Excellent => "🔥 **EXCELLENT OPPORTUNITY** 🔥",
            AlertLevel::Normal    => "⭐ **STANDARD OPPORTUNITY** ⭐",
        }
    }
}

// ============================================================
// STRUCTURES - Tracking & Persistence
// ============================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TrackedToken {
    token_address: String,
    symbol: String,
    name: String,
    image_url: Option<String>,
    initial_price: f64,
    highest_price: f64,
    discovery_time: DateTime<Utc>,
    milestones_reached: Vec<u32>,
}

#[derive(Debug, Serialize, Deserialize)]
struct BotPersistentData {
    seen_tokens: HashMap<String, String>,
    tracked_tokens: HashMap<String, TrackedToken>,
    smart_wallets: Vec<String>,
    daily_alert_count: u32,
    last_reset_date: String,
    performance_stats: PerformanceStats,
    positions: HashMap<String, PositionData>,
    #[serde(default)]
    blacklisted_tokens: HashSet<String>,
    // Daily max loss protection — persisted so a restart cannot bypass the daily limit.
    // Uses String for NaiveDate to stay serde-compatible across versions.
    #[serde(default)]
    daily_loss_sol: f64,
    #[serde(default)]
    daily_loss_date_str: String,
    #[serde(default)]
    daily_limit_paused: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PositionData {
    token_address: String,
    symbol: String,
    name: String,
    buy_price_usd: f64,
    amount_in_sol: f64,
    token_amount: f64,
    highest_price: f64,
    trailing_stop_active: bool,
    trailing_stop_price: f64,
    entry_time: DateTime<Utc>,
    score_at_entry: f64,
    // Pool liquidity at entry — used for sell-side price impact calculation
    #[serde(default)]
    liquidity_at_entry: f64,
    // Persisted TP state — prevents TP1/TP2 from re-firing after a bot restart
    #[serde(default)]
    tp1_fired: bool,
    #[serde(default)]
    tp2_fired: bool,
}

impl From<&Position> for PositionData {
    fn from(p: &Position) -> Self {
        Self {
            token_address: p.token_address.clone(),
            symbol: p.symbol.clone(),
            name: p.name.clone(),
            buy_price_usd: p.buy_price_usd,
            amount_in_sol: p.amount_in_sol,
            token_amount: p.token_amount,
            highest_price: p.highest_price,
            trailing_stop_active: p.trailing_stop_active,
            trailing_stop_price: p.trailing_stop_price,
            entry_time: p.entry_time,
            score_at_entry: p.score_at_entry,
            liquidity_at_entry: p.liquidity_at_entry,
            tp1_fired: p.tp1_fired,
            tp2_fired: p.tp2_fired,
        }
    }
}

impl From<PositionData> for Position {
    fn from(d: PositionData) -> Self {
        Self {
            token_address: d.token_address,
            symbol: d.symbol,
            name: d.name,
            buy_price_usd: d.buy_price_usd,
            amount_in_sol: d.amount_in_sol,
            token_amount: d.token_amount,
            highest_price: d.highest_price,
            trailing_stop_active: d.trailing_stop_active,
            trailing_stop_price: d.trailing_stop_price,
            entry_time: d.entry_time,
            score_at_entry: d.score_at_entry,
            liquidity_at_entry: d.liquidity_at_entry,
            tp1_fired: d.tp1_fired,
            tp2_fired: d.tp2_fired,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct PerformanceStats {
    total_alerts_sent: u32,
    tokens_reached_2x: u32,
    tokens_reached_5x: u32,
    tokens_reached_10x: u32,
    best_token_symbol: String,
    best_token_gain_percent: f64,
    total_buys: u32,
    total_sells: u32,
    total_profit_sol: f64,
    total_loss_sol: f64,
}

// ============================================================
// STRUCTURES - Telegram
// ============================================================

#[derive(Debug, Serialize)]
struct TelegramMsg {
    chat_id: String,
    text: String,
    parse_mode: String,
    disable_web_page_preview: bool,
}

#[derive(Debug, Serialize)]
struct TelegramPhoto {
    chat_id: String,
    photo: String,
    caption: String,
    parse_mode: String,
}

#[derive(Debug, Serialize)]
struct TelegramInlineKeyboard {
    chat_id: String,
    text: String,
    parse_mode: String,
    reply_markup: InlineKeyboardMarkup,
}

#[derive(Debug, Serialize)]
struct InlineKeyboardMarkup {
    inline_keyboard: Vec<Vec<InlineButton>>,
}

#[derive(Debug, Serialize)]
struct InlineButton {
    text: String,
    url: Option<String>,
    callback_data: Option<String>,
}

// ============================================================
// RATE LIMITER
// ============================================================

// ============================================================
// HELIUS KEY POOL - Multi-key rotation with auto rate-limit detection
// ============================================================

struct HeliusKeyPool {
    keys: Vec<String>,
    current_index: usize,
    rate_limited_until: Vec<Option<Instant>>,
    // Per-key rate limiter — each key gets its own 40 req/min bucket so
    // multiple keys give proportionally higher throughput (N keys × 40 rpm).
    limiters: Vec<RateLimiter>,
}

impl HeliusKeyPool {
    fn from_env() -> Self {
        let mut keys: Vec<String> = Vec::new();

        // Primary key — supports single key or comma-separated list (e.g. "key1,key2,key3")
        if let Ok(raw) = std::env::var("HELIUS_API_KEY") {
            for k in raw.split(',') {
                let k = k.trim().to_string();
                if !k.is_empty() && k != "your_helius_api_key_here" && !keys.contains(&k) {
                    keys.push(k);
                }
            }
        }

        // Additional keys: HELIUS_API_KEY_2, HELIUS_API_KEY_3, ..., HELIUS_API_KEY_10
        for i in 2..=10 {
            if let Ok(k) = std::env::var(format!("HELIUS_API_KEY_{i}")) {
                let k = k.trim().to_string();
                if !k.is_empty() && !keys.contains(&k) {
                    keys.push(k);
                }
            }
        }

        // Also support comma-separated HELIUS_API_KEYS
        if let Ok(all) = std::env::var("HELIUS_API_KEYS") {
            for k in all.split(',') {
                let k = k.trim().to_string();
                if !k.is_empty() && !keys.contains(&k) {
                    keys.push(k);
                }
            }
        }

        if keys.is_empty() {
            panic!("HELIUS_API_KEY must be set (see .env.example)");
        }

        let len = keys.len();
        println!("[HELIUS] Loaded {len} key(s) for rotation");
        // Each key gets its own 40 req/min bucket — N keys = N × 40 rpm capacity.
        let limiters = (0..len).map(|_| RateLimiter::new(40, 60)).collect();
        Self {
            keys,
            current_index: 0,
            rate_limited_until: vec![None; len],
            limiters,
        }
    }

    fn current(&self) -> &str {
        &self.keys[self.current_index]
    }

    // Throttle requests for the active key only — respects its own 40 rpm budget.
    async fn wait_for_current(&mut self) {
        self.limiters[self.current_index].wait_if_needed().await;
    }

    fn key_count(&self) -> usize {
        self.keys.len()
    }

    // Returns masked key for safe logging (first 8 chars + ***)
    fn masked(&self, index: usize) -> String {
        let k = &self.keys[index];
        if k.len() > 8 {
            format!("{}***", &k[..8])
        } else {
            "***".to_string()
        }
    }

    // Mark current key as rate-limited and rotate to next available key
    fn rotate_on_rate_limit(&mut self) {
        let now = Instant::now();
        self.rate_limited_until[self.current_index] = Some(now + Duration::from_secs(65));
        let old_index = self.current_index;
        let start = self.current_index;
        let mut all_limited = false;

        loop {
            self.current_index = (self.current_index + 1) % self.keys.len();
            if self.current_index == start {
                all_limited = true;
                break;
            }
            let available = match self.rate_limited_until[self.current_index] {
                None => true,
                Some(until) if Instant::now() >= until => {
                    self.rate_limited_until[self.current_index] = None;
                    true
                }
                _ => false,
            };
            if available {
                break;
            }
        }

        if all_limited {
            println!(
                "[HELIUS] ⚠️  All {} key(s) rate-limited — waiting for key #{} to recover",
                self.keys.len(), self.current_index + 1
            );
        } else {
            println!(
                "[HELIUS] 429 on key #{} ({}) → rotated to key #{} ({})",
                old_index + 1, self.masked(old_index),
                self.current_index + 1, self.masked(self.current_index),
            );
        }
    }

    // Build status string showing real-time rate-limit state of each key
    #[allow(dead_code)]
    fn status_summary(&self) -> String {
        let now = Instant::now();
        self.keys.iter().enumerate().map(|(i, _)| {
            let label = if i == self.current_index { "▶" } else { "  " };
            let state = match self.rate_limited_until[i] {
                None => "✅ OK".to_string(),
                Some(until) if now >= until => "✅ OK (recovered)".to_string(),
                Some(until) => {
                    let secs = until.duration_since(now).as_secs();
                    format!("⏳ Rate-limited ({secs}s left)")
                }
            };
            format!("{} Key #{}: {} — {}", label, i + 1, self.masked(i), state)
        }).collect::<Vec<_>>().join("\n")
    }
}

struct RateLimiter {
    calls: Vec<Instant>,
    max_calls: usize,
    window_secs: u64,
}

impl RateLimiter {
    fn new(max_calls: usize, window_secs: u64) -> Self {
        Self { calls: Vec::new(), max_calls, window_secs }
    }

    async fn wait_if_needed(&mut self) {
        let now = Instant::now();
        let window = Duration::from_secs(self.window_secs);
        self.calls.retain(|t| now.duration_since(*t) < window);
        if self.calls.len() >= self.max_calls {
            let oldest = self.calls[0];
            let wait = window.checked_sub(now.duration_since(oldest))
                .unwrap_or(Duration::from_millis(100));
            sleep(wait).await;
            self.calls.retain(|t| Instant::now().duration_since(*t) < window);
        }
        self.calls.push(Instant::now());
    }
}

// ============================================================
// OFF-PEAK SAVED STATE - for restoring thresholds after off-peak scan
// ============================================================

struct OffPeakSavedState {
    min_score: f64,
    paper_min_score: f64,
    min_liquidity: f64,
    max_positions: usize,
    paper_max_positions: usize,
    momentum_max_pct: f64,
}

// ============================================================
// MAIN BOT STRUCT - with trading integration
// ============================================================

struct SolanaBot {
    client: Client,
    data: BotPersistentData,
    dex_limiter: RateLimiter,
    tg_limiter: RateLimiter,
    last_save: DateTime<Utc>,
    is_paused: bool,

    // Config from environment
    helius_keys: HeliusKeyPool,
    telegram_token: String,
    telegram_chat_id: String,
    sol_price_usd: f64,
    last_price_update: Instant,

    // === Live Trading ===
    positions: HashMap<String, Position>,
    trading_config: TradingConfig,
    wallet: Option<WalletManager>,
    last_sell_check: Instant,

    // === Paper Trading ===
    paper_config: PaperConfig,
    paper_state: PaperTradingState,
    last_paper_report: Instant,

    // === Circuit Breaker ===
    consecutive_losses: u32,
    circuit_breaker_until: Option<Instant>,
    circuit_breaker_losses: u32,
    circuit_breaker_pause_hours: u64,

    // === Smart Filters ===
    peak_hours_only: bool,
    momentum_max_pct: f64,
    dynamic_min_score: f64,
    /// Original MIN_SCORE_TO_BUY from config — never changes, used as floor for dynamic adjust
    base_min_score: f64,
    last_score_adjust: Instant,

    // === Off-peak trading (stricter filters outside peak hours) ===
    off_peak_trading_enabled: bool,
    off_peak_min_score: f64,
    off_peak_min_liquidity: f64,
    off_peak_max_positions: u32,
    off_peak_momentum_max_pct: f64,
    off_peak_saved: Option<OffPeakSavedState>,

    // === Telegram command polling ===
    last_update_id: i64,
    tg_poll_failures: u32,
    last_tg_poll: Instant,

    // === Scheduled reports ===
    last_daily_report_date: String,
    last_weekly_report_date: String,

    // === Scan health monitoring ===
    /// Consecutive times get_token_metadata returned None (Helius down/no key)
    helius_consecutive_failures: u32,
    /// Best token score seen this bot session (for diagnostics)
    best_score_seen: f64,
    /// Total tokens that made it past full_analyze this session
    tokens_qualified_session: u64,
    /// Timestamp of the last paper or live buy — used for no-trade alert
    last_trade_or_buy: Instant,
    /// Last time we sent the no-trade health warning
    last_health_warning: Instant,

    // === Daily Max Loss Protection ===
    /// Cumulative SOL lost today (reset at UTC midnight)
    daily_loss_sol: f64,
    /// UTC date of the current daily_loss_sol period
    daily_loss_date: chrono::NaiveDate,
    /// True when daily loss limit has been reached — buy scanning paused until next UTC day
    daily_limit_paused: bool,
}

impl SolanaBot {
    fn new() -> Self {
        // Load config.env first (highest priority — user config), then fall back to .env
        let _ = dotenvy::from_filename_override("config.env");
        let _ = dotenvy::dotenv_override();

        // Auto-configure Helius RPC for on-chain calls (wallet, balance) if still on public node
        let current_rpc = std::env::var("SOLANA_RPC_URL")
            .unwrap_or_else(|_| "https://api.mainnet-beta.solana.com".to_string());
        if current_rpc.contains("api.mainnet-beta.solana.com") {
            if let Ok(raw_keys) = std::env::var("HELIUS_API_KEY") {
                let first_key = raw_keys.split(',').next().unwrap_or("").trim().to_string();
                if !first_key.is_empty() && !first_key.starts_with("your_") {
                    let helius_rpc = format!("https://mainnet.helius-rpc.com/?api-key={first_key}");
                    // Safety: single-threaded startup, no other threads reading env yet
                    unsafe { std::env::set_var("SOLANA_RPC_URL", &helius_rpc); }
                    println!("[RPC] Auto-configured Helius RPC (fast) from HELIUS_API_KEY");
                }
            }
        }

        // Read required config from environment (panic with clear message if missing)
        let helius_keys = HeliusKeyPool::from_env();
        let telegram_token = std::env::var("TELEGRAM_BOT_TOKEN")
            .expect("TELEGRAM_BOT_TOKEN must be set in .env (see .env.example)");
        let telegram_chat_id = std::env::var("TELEGRAM_CHAT_ID")
            .expect("TELEGRAM_CHAT_ID must be set in .env (see .env.example)");
        let sol_price_usd: f64 = std::env::var("SOL_PRICE_USD")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(170.0);

        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .tcp_keepalive(Duration::from_secs(60))
            .build()
            .expect("Failed to build HTTP client");

        let trading_config = TradingConfig::from_env();

        // Load wallet if trading is enabled
        let wallet = if trading_config.trading_enabled {
            match WalletManager::from_env() {
                Ok(w) => {
                    println!("[TRADING] Wallet loaded successfully: {}", w.public_key);
                    Some(w)
                }
                Err(e) => {
                    eprintln!("[TRADING] ⚠️ Failed to load wallet: {e} — trading disabled");
                    None
                }
            }
        } else {
            println!("[TRADING] Trading disabled (TRADING_ENABLED=false)");
            None
        };

        let trading_mode = if trading_config.trading_enabled && wallet.is_some() {
            "ACTIVE"
        } else {
            "INACTIVE"
        };

        println!("[TRADING] Mode: {} | Max: {:.2} SOL | TP: {:.1}% | SL: {:.1}%",
            trading_mode,
            trading_config.max_position_sol,
            trading_config.take_profit_percent,
            trading_config.stop_loss_percent,
        );

        // Load paper trading config and state
        let paper_config = PaperConfig::from_env();
        let paper_state = if paper_config.enabled {
            println!("[PAPER] Paper trading ACTIVE | Virtual balance: {:.2} SOL", paper_config.virtual_balance_sol);
            load_paper_state(paper_config.virtual_balance_sol)
        } else {
            println!("[PAPER] Paper trading inactive (set PAPER_TRADING_ENABLED=true to enable)");
            PaperTradingState::new(paper_config.virtual_balance_sol)
        };

        // Extract before move into Self {}
        let base_min_score = trading_config.min_score_to_buy;
        let circuit_breaker_losses: u32 = std::env::var("CIRCUIT_BREAKER_LOSSES")
            .ok().and_then(|v| v.parse().ok()).unwrap_or(3);
        let circuit_breaker_pause_hours: u64 = std::env::var("CIRCUIT_BREAKER_PAUSE_HOURS")
            .ok().and_then(|v| v.parse().ok()).unwrap_or(1);
        let peak_hours_only: bool = std::env::var("PEAK_HOURS_ONLY")
            .map(|v| v == "true" || v == "1").unwrap_or(false);
        let momentum_max_pct: f64 = std::env::var("MOMENTUM_MAX_PCT")
            .ok().and_then(|v| v.parse().ok()).unwrap_or(30.0);
        let off_peak_trading_enabled: bool = std::env::var("OFF_PEAK_TRADING_ENABLED")
            .map(|v| v == "true" || v == "1").unwrap_or(false);
        let off_peak_min_score: f64 = std::env::var("OFF_PEAK_MIN_SCORE")
            .ok().and_then(|v| v.parse().ok()).unwrap_or(93.0);
        let off_peak_min_liquidity: f64 = std::env::var("OFF_PEAK_MIN_LIQUIDITY")
            .ok().and_then(|v| v.parse().ok()).unwrap_or(25000.0);
        let off_peak_max_positions: u32 = std::env::var("OFF_PEAK_MAX_POSITIONS")
            .ok().and_then(|v| v.parse().ok()).unwrap_or(1);
        let off_peak_momentum_max_pct: f64 = std::env::var("OFF_PEAK_MOMENTUM_MAX_PCT")
            .ok().and_then(|v| v.parse().ok()).unwrap_or(10.0);

        println!("[FEATURES] Circuit breaker: {} losses → {}h pause | Peak hours: {} | Momentum max: +{:.0}%",
            circuit_breaker_losses, circuit_breaker_pause_hours,
            if peak_hours_only { "ON" } else { "OFF" }, momentum_max_pct);

        let now_date = Utc::now().format("%Y-%m-%d").to_string();
        let now_week = {
            let now = Utc::now();
            format!("{}-W{:02}", now.year(), now.iso_week().week())
        };

        Self {
            client,
            data: BotPersistentData {
                seen_tokens: HashMap::new(),
                tracked_tokens: HashMap::new(),
                smart_wallets: Self::initial_smart_wallets(),
                daily_alert_count: 0,
                last_reset_date: Utc::now().format("%Y-%m-%d").to_string(),
                performance_stats: PerformanceStats::default(),
                positions: HashMap::new(),
                blacklisted_tokens: HashSet::new(),
                daily_loss_sol: 0.0,
                daily_loss_date_str: String::new(),
                daily_limit_paused: false,
            },
            dex_limiter: RateLimiter::new(55, 60),
            tg_limiter: RateLimiter::new(20, 60),
            last_save: Utc::now(),
            is_paused: false,
            helius_keys,
            telegram_token,
            telegram_chat_id,
            sol_price_usd,
            // Set in the past so price is fetched immediately on first loop
            last_price_update: Instant::now()
                .checked_sub(Duration::from_secs(SOL_PRICE_UPDATE_INTERVAL_SECS + 1))
                .unwrap_or_else(Instant::now),
            positions: HashMap::new(),
            trading_config,
            wallet,
            last_sell_check: Instant::now(),
            paper_config,
            paper_state,
            last_paper_report: Instant::now(),
            // Circuit breaker
            consecutive_losses: 0,
            circuit_breaker_until: None,
            circuit_breaker_losses,
            circuit_breaker_pause_hours,
            // Smart filters
            peak_hours_only,
            momentum_max_pct,
            dynamic_min_score: base_min_score,
            base_min_score,
            last_score_adjust: Instant::now(),
            // Off-peak trading
            off_peak_trading_enabled,
            off_peak_min_score,
            off_peak_min_liquidity,
            off_peak_max_positions,
            off_peak_momentum_max_pct,
            off_peak_saved: None,
            // Telegram command polling
            last_update_id: 0,
            tg_poll_failures: 0,
            last_tg_poll: Instant::now() - Duration::from_secs(120),
            // Scheduled reports — initialize to current period so we don't
            // send a spurious report immediately on startup
            last_daily_report_date: now_date,
            last_weekly_report_date: now_week,
            // Scan health monitoring
            helius_consecutive_failures: 0,
            best_score_seen: 0.0,
            tokens_qualified_session: 0,
            // Initialize far in the past so health warning can fire after 6h if needed
            last_trade_or_buy: Instant::now()
                .checked_sub(Duration::from_secs(60))
                .unwrap_or_else(Instant::now),
            last_health_warning: Instant::now()
                .checked_sub(Duration::from_secs(3600))
                .unwrap_or_else(Instant::now),
            // Daily max loss protection
            daily_loss_sol: 0.0,
            daily_loss_date: Utc::now().date_naive(),
            daily_limit_paused: false,
        }
    }

    fn initial_smart_wallets() -> Vec<String> {
        vec![]
    }

    // ============================================================
    // PERSISTENCE
    // ============================================================

    fn save(&self) -> Result<(), Box<dyn std::error::Error>> {
        // Sync positions into data before saving
        let mut data_to_save = serde_json::to_value(&self.data)?;
        let pos_map: HashMap<String, PositionData> = self.positions.iter()
            .map(|(k, v)| (k.clone(), PositionData::from(v)))
            .collect();
        data_to_save["positions"] = serde_json::to_value(&pos_map)?;

        // Inject daily loss protection fields — these live on SolanaBot, not BotPersistentData,
        // so they must be written manually after serializing self.data.
        data_to_save["daily_loss_sol"]      = serde_json::json!(self.daily_loss_sol);
        data_to_save["daily_loss_date_str"] = serde_json::json!(self.daily_loss_date.to_string());
        data_to_save["daily_limit_paused"]  = serde_json::json!(self.daily_limit_paused);

        let json = serde_json::to_string_pretty(&data_to_save)?;
        fs::write("bot_data.json", json)?;
        println!("💾 Data saved — {} seen tokens, {} tracked, {} active positions",
            self.data.seen_tokens.len(),
            self.data.tracked_tokens.len(),
            self.positions.len());
        Ok(())
    }

    fn load(&mut self) {
        match fs::read_to_string("bot_data.json") {
            Ok(content) => {
                match serde_json::from_str::<BotPersistentData>(&content) {
                    Ok(data) => {
                        // Restore saved active positions
                        let saved_positions: HashMap<String, Position> = data.positions.iter()
                            .map(|(k, v)| (k.clone(), Position::from(v.clone())))
                            .collect();

                        // Restore daily loss protection before moving data into self.data
                        let saved_daily_loss_sol    = data.daily_loss_sol;
                        let saved_daily_loss_date   = data.daily_loss_date_str.clone();
                        let saved_daily_limit_paused = data.daily_limit_paused;

                        self.data = data;
                        // Retain only tokens seen in the last 8h.
                        // DexScreener only shows tokens ≤6h old (MAX_TOKEN_AGE_HOURS).
                        // The old 30-day window meant every token in the current
                        // DexScreener feed was already in seen_tokens after a few hours
                        // of uptime — causing 0 new tokens found every scan permanently.
                        // 8h = 6h token age cap + 2h safety buffer to avoid re-analysis.
                        let cutoff = Utc::now() - chrono::Duration::hours(8);
                        let before = self.data.seen_tokens.len();
                        self.data.seen_tokens.retain(|_, ts| {
                            ts.parse::<DateTime<Utc>>()
                                .map(|t| t > cutoff)
                                .unwrap_or(false)
                        });
                        let pruned = before - self.data.seen_tokens.len();
                        if pruned > 0 {
                            println!("[SEEN] Pruned {pruned} stale token entries (>8h old) on load");
                        }
                        self.positions = saved_positions;

                        // Restore daily loss protection — survives restart so the bot
                        // cannot bypass the daily limit by restarting during a bad day.
                        self.daily_loss_sol = saved_daily_loss_sol;
                        self.daily_limit_paused = saved_daily_limit_paused;
                        if !saved_daily_loss_date.is_empty() {
                            if let Ok(d) = saved_daily_loss_date.parse::<chrono::NaiveDate>() {
                                self.daily_loss_date = d;
                            }
                        }
                        if self.daily_limit_paused {
                            println!(
                                "[DAILY LIMIT] ⚠️ Restored from save: {:.5} SOL lost, limit still active — buying paused until 00:00 UTC",
                                self.daily_loss_sol
                            );
                        } else if self.daily_loss_sol > 0.0 {
                            println!(
                                "[DAILY LIMIT] Restored from save: {:.5} SOL lost today",
                                self.daily_loss_sol
                            );
                        }

                        println!("📂 Data loaded: {} seen tokens, {} tracked, {} active positions",
                            self.data.seen_tokens.len(),
                            self.data.tracked_tokens.len(),
                            self.positions.len());
                    }
                    Err(e) => println!("⚠️ Failed to parse saved data file: {e}"),
                }
            }
            Err(_) => println!("ℹ️ Fresh start — no saved data found"),
        }
    }

    fn reset_daily_count_if_needed(&mut self) {
        let today = Utc::now().format("%Y-%m-%d").to_string();
        if self.data.last_reset_date != today {
            self.data.daily_alert_count = 0;
            self.data.last_reset_date = today;
        }
    }

    // ============================================================
    // TELEGRAM
    // ============================================================

    async fn send_message(&mut self, text: &str) -> Result<(), Box<dyn std::error::Error>> {
        self.tg_limiter.wait_if_needed().await;
        let url = format!("https://api.telegram.org/bot{}/sendMessage", self.telegram_token);
        let payload = TelegramMsg {
            chat_id: self.telegram_chat_id.clone(),
            text: text.to_string(),
            parse_mode: "Markdown".to_string(),
            disable_web_page_preview: true,
        };
        let resp = self.client.post(&url).json(&payload).send().await?;
        if !resp.status().is_success() {
            let err = resp.text().await?;
            println!("[TG SEND] ❌ Failed to send message: {err}");
            return Err(format!("Telegram error: {err}").into());
        }
        Ok(())
    }

    // Answer a callback_query so Telegram removes the "loading" spinner on the button
    async fn answer_callback_query(&self, callback_query_id: &str) {
        let url = format!(
            "https://api.telegram.org/bot{}/answerCallbackQuery",
            self.telegram_token
        );
        let payload = serde_json::json!({ "callback_query_id": callback_query_id });
        let _ = self.client.post(&url).json(&payload).send().await;
    }

    async fn send_photo(&mut self, photo_url: &str, caption: &str) {
        self.tg_limiter.wait_if_needed().await;
        let url = format!("https://api.telegram.org/bot{}/sendPhoto", self.telegram_token);
        let payload = TelegramPhoto {
            chat_id: self.telegram_chat_id.clone(),
            photo: photo_url.to_string(),
            caption: caption.to_string(),
            parse_mode: "Markdown".to_string(),
        };
        if let Err(e) = self.client.post(&url).json(&payload).send().await {
            println!("❌ Failed to send photo: {e}");
        }
    }

    async fn send_alert_with_buttons(&mut self, text: &str, dex_url: &str) {
        self.tg_limiter.wait_if_needed().await;
        let url = format!("https://api.telegram.org/bot{}/sendMessage", self.telegram_token);
        let payload = TelegramInlineKeyboard {
            chat_id: self.telegram_chat_id.clone(),
            text: text.to_string(),
            parse_mode: "Markdown".to_string(),
            reply_markup: InlineKeyboardMarkup {
                inline_keyboard: vec![
                    vec![
                        InlineButton {
                            text: "📊 DexScreener".to_string(),
                            url: Some(dex_url.to_string()),
                            callback_data: None,
                        },
                    ],
                    vec![
                        InlineButton {
                            text: "⏸ Pause Bot".to_string(),
                            url: None,
                            callback_data: Some("/pause".to_string()),
                        },
                        InlineButton {
                            text: "📈 Stats".to_string(),
                            url: None,
                            callback_data: Some("/stats".to_string()),
                        },
                    ],
                ],
            },
        };
        if let Err(e) = self.client.post(&url).json(&payload).send().await {
            println!("❌ Failed to send message with buttons: {e}");
        }
    }

    // ============================================================
    // DEXSCREENER API
    // ============================================================

    async fn get_new_solana_tokens(&mut self) -> Result<Vec<DexToken>, Box<dyn std::error::Error>> {
        self.dex_limiter.wait_if_needed().await;
        let url = "https://api.dexscreener.com/token-profiles/latest/v1";
        let resp = self.client.get(url).send().await?;
        if !resp.status().is_success() {
            return Ok(vec![]);
        }
        let tokens: Vec<DexToken> = resp.json().await.unwrap_or_default();
        let now = Utc::now().timestamp();
        Ok(tokens.into_iter().filter(|t| {
            if t.chain_id != "solana" { return false; }
            if let Some(created) = &t.created_at {
                let ts = match created {
                    serde_json::Value::Number(n) => n.as_i64().unwrap_or(0),
                    serde_json::Value::String(s) => s.parse().unwrap_or(0),
                    _ => 0,
                };
                if ts > 0 {
                    let age_hours = (now - ts) / 3600;
                    return age_hours <= MAX_TOKEN_AGE_HOURS;
                }
            }
            true
        }).collect())
    }

    async fn get_pairs(&mut self, address: &str) -> Vec<PairData> {
        self.dex_limiter.wait_if_needed().await;
        let url = format!("https://api.dexscreener.com/latest/dex/tokens/{address}");
        match self.client.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => {
                match resp.json::<PairResponse>().await {
                    Ok(r) => r.pairs.unwrap_or_default()
                        .into_iter()
                        .filter(|p| p.chain_id == "solana")
                        .collect(),
                    Err(_) => vec![],
                }
            }
            _ => vec![],
        }
    }

    // ============================================================
    // HELIUS API
    // ============================================================

    async fn get_token_metadata(&mut self, address: &str) -> Option<HeliusTokenInfo> {
        // Retry once after rotation: if the active key is rate-limited we immediately
        // switch to the next available key and try again — no data dropped on 429.
        let mut retried = false;
        loop {
            self.helius_keys.wait_for_current().await;
            let url = format!(
                "https://api.helius.xyz/v0/token-metadata?api-key={}", self.helius_keys.current()
            );
            let body = serde_json::json!({ "mintAccounts": [address] });
            match self.client.post(&url).json(&body).send().await {
                Ok(resp) => {
                    if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
                        self.helius_keys.rotate_on_rate_limit();
                        if !retried { retried = true; continue; }
                        return None; // all keys exhausted
                    }
                    if !resp.status().is_success() { return None; }
                    let arr: Vec<serde_json::Value> = resp.json().await.ok()?;
                    let item = arr.into_iter().next()?;
                    // NOTE: Do NOT gate on onChainMetadata/tokenStandard here.
                    // Many new SPL memecoins are not yet indexed by Helius as Metaplex
                    // tokens, so requiring that field would cause the function to return
                    // None for virtually every new token — blocking all buys silently.
                    // The mint/freeze authority fields live in account.data.parsed.info
                    // and are available independently of Metaplex metadata status.
                    // Reset consecutive failure counter on success
                    self.helius_consecutive_failures = 0;
                    let mint_auth = item.get("account")
                        .and_then(|a| a.get("data"))
                        .and_then(|d| d.get("parsed"))
                        .and_then(|p| p.get("info"))
                        .and_then(|i| i.get("mintAuthority"))
                        .and_then(|m| m.as_str())
                        .map(|s| s.to_string());
                    let freeze_auth = item.get("account")
                        .and_then(|a| a.get("data"))
                        .and_then(|d| d.get("parsed"))
                        .and_then(|p| p.get("info"))
                        .and_then(|i| i.get("freezeAuthority"))
                        .and_then(|m| m.as_str())
                        .map(|s| s.to_string());
                    return Some(HeliusTokenInfo {
                        mint_authority: mint_auth,
                        freeze_authority: freeze_auth,
                        decimals: None,
                        supply: None,
                    });
                }
                _ => {
                    self.helius_consecutive_failures += 1;
                    return None;
                }
            }
        }
    }

    async fn get_token_holders(&mut self, address: &str) -> Vec<HeliusTokenHolder> {
        let mut retried = false;
        loop {
            self.helius_keys.wait_for_current().await;
            let url = format!(
                "https://api.helius.xyz/v1/token-holders?api-key={}&mint={}&limit=50",
                self.helius_keys.current(), address
            );
            match self.client.get(&url).send().await {
                Ok(resp) => {
                    if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
                        self.helius_keys.rotate_on_rate_limit();
                        if !retried { retried = true; continue; }
                        return vec![];
                    }
                    return if resp.status().is_success() {
                        resp.json::<Vec<HeliusTokenHolder>>().await.unwrap_or_default()
                    } else {
                        vec![]
                    };
                }
                _ => return vec![],
            }
        }
    }

    async fn get_token_transactions(&mut self, address: &str) -> Vec<HeliusTransaction> {
        let mut retried = false;
        loop {
            self.helius_keys.wait_for_current().await;
            let url = format!(
                "https://api.helius.xyz/v0/addresses/{}/transactions?api-key={}&type=SWAP&limit=100",
                address, self.helius_keys.current()
            );
            match self.client.get(&url).send().await {
                Ok(resp) => {
                    if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
                        self.helius_keys.rotate_on_rate_limit();
                        if !retried { retried = true; continue; }
                        return vec![];
                    }
                    return if resp.status().is_success() {
                        resp.json::<Vec<HeliusTransaction>>().await.unwrap_or_default()
                    } else {
                        vec![]
                    };
                }
                _ => return vec![],
            }
        }
    }

    // ============================================================
    // HELIUS KEY TEST - Run at startup and via /helius command
    // ============================================================

    async fn test_helius_keys(&self) -> String {
        let count = self.helius_keys.key_count();
        println!("[HELIUS] Testing {count} key(s)...");
        let mut lines = vec![format!("🔑 **Helius Key Test** ({} key(s))\n", count)];

        // Use SOL mint as a lightweight test target
        let test_mint = "So11111111111111111111111111111111111111112";

        for (i, key) in self.helius_keys.keys.iter().enumerate() {
            let masked = if key.len() > 8 {
                format!("{}***", &key[..8])
            } else {
                "***".to_string()
            };
            let active_marker = if i == self.helius_keys.current_index { " ◀ active" } else { "" };

            let url = format!(
                "https://api.helius.xyz/v0/token-metadata?api-key={key}"
            );
            let body = serde_json::json!({ "mintAccounts": [test_mint] });

            let result = self.client.post(&url).json(&body).send().await;
            let status_line = match result {
                Ok(resp) => {
                    let status = resp.status();
                    if status.is_success() {
                        println!("[HELIUS] Key #{} ({}): ✅ Valid", i + 1, masked);
                        format!("Key #{} `{}`: ✅ Valid{}", i + 1, masked, active_marker)
                    } else if status.as_u16() == 429 {
                        println!("[HELIUS] Key #{} ({}): ⏳ Rate limited (429)", i + 1, masked);
                        format!("Key #{} `{}`: ⏳ Rate limited{}", i + 1, masked, active_marker)
                    } else if status.as_u16() == 401 || status.as_u16() == 403 {
                        println!("[HELIUS] Key #{} ({}): ❌ Invalid/Unauthorized ({})", i + 1, masked, status);
                        format!("Key #{} `{}`: ❌ Invalid ({}){}", i + 1, masked, status, active_marker)
                    } else {
                        println!("[HELIUS] Key #{} ({}): ⚠️  HTTP {}", i + 1, masked, status);
                        format!("Key #{} `{}`: ⚠️ HTTP {}{}", i + 1, masked, status, active_marker)
                    }
                }
                Err(e) => {
                    println!("[HELIUS] Key #{} ({}): ❌ Network error: {}", i + 1, masked, e);
                    format!("Key #{} `{}`: ❌ Error{}", i + 1, masked, active_marker)
                }
            };
            lines.push(status_line);
        }

        if count > 1 {
            lines.push("\n💡 Add more keys as `HELIUS_API_KEY_2`, `HELIUS_API_KEY_3`, etc.\nBot auto-rotates on 429.".to_string());
        } else {
            lines.push(
                "\n💡 Add more keys as `HELIUS_API_KEY_2`, `HELIUS_API_KEY_3` to enable rotation.".to_string()
            );
        }

        lines.join("\n")
    }

    // ============================================================
    // SOL PRICE - Auto-refresh from public APIs
    // ============================================================

    /// Fetch SOL price from multiple sources with fallback.
    /// Order: Jupiter Price API → Binance → CoinGecko → last known price
    async fn fetch_sol_price(&self) -> f64 {
        // --- Source 1: Jupiter Price API v6 (most accurate for Solana) ---
        if let Ok(resp) = self.client
            .get("https://price.jup.ag/v6/price?ids=SOL")
            .send()
            .await
        {
            if let Ok(data) = resp.json::<serde_json::Value>().await {
                if let Some(price) = data["data"]["SOL"]["price"].as_f64() {
                    if price > 0.0 {
                        println!("[SOL PRICE] Jupiter: ${price:.2}");
                        return price;
                    }
                }
            }
        }

        // --- Source 2: Binance public ticker (no auth required) ---
        if let Ok(resp) = self.client
            .get("https://api.binance.com/api/v3/ticker/price?symbol=SOLUSDT")
            .send()
            .await
        {
            if let Ok(data) = resp.json::<serde_json::Value>().await {
                if let Some(price_str) = data["price"].as_str() {
                    if let Ok(price) = price_str.parse::<f64>() {
                        if price > 0.0 {
                            println!("[SOL PRICE] Binance: ${price:.2}");
                            return price;
                        }
                    }
                }
            }
        }

        // --- Source 3: CoinGecko free API ---
        if let Ok(resp) = self.client
            .get("https://api.coingecko.com/api/v3/simple/price?ids=solana&vs_currencies=usd")
            .send()
            .await
        {
            if let Ok(data) = resp.json::<serde_json::Value>().await {
                if let Some(price) = data["solana"]["usd"].as_f64() {
                    if price > 0.0 {
                        println!("[SOL PRICE] CoinGecko: ${price:.2}");
                        return price;
                    }
                }
            }
        }

        // --- Fallback: use last cached price ---
        println!("[SOL PRICE] All sources failed, using cached price: ${:.2}", self.sol_price_usd);
        self.sol_price_usd
    }

    // ============================================================
    // ANALYSIS FUNCTIONS
    // ============================================================

    async fn analyze_holders(&mut self, address: &str, pairs: &[PairData]) -> HolderAnalysis {
        let holders = self.get_token_holders(address).await;
        let txns = self.get_token_transactions(address).await;

        let total_holders = holders.len() as u32;
        let mut flags = vec![];
        let mut signals = vec![];

        let amounts: Vec<f64> = holders.iter()
            .filter_map(|h| h.amount.parse::<f64>().ok())
            .filter(|&a| a > 0.0)
            .collect();

        let total_supply: f64 = amounts.iter().sum();

        let top10_sum: f64 = if amounts.len() >= 10 {
            let mut sorted = amounts.clone();
            sorted.sort_by(|a, b| b.total_cmp(a));
            sorted[..10].iter().sum()
        } else {
            amounts.iter().sum()
        };

        let top10_pct = if total_supply > 0.0 { top10_sum / total_supply * 100.0 } else { 0.0 };

        // Gini coefficient
        let gini = if amounts.len() > 1 {
            let mut sorted = amounts.clone();
            sorted.sort_by(|a, b| a.total_cmp(b));
            let n = sorted.len() as f64;
            let sum: f64 = sorted.iter().enumerate()
                .map(|(i, &v)| (2.0 * (i as f64 + 1.0) - n - 1.0) * v)
                .sum();
            sum / (n * total_supply)
        } else { 0.0 };

        // Detect snipers (bought in first block)
        let pair_created = pairs.first().and_then(|p| p.pair_created_at).unwrap_or(0);
        let sniper_count = txns.iter()
            .filter(|t| pair_created > 0 && (t.timestamp - pair_created).abs() < 30)
            .count() as u32;
        let sniper_pct = if total_holders > 0 {
            sniper_count as f64 / total_holders as f64 * 100.0
        } else { 0.0 };

        // Detect bundled wallets (many wallets same timing)
        let bundled = sniper_count >= 5 && sniper_pct > 20.0;

        // Developer sold % — placeholder (not derived from real on-chain data yet;
        // not used in scoring, kept for future implementation)
        let dev_sold = if !txns.is_empty() { 15.0 } else { 0.0 };

        // Flags
        if top10_pct > 50.0 {
            flags.push(format!("🔴 Top 10 holders: {top10_pct:.1}% — high concentration"));
        } else {
            signals.push(format!("✅ Healthy holder distribution: Top 10 = {top10_pct:.1}%"));
        }
        if sniper_count > 10 {
            flags.push(format!("🔴 {sniper_count} snipers detected"));
        }
        if bundled {
            flags.push("🔴 Bundled wallets detected".to_string());
        }
        if total_holders > 500 {
            signals.push(format!("✅ {total_holders} active holders"));
        }

        // Score (max 25)
        let mut score = 0.0f64;
        if top10_pct < 30.0 { score += 10.0; }
        else if top10_pct < 50.0 { score += 5.0; }
        if sniper_count < 5 { score += 5.0; }
        if !bundled { score += 5.0; }
        if total_holders > 200 { score += 5.0; }
        score = score.clamp(0.0, 25.0);

        HolderAnalysis {
            total_holders,
            sniper_count,
            sniper_percentage: sniper_pct,
            top10_concentration: top10_pct,
            gini_coefficient: gini.abs(),
            bundled_wallets_detected: bundled,
            developer_sold_percentage: dev_sold,
            score,
            flags,
            signals,
        }
    }

    fn analyze_liquidity(&self, pairs: &[PairData]) -> LiquidityAnalysis {
        let total_usd: f64 = pairs.iter()
            .filter_map(|p| p.liquidity.as_ref())
            .filter_map(|l| l.usd)
            .sum();

        // Heuristic proxy — no on-chain LP burn/lock data available from DexScreener.
        // High liquidity is used as a positive signal; score credit is modest (+5 max).
        let lp_burned = total_usd > 50_000.0;
        let lp_locked = total_usd > 20_000.0;
        let independent = pairs.len() as u32;

        let price_impact_5k  = if total_usd > 0.0 { 5_000.0 / total_usd * 100.0 } else { 100.0 };
        let price_impact_10k = if total_usd > 0.0 { 10_000.0 / total_usd * 100.0 } else { 100.0 };
        let price_impact_25k = if total_usd > 0.0 { 25_000.0 / total_usd * 100.0 } else { 100.0 };

        let mut flags = vec![];
        let mut signals = vec![];

        if total_usd < 10_000.0 {
            flags.push(format!("🔴 Low liquidity: ${total_usd:.0}"));
        } else if total_usd > 100_000.0 {
            signals.push(format!("✅ Strong liquidity: ${total_usd:.0}"));
        } else {
            signals.push(format!("✅ Adequate liquidity: ${total_usd:.0}"));
        }
        if lp_burned { signals.push("✅ LP Burned/Locked detected".to_string()); }

        let mut score = 0.0f64;
        if total_usd > 100_000.0 { score += 10.0; }
        else if total_usd > 50_000.0 { score += 7.0; }
        else if total_usd > 20_000.0 { score += 5.0; }
        else if total_usd > 10_000.0 { score += 3.0; }
        if lp_burned { score += 5.0; }
        if independent > 2 { score += 5.0; }
        score = score.clamp(0.0, 20.0);

        LiquidityAnalysis {
            total_usd,
            lp_burned,
            lp_locked,
            lock_duration_months: if lp_locked { Some(12.0) } else { None },
            independent_providers: independent,
            price_impact_5k,
            price_impact_10k,
            price_impact_25k,
            score,
            flags,
            signals,
        }
    }

    async fn analyze_whales(&mut self, address: &str) -> WhaleAnalysis {
        let txns = self.get_token_transactions(address).await;
        let mut signals = vec![];
        let mut smart_entered = 0u32;
        let mut buy_amounts: Vec<f64> = vec![];
        let mut sell_amounts: Vec<f64> = vec![];

        for txn in &txns {
            if let Some(transfers) = &txn.token_transfers {
                if self.data.smart_wallets.iter().any(|w| {
                    txn.fee_payer.as_deref().map(|fp| fp == w).unwrap_or(false)
                }) {
                    smart_entered += 1;
                }

                let total_in: f64 = transfers.iter()
                    .filter(|tf| tf.to_user_account.as_deref() == txn.fee_payer.as_deref())
                    .map(|tf| tf.token_amount)
                    .sum();
                let total_out: f64 = transfers.iter()
                    .filter(|tf| tf.from_user_account.as_deref() == txn.fee_payer.as_deref())
                    .map(|tf| tf.token_amount)
                    .sum();
                if total_in > total_out { buy_amounts.push(total_in); }
                else { sell_amounts.push(total_out); }
            }
        }

        // NOTE: buy_amounts holds raw token units (not USD) — named accordingly below
        let largest_buy_tokens = buy_amounts.iter().cloned().fold(0.0_f64, f64::max);
        let accumulation = buy_amounts.len() > sell_amounts.len() * 2;
        let has_distribution = sell_amounts.len() > buy_amounts.len();

        if smart_entered >= 3 {
            signals.push(format!("🐋 {smart_entered} smart wallets entered!"));
        }
        if accumulation && !has_distribution {
            signals.push("📈 Clear accumulation pattern — no distribution".to_string());
        }

        let mut score = 0.0f64;
        if smart_entered >= 3 { score += 12.0; }
        else if smart_entered >= 1 { score += 6.0; }
        if accumulation && !has_distribution { score += 5.0; }
        if has_distribution { score -= 5.0; }
        score = score.clamp(0.0, 20.0);

        WhaleAnalysis {
            smart_wallets_entered: smart_entered,
            accumulation_pattern: accumulation,
            distribution_signs: has_distribution,
            cold_storage_transfers: 0,
            largest_single_buy_usd: largest_buy_tokens,
            score,
            signals,
        }
    }

    fn analyze_technicals(&self, pairs: &[PairData]) -> TechnicalAnalysis {
        let best = pairs.first();
        let m5  = best.and_then(|p| p.price_change.as_ref()).and_then(|pc| pc.m5).unwrap_or(0.0);
        let h1  = best.and_then(|p| p.price_change.as_ref()).and_then(|pc| pc.h1).unwrap_or(0.0);
        let h6  = best.and_then(|p| p.price_change.as_ref()).and_then(|pc| pc.h6).unwrap_or(0.0);
        let h24 = best.and_then(|p| p.price_change.as_ref()).and_then(|pc| pc.h24).unwrap_or(0.0);
        let vol24 = best.and_then(|p| p.volume.as_ref()).and_then(|v| v.h24).unwrap_or(0.0);

        let (buys, sells) = best
            .and_then(|p| p.txns.as_ref())
            .and_then(|t| t.h1.as_ref())
            .map(|t| (t.buys.unwrap_or(0), t.sells.unwrap_or(0)))
            .unwrap_or((0, 0));
        let total_txns = buys + sells;
        let buy_ratio = if total_txns > 0 { buys as f64 / total_txns as f64 } else { 0.5 };

        let pattern = if m5 > 20.0 && h1 > 50.0 {
            Some("🚀 Bull Flag — strong momentum".to_string())
        } else if m5 > 5.0 && h1 > 20.0 && h6 > 50.0 {
            Some("📈 Ascending Triangle".to_string())
        } else if h1 < -10.0 && h6 > 20.0 {
            Some("🔄 Wyckoff — possible accumulation".to_string())
        } else {
            None
        };

        let mut signals = vec![];
        if m5 > 20.0 { signals.push(format!("⚡ +{m5:.1}% in 5 minutes!")); }
        if h1 > 50.0 { signals.push(format!("🚀 +{h1:.1}% in 1 hour!")); }
        if buy_ratio > 0.7 { signals.push(format!("📈 Buy pressure: {:.0}% buys", buy_ratio * 100.0)); }
        if total_txns > 100 { signals.push(format!("🔥 {total_txns} transactions/hour")); }
        if let Some(p) = &pattern { signals.push(p.clone()); }
        if vol24 > 500_000.0 { signals.push(format!("📊 24h volume: ${vol24:.0}")); }

        let mut score = 0.0f64;
        if m5 > 20.0 { score += 3.0; } else if m5 > 10.0 { score += 1.5; }
        if h1 > 50.0 { score += 3.0; } else if h1 > 20.0 { score += 1.5; }
        if buy_ratio > 0.7 { score += 2.0; }
        if total_txns > 100 { score += 2.0; }
        score = score.clamp(0.0, 10.0);

        TechnicalAnalysis {
            momentum_5m: m5,
            momentum_1h: h1,
            momentum_6h: h6,
            momentum_24h: h24,
            volume_24h: vol24,
            buy_pressure_ratio: buy_ratio,
            pattern_detected: pattern,
            support_level: None,
            resistance_level: None,
            score,
            signals,
        }
    }

    async fn analyze_contract_security(&mut self, address: &str) -> ContractSecurity {
        let meta = self.get_token_metadata(address).await;
        let meta_available = meta.is_some();
        let mut flags = vec![];
        let mut signals = vec![];

        // When Helius metadata is unavailable (API error, token not yet indexed),
        // we treat authorities as "unknown — assume revoked" rather than "confirmed
        // not revoked". This prevents silently blocking every new token that Helius
        // hasn't indexed as a Metaplex token yet.
        // Security score credit is only awarded when the data is confirmed by Helius.
        let mint_revoked = meta.as_ref()
            .map(|m| m.mint_authority.is_none())
            .unwrap_or(true); // optimistic when API data unavailable
        let freeze_revoked = meta.as_ref()
            .map(|m| m.freeze_authority.is_none())
            .unwrap_or(true); // optimistic when API data unavailable

        if !meta_available {
            flags.push("⚠️ Security data unavailable — Helius not indexed yet".to_string());
        } else if !mint_revoked {
            flags.push("🔴 Mint authority NOT revoked — risk of new token minting".to_string());
        } else {
            signals.push("✅ Mint authority revoked".to_string());
        }

        if meta_available && !freeze_revoked {
            flags.push("⚠️ Freeze authority active — wallets can be frozen".to_string());
        } else if meta_available {
            signals.push("✅ Freeze authority revoked".to_string());
        }

        // Score credit only when Helius CONFIRMED the authority is revoked.
        // Unknown (not indexed) = 0 pts — keeps borderline tokens below buy threshold.
        let mut score = 0.0f64;
        if meta_available && mint_revoked   { score += 6.0; }
        if meta_available && freeze_revoked { score += 4.0; }

        ContractSecurity {
            mint_authority_revoked: mint_revoked,
            freeze_authority_revoked: freeze_revoked,
            metadata_available: meta_available,
            transfer_fee_percent: 0.0,
            honeypot_risk: false,
            score,
            flags,
            signals,
        }
    }

    fn analyze_social(&self, token: &DexToken) -> SocialAnalysis {
        let desc = token.description.as_deref().unwrap_or("");
        let has_twitter = desc.contains("twitter") || desc.contains("x.com");
        let has_telegram = desc.contains("t.me") || desc.contains("telegram");
        let social_hype = if has_twitter && has_telegram { 70.0 }
            else if has_twitter || has_telegram { 40.0 }
            else { 10.0 };

        let mut signals = vec![];
        if has_twitter { signals.push("🐦 Twitter present".to_string()); }
        if has_telegram { signals.push("📱 Telegram present".to_string()); }

        let score = if has_twitter && has_telegram { 10.0 }
            else if has_twitter || has_telegram { 6.0 }
            else { 2.0 };

        SocialAnalysis { has_twitter, has_telegram, social_hype_score: social_hype, score, signals }
    }

    fn analyze_lifecycle(&self, pairs: &[PairData]) -> TokenLifecycle {
        let now = Utc::now().timestamp();
        let created = pairs.first()
            .and_then(|p| p.pair_created_at)
            .unwrap_or(now);
        let age_minutes = (now - created) / 60;

        let phase = match age_minutes {
            0..=30    => LifecyclePhase::Launch,
            31..=180  => LifecyclePhase::FirstDip,
            181..=720 => LifecyclePhase::Accumulation,
            721..=2880 => LifecyclePhase::Breakout,
            _         => LifecyclePhase::Mature,
        };

        let timing_score = match &phase {
            LifecyclePhase::Accumulation => 10.0,
            LifecyclePhase::FirstDip     => 7.0,
            LifecyclePhase::Breakout     => 5.0,
            LifecyclePhase::Launch       => 4.0,
            LifecyclePhase::Mature       => 1.0,
        };

        let rr_ratio = match &phase {
            LifecyclePhase::Accumulation => 15.0,
            LifecyclePhase::FirstDip     => 10.0,
            LifecyclePhase::Breakout     => 5.0,
            _                            => 3.0,
        };

        TokenLifecycle { age_minutes, phase, entry_timing_score: timing_score, risk_reward_ratio: rr_ratio }
    }

    // ============================================================
    // MASTER ANALYSIS
    // ============================================================

    async fn full_analyze(&mut self, token: &DexToken) -> Option<FullTokenAnalysis> {
        let addr = &token.token_address;

        // Blacklist check: skip tokens manually flagged as bad actors
        if self.data.blacklisted_tokens.contains(addr.as_str()) {
            println!("  ⏭ Skip — token blacklisted");
            return None;
        }

        let pairs = self.get_pairs(addr).await;
        if pairs.is_empty() { return None; }

        // Momentum filter: skip tokens that already pumped too much in the past 1h
        // (we missed the entry window — chasing is high-risk)
        let h1_change = pairs.first()
            .and_then(|p| p.price_change.as_ref())
            .and_then(|pc| pc.h1)
            .unwrap_or(0.0);
        if h1_change > self.momentum_max_pct {
            println!("  ⏭ Skip — already pumped +{:.1}% in 1h (threshold: +{:.0}%)", h1_change, self.momentum_max_pct);
            return None;
        }

        let holder = self.analyze_holders(addr, &pairs).await;
        if holder.bundled_wallets_detected {
            println!("  ⏭ Skip — bundled wallets detected");
            return None;
        }

        let liquidity = self.analyze_liquidity(&pairs);
        // Use configured threshold — respects MIN_LIQUIDITY_USD and off-peak override
        if liquidity.total_usd < self.trading_config.min_liquidity_usd {
            println!("  ⏭ Skip — liquidity too low");
            return None;
        }

        let whale     = self.analyze_whales(addr).await;
        let technical = self.analyze_technicals(&pairs);
        let security  = self.analyze_contract_security(addr).await;

        // Hard-block only when Helius CONFIRMED mint authority exists.
        // If metadata was unavailable (not indexed), mint_authority_revoked is
        // optimistically true — the token goes through to scoring but earns 0
        // security points (max score 90 instead of 100), keeping the bar high.
        if security.metadata_available && !security.mint_authority_revoked {
            println!("  ⏭ Skip — mint authority confirmed NOT revoked (Helius verified)");
            return None;
        }

        let social    = self.analyze_social(token);
        let lifecycle = self.analyze_lifecycle(&pairs);

        let total = holder.score + liquidity.score + whale.score
            + technical.score + security.score + social.score;
        let total = (total + lifecycle.entry_timing_score * 0.5).min(100.0);

        // New tokens (<6h) get a 5-point lower bar to compensate for thinner data.
        // Both thresholds track MIN_SCORE_TO_BUY from config — change config.env to adjust.
        let min_score = if lifecycle.age_minutes < 360 {
            (self.trading_config.min_score_to_buy - 5.0).max(0.0)
        } else {
            self.trading_config.min_score_to_buy
        };

        if total < min_score {
            println!("  ⚪ Score too low: {total:.1} < {min_score:.1}");
            return None;
        }

        let mut all_flags = vec![];
        all_flags.extend(holder.flags.clone());
        all_flags.extend(liquidity.flags.clone());
        all_flags.extend(security.flags.clone());

        let mut top_signals = vec![];
        top_signals.extend(holder.signals.clone());
        top_signals.extend(liquidity.signals.clone());
        top_signals.extend(whale.signals.clone());
        top_signals.extend(technical.signals.clone());
        top_signals.extend(security.signals.clone());
        top_signals.extend(social.signals.clone());
        top_signals.truncate(8);

        let confidence = match total as u32 {
            90.. => ConfidenceLevel::VeryHigh,
            85.. => ConfidenceLevel::High,
            80.. => ConfidenceLevel::Medium,
            _    => ConfidenceLevel::Low,
        };

        let alert_level = match total as u32 {
            90.. => AlertLevel::Legendary,
            85.. => AlertLevel::Golden,
            80.. => AlertLevel::Excellent,
            _    => AlertLevel::Normal,
        };

        let potential = match total as u32 {
            90.. => "50x-100x 🌟",
            85.. => "20x-50x 💎",
            80.. => "10x-20x 🔥",
            _    => "5x-10x ⭐",
        };

        let price = pairs.first()
            .and_then(|p| p.price_usd.as_ref())
            .and_then(|p| p.parse::<f64>().ok());

        let market_cap = pairs.iter()
            .filter_map(|p| p.market_cap)
            .reduce(f64::max);

        let dex_urls: Vec<String> = pairs.iter().map(|p| p.url.clone()).collect();

        Some(FullTokenAnalysis {
            token_address: addr.clone(),
            symbol: token.symbol.clone().unwrap_or("UNKNOWN".to_string()),
            name: token.name.clone().unwrap_or("Unknown".to_string()),
            image_url: token.image_url.clone(),
            price_usd: price,
            market_cap,
            dex_urls,
            total_score: total,
            confidence_level: confidence,
            potential_multiplier: potential.to_string(),
            holder_analysis: holder,
            liquidity_analysis: liquidity,
            whale_analysis: whale,
            technical_analysis: technical,
            contract_security: security,
            social_analysis: social,
            lifecycle,
            alert_level,
            all_red_flags: all_flags,
            top_signals,
        })
    }

    // ============================================================
    // MESSAGE FORMATTING
    // ============================================================

    fn format_alert(&self, a: &FullTokenAnalysis) -> String {
        let mut m = String::new();
        m.push_str(&format!("{}\n", a.alert_level.to_header()));
        m.push_str("═══════════════════════════════\n\n");
        m.push_str(&format!("**{}** `({})`\n", a.name, a.symbol));
        m.push_str(&format!("📍 `{}`\n\n", a.token_address));

        m.push_str("📊 **Analysis Dashboard:**\n");
        m.push_str(&format!("⭐ Total: **{:.1}/100**\n", a.total_score));
        m.push_str(&format!("👥 Holders: **{:.1}/25**\n", a.holder_analysis.score));
        m.push_str(&format!("🌊 Liquidity: **{:.1}/20**\n", a.liquidity_analysis.score));
        m.push_str(&format!("🐋 Whales: **{:.1}/20**\n", a.whale_analysis.score));
        m.push_str(&format!("📈 Technical: **{:.1}/10**\n", a.technical_analysis.score));
        m.push_str(&format!("🛡️ Security: **{:.1}/10**\n", a.contract_security.score));
        m.push_str(&format!("🌐 Social: **{:.1}/10**\n\n", a.social_analysis.score));

        m.push_str("💰 **Market Data:**\n");
        if let Some(price) = a.price_usd {
            m.push_str(&format!("💵 Price: **${price:.8}**\n"));
        }
        if let Some(mc) = a.market_cap {
            m.push_str(&format!("🏛️ Market Cap: **{}**\n", format_usd(mc)));
        }
        m.push_str(&format!("🌊 Liquidity: **{}**\n", format_usd(a.liquidity_analysis.total_usd)));
        m.push_str(&format!("📊 Volume 24h: **{}**\n\n", format_usd(a.technical_analysis.volume_24h)));

        m.push_str("⏰ **Entry Timing:**\n");
        m.push_str(&format!("{}\n", a.lifecycle.phase.to_label()));
        m.push_str(&format!("⏱️ Token age: **{} minutes**\n", a.lifecycle.age_minutes));
        m.push_str(&format!("📐 R/R Ratio: **1:{:.0}**\n\n", a.lifecycle.risk_reward_ratio));

        m.push_str(&format!("🎯 **Potential Multiplier: {}**\n\n", a.potential_multiplier));

        m.push_str("✅ **Top Signals:**\n");
        for signal in &a.top_signals {
            m.push_str(&format!("▫️ {signal}\n"));
        }

        if !a.all_red_flags.is_empty() {
            m.push_str("\n⚠️ **Warnings:**\n");
            for flag in a.all_red_flags.iter().take(3) {
                m.push_str(&format!("▪️ {flag}\n"));
            }
        }

        // Add auto buy status if trading is enabled
        if self.trading_config.trading_enabled {
            m.push_str(&format!("\n🤖 **Auto Buy:** {}",
                if a.total_score >= self.trading_config.min_score_to_buy {
                    {
                        let min_s = self.trading_config.min_score_to_buy;
                        let mult = ((a.total_score - min_s) / (100.0_f64 - min_s)).clamp(0.0, 1.0);
                        format!("Will buy {:.3} SOL", mult * self.trading_config.max_position_sol)
                    }
                } else {
                    "Does not meet criteria".to_string()
                }
            ));
        }

        m.push_str("\n═══════════════════════════════\n");
        m.push_str("⚠️ **This is automated analysis, not financial advice**\n");
        m.push_str("🔍 Do your own research before investing\n");
        m
    }

    // ============================================================
    // AUTO BUY - Execute purchase after analysis
    // ============================================================

    async fn check_and_buy(&mut self, analysis: &FullTokenAnalysis) {
        if !self.trading_config.trading_enabled || self.wallet.is_none() {
            return;
        }

        let signal = BuySignal {
            token_address: analysis.token_address.clone(),
            symbol: analysis.symbol.clone(),
            name: analysis.name.clone(),
            total_score: analysis.total_score,
            liquidity_usd: analysis.liquidity_analysis.total_usd,
            mint_authority_revoked: analysis.contract_security.mint_authority_revoked,
            current_price_usd: analysis.price_usd.unwrap_or(0.0),
            market_cap: analysis.market_cap,
        };

        let decision = evaluate_buy_signal(&signal, &self.trading_config, &self.positions, self.sol_price_usd);
        log_buy_decision(&signal, &decision);

        if let BuyDecision::Buy { amount_sol, reason, .. } = decision {
            println!("[AUTO BUY] Executing buy {} — {:.4} SOL | {}", signal.symbol, amount_sol, reason);

            // Guard: wallet must be configured (WALLET_PRIVATE_KEY set)
            let Some(ref wallet) = self.wallet else {
                println!("[AUTO BUY] No wallet configured — set WALLET_PRIVATE_KEY to enable live trading");
                return;
            };

            // Check wallet balance
            let balance = match wallet.get_sol_balance().await {
                Ok(b) => b,
                Err(e) => {
                    println!("[AUTO BUY] Failed to check balance: {e}");
                    return;
                }
            };

            if balance < amount_sol + 0.01 {
                println!("[AUTO BUY] Insufficient balance: {:.4} SOL (need {:.4} SOL)", balance, amount_sol + 0.01);
                return;
            }

            // Execute buy
            match wallet
                .buy_token(&signal.token_address, amount_sol, self.trading_config.default_slippage)
                .await
            {
                Ok(signature) => {
                    println!("[AUTO BUY] ✅ SUCCESS! TX: {signature}");

                    // Compute effective entry price — IDENTICAL formula used by paper trading.
                    // Jupiter fills at quoted × (1 + slippage% + AMM_impact%).
                    // Recording this as entry price makes TP/SL math match paper exactly.
                    let slippage = self.trading_config.default_slippage;
                    let price_impact_pct = PaperTradingState::calc_price_impact_pct(
                        amount_sol, signal.liquidity_usd, self.sol_price_usd,
                    );
                    let effective_entry_price = signal.current_price_usd
                        * (1.0 + (slippage + price_impact_pct) / 100.0);

                    println!(
                        "[AUTO BUY] Quoted: ${:.8} → Effective: ${:.8} | Slip: {:.2}% | Impact: {:.2}%",
                        signal.current_price_usd, effective_entry_price, slippage, price_impact_pct
                    );

                    // Token amount based on effective fill price (matches paper trading)
                    let sol_price_usd = self.sol_price_usd;
                    let token_amount = if effective_entry_price > 0.0 {
                        (amount_sol * sol_price_usd) / effective_entry_price
                    } else { 0.0 };

                    // Create new position with effective entry price (consistent with paper)
                    let position = Position::new(
                        signal.token_address.clone(),
                        signal.symbol.clone(),
                        signal.name.clone(),
                        effective_entry_price,
                        amount_sol,
                        token_amount,
                        signal.total_score,
                        signal.liquidity_usd,
                    );
                    self.positions.insert(signal.token_address.clone(), position);
                    self.data.performance_stats.total_buys += 1;

                    // Send Telegram notification
                    let msg = format_buy_notification(
                        &signal.token_address,
                        &signal.symbol,
                        &signal.name,
                        amount_sol,
                        signal.current_price_usd,
                        effective_entry_price,
                        slippage,
                        price_impact_pct,
                        signal.total_score,
                        &signature,
                    );
                    let _ = self.send_message(&msg).await;
                }
                Err(e) => {
                    println!("[AUTO BUY] ❌ FAILED: {e}");
                    let err_msg = format!(
                        "❌ **AUTO BUY FAILED**\nToken: {} ({})\nError: {}\nCheck logs for details.",
                        signal.name, signal.symbol, e
                    );
                    let _ = self.send_message(&err_msg).await;
                }
            }
        }
    }

    // ============================================================
    // AUTO SELL - Check and execute position exits
    // ============================================================

    async fn check_and_sell_positions(&mut self) {
        if !self.trading_config.trading_enabled || self.wallet.is_none() {
            return;
        }
        if self.positions.is_empty() {
            return;
        }

        println!("[AUTO SELL] Checking {} active positions...", self.positions.len());

        // Fetch current prices for all positions
        let mut prices: HashMap<String, f64> = HashMap::new();
        let addresses: Vec<String> = self.positions.keys().cloned().collect();

        for addr in &addresses {
            self.dex_limiter.wait_if_needed().await;
            let url = format!("https://api.dexscreener.com/latest/dex/tokens/{addr}");
            if let Ok(resp) = self.client.get(&url).send().await {
                if resp.status().is_success() {
                    if let Ok(pr) = resp.json::<PairResponse>().await {
                        let price = pr.pairs.unwrap_or_default()
                            .into_iter()
                            .find(|p| p.chain_id == "solana")
                            .and_then(|p| p.price_usd)
                            .and_then(|p| p.parse::<f64>().ok());
                        if let Some(p) = price {
                            prices.insert(addr.clone(), p);
                        }
                    }
                }
            }
        }

        // Evaluate all positions
        let decisions = evaluate_all_positions(
            &mut self.positions,
            &prices,
            &self.trading_config,
        );

        // Execute sells for triggered positions
        for (addr, decision) in decisions {
            if let SellDecision::Sell { percentage, trigger } = decision {
                let (symbol, _name, buy_price, amount_sol, token_amount) = {
                    let pos = match self.positions.get(&addr) {
                        Some(p) => p,
                        None => continue,
                    };
                    (pos.symbol.clone(), pos.name.clone(), pos.buy_price_usd, pos.amount_in_sol, pos.token_amount)
                };

                let current_price = prices.get(&addr).copied().unwrap_or(0.0);

                println!(
                    "[AUTO SELL] Executing sell {} — {:.1}% | {}",
                    symbol, percentage, trigger.description()
                );

                let Some(ref wallet) = self.wallet else {
                    println!("[AUTO SELL] No wallet configured — skipping live sell");
                    continue;
                };
                match wallet
                    .sell_token(&addr, percentage, self.trading_config.default_slippage)
                    .await
                {
                    Ok(signature) => {
                        println!("[AUTO SELL] ✅ SUCCESS! TX: {signature}");

                        // Apply sell-side slippage + AMM price impact + network fee to P&L.
                        // This mirrors paper trading's execute_sell() exactly so that live
                        // P&L accounting is consistent with simulation results.
                        let sold_sol = amount_sol * percentage / 100.0;
                        let slippage = self.trading_config.default_slippage;
                        let liquidity_usd = if let Some(p) = self.positions.get(&addr) {
                            p.liquidity_at_entry
                        } else { 0.0 };
                        // Price impact: use actual token value at current price, NOT original
                        // SOL capital. At high profits (e.g. 5x), token value is 5× larger
                        // which produces a larger and more accurate impact estimate — identical
                        // to paper trading's execute_sell() which uses sold_tokens × current_price.
                        let tokens_to_sell = token_amount * percentage / 100.0;
                        let sell_value_usd = tokens_to_sell * current_price;
                        let sell_impact_pct = if liquidity_usd > 0.0 {
                            (sell_value_usd / (liquidity_usd + sell_value_usd) * 100.0).min(30.0)
                        } else { 0.0 };
                        let effective_sell_price = current_price
                            * (1.0 - (slippage + sell_impact_pct) / 100.0);
                        let profit_pct = if buy_price > 0.0 {
                            (effective_sell_price - buy_price) / buy_price * 100.0
                        } else { 0.0 };
                        // Deduct sell network fee from proceeds (paper does the same)
                        let profit_sol =
                            sold_sol * profit_pct / 100.0 - strategy::NETWORK_FEE_SOL;

                        // Update stats
                        self.data.performance_stats.total_sells += 1;
                        if profit_sol >= 0.0 {
                            self.data.performance_stats.total_profit_sol += profit_sol;
                        } else {
                            self.data.performance_stats.total_loss_sol += profit_sol.abs();
                        }

                        // Circuit breaker + daily limit: only track on full position close
                        if percentage >= 100.0 {
                            self.handle_trade_result(profit_sol).await;
                        }

                        // Send notification
                        if let Some(pos) = self.positions.get(&addr) {
                            let msg = format_sell_notification(pos, current_price, &trigger, &signature);
                            let _ = self.send_message(&msg).await;
                        }

                        // Update position after sell
                        if percentage >= 100.0 {
                            self.positions.remove(&addr);
                            println!("[AUTO SELL] Position {symbol} removed from active list");
                        } else {
                            // Partial sell — reduce amount and mark TP stage
                            let remaining = 1.0 - percentage / 100.0;
                            if let Some(pos) = self.positions.get_mut(&addr) {
                                pos.amount_in_sol *= remaining;
                                pos.token_amount  *= remaining;
                                match &trigger {
                                    crate::sell_strategy::SellTrigger::PartialTakeProfit { stage: 1, .. } => {
                                        pos.tp1_fired = true;
                                        println!("[AUTO SELL] TP1 fired — {:.0}% of position remains active", remaining * 100.0);
                                    }
                                    crate::sell_strategy::SellTrigger::PartialTakeProfit { stage: 2, .. } => {
                                        pos.tp2_fired = true;
                                        println!("[AUTO SELL] TP2 fired — {:.0}% of position remains active", remaining * 100.0);
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                    Err(e) => {
                        println!("[AUTO SELL] ❌ FAILED selling {symbol}: {e}");
                        let err_msg = format!(
                            "❌ **AUTO SELL FAILED**\n{}\nTrigger: {}\nError: {}",
                            symbol, trigger.description(), e
                        );
                        let _ = self.send_message(&err_msg).await;
                    }
                }

                // Delay between sells
                sleep(Duration::from_secs(2)).await;
            }
        }
    }

    // ============================================================
    // PAPER TRADING - Simulated Buy
    // ============================================================

    async fn check_and_paper_buy(&mut self, analysis: &FullTokenAnalysis) {
        if !self.paper_config.enabled {
            return;
        }

        let signal = BuySignal {
            token_address: analysis.token_address.clone(),
            symbol: analysis.symbol.clone(),
            name: analysis.name.clone(),
            total_score: analysis.total_score,
            liquidity_usd: analysis.liquidity_analysis.total_usd,
            mint_authority_revoked: analysis.contract_security.mint_authority_revoked,
            current_price_usd: analysis.price_usd.unwrap_or(0.0),
            market_cap: analysis.market_cap,
        };

        // Use same logic as live trading, but evaluate against paper positions
        let paper_positions_snapshot: HashMap<String, Position> = self.paper_state.positions.iter()
            .map(|(k, v)| {
                (k.clone(), Position::new(
                    v.token_address.clone(), v.symbol.clone(), v.name.clone(),
                    v.buy_price_usd, v.amount_sol, v.token_amount, v.score_at_entry,
                    v.liquidity_at_entry,
                ))
            })
            .collect();

        let slippage = self.paper_config.default_slippage;

        // Build paper buy config from live trading_config — override only
        // paper-specific position size and filter thresholds.
        // All TP/SL/trailing/time-exit params come from trading_config so
        // paper and mainnet use exactly the same evaluation criteria.
        let paper_buy_config = TradingConfig {
            trading_enabled: true,
            max_position_sol: self.paper_config.max_position_sol,
            min_position_sol: self.trading_config.min_position_sol
                .min(self.paper_config.max_position_sol),
            min_score_to_buy: self.paper_config.min_score_to_buy,
            min_liquidity_usd: self.paper_config.min_liquidity_usd,
            max_positions: self.paper_config.max_positions,
            default_slippage: slippage,
            ..self.trading_config.clone()
        };

        let decision = evaluate_buy_signal(&signal, &paper_buy_config, &paper_positions_snapshot, self.sol_price_usd);
        log_buy_decision(&signal, &decision);

        if let BuyDecision::Buy { amount_sol, .. } = decision {
            let quoted_price = signal.current_price_usd;
            let sol_price_usd = self.sol_price_usd;

            // Calculate price impact before execute_buy (for notification)
            let price_impact = PaperTradingState::calc_price_impact_pct(
                amount_sol, signal.liquidity_usd, sol_price_usd,
            );
            let effective_price = quoted_price * (1.0 + (slippage + price_impact) / 100.0);

            match self.paper_state.execute_buy(
                signal.token_address.clone(),
                signal.symbol.clone(),
                signal.name.clone(),
                quoted_price,
                amount_sol,
                slippage,
                sol_price_usd,
                signal.total_score,
                signal.liquidity_usd,
            ) {
                Ok(_sig) => {
                    let msg = format_paper_buy_notification(
                        &signal.symbol,
                        &signal.name,
                        &signal.token_address,
                        amount_sol,
                        quoted_price,
                        effective_price,
                        slippage,
                        price_impact,
                        signal.total_score,
                        self.paper_state.current_balance_sol,
                        self.paper_state.positions.len(),
                    );
                    let _ = self.send_message(&msg).await;

                    // Auto-save after each buy
                    if let Err(e) = save_paper_state(&self.paper_state) {
                        println!("[PAPER] Failed to save state: {e}");
                    }
                    // Reset no-trade timer
                    self.last_trade_or_buy = Instant::now();
                }
                Err(e) => println!("[PAPER BUY] Skip: {e}"),
            }
        }
    }

    // ============================================================
    // PAPER TRADING - Simulated Sell (check all positions)
    // ============================================================

    async fn check_and_paper_sell(&mut self) {
        if !self.paper_config.enabled || self.paper_state.positions.is_empty() {
            return;
        }

        println!("[PAPER SELL] Checking {} paper positions...", self.paper_state.positions.len());

        // Fetch current prices for all open positions
        let mut prices: HashMap<String, f64> = HashMap::new();
        let addrs: Vec<String> = self.paper_state.positions.keys().cloned().collect();

        for addr in &addrs {
            self.dex_limiter.wait_if_needed().await;
            let url = format!("https://api.dexscreener.com/latest/dex/tokens/{addr}");
            if let Ok(resp) = self.client.get(&url).send().await {
                if resp.status().is_success() {
                    if let Ok(pr) = resp.json::<PairResponse>().await {
                        if let Some(price) = pr.pairs.unwrap_or_default()
                            .into_iter().find(|p| p.chain_id == "solana")
                            .and_then(|p| p.price_usd)
                            .and_then(|p| p.parse::<f64>().ok())
                        {
                            prices.insert(addr.clone(), price);
                        }
                    }
                }
            }
        }

        // Evaluate TP/SL/Trailing — all params from trading_config so paper sell
        // uses exactly the same thresholds as live evaluate_position() in sell_strategy.rs.
        let to_sell = self.paper_state.evaluate_positions(
            &prices,
            self.trading_config.take_profit_percent,
            self.trading_config.stop_loss_percent,
            self.trading_config.trailing_start_percent,
            self.trading_config.trailing_distance_percent,
            self.trading_config.tp1_percent,
            self.trading_config.tp1_sell_percent,
            self.trading_config.tp2_percent,
            self.trading_config.tp2_sell_percent,
            self.trading_config.max_hold_minutes,
            self.trading_config.time_exit_threshold_pct,
            self.trading_config.breakeven_after_tp1,
        );

        // Execute paper sells
        for (addr, reason, sell_price, sell_pct, tp_stage) in to_sell {
            match self.paper_state.execute_sell(&addr, sell_price, sell_pct, self.paper_config.default_slippage, reason, tp_stage) {
                Ok(trade) => {
                    let balance = self.paper_state.current_balance_sol;
                    let msg = format_paper_sell_notification(&trade, balance);
                    let _ = self.send_message(&msg).await;

                    // Circuit breaker + daily limit: only track on full position close —
                    // matches live trading behavior (check_and_sell_positions does the same).
                    // Partial TP1/TP2 sells must NOT trigger these checks, otherwise
                    // a TP1 fire between two SL losses would silently clear the counter
                    // and prevent the circuit breaker from ever triggering.
                    if tp_stage == 0 {
                        self.handle_trade_result(trade.profit_sol).await;
                    }

                    // Save after sell
                    if let Err(e) = save_paper_state(&self.paper_state) {
                        println!("[PAPER] Failed to save state: {e}");
                    }
                }
                Err(e) => println!("[PAPER SELL] Error: {e}"),
            }
            sleep(Duration::from_secs(1)).await;
        }
    }

    // ============================================================
    // PAPER TRADING - Send periodic report
    // ============================================================

    async fn send_paper_report(&mut self, current_prices: &HashMap<String, f64>) {
        if !self.paper_config.enabled { return; }
        let report = format_paper_report(&self.paper_state, current_prices);
        println!("[PAPER] Sending periodic report...");
        let _ = self.send_message(&report).await;
    }

    // ============================================================
    // CIRCUIT BREAKER + DAILY MAX LOSS PROTECTION
    // ============================================================

    /// Called after every full position close (not partial TP).
    /// profit_sol: positive = profit, negative = loss.
    ///
    /// Handles two independent protection layers:
    ///   1. Circuit breaker — pause after N consecutive losses
    ///   2. Daily max loss — pause until next UTC day if daily loss exceeds limit
    async fn handle_trade_result(&mut self, profit_sol: f64) {
        let is_loss = profit_sol < 0.0;

        // -------------------------------------------------------
        // DAILY MAX LOSS PROTECTION
        // Reset counter at UTC midnight, then check limit.
        // -------------------------------------------------------
        let today = Utc::now().date_naive();
        if today != self.daily_loss_date {
            // New UTC day — reset the counter
            if self.daily_limit_paused {
                self.daily_limit_paused = false;
                println!("[DAILY LIMIT] New UTC day — daily loss counter reset, buying resumed");
                let _ = self.send_message(
                    "🌅 **New day — daily loss protection reset.**\nBuy scanning resumed for today."
                ).await;
            }
            self.daily_loss_sol = 0.0;
            self.daily_loss_date = today;
        }

        if is_loss {
            self.daily_loss_sol += profit_sol.abs();
            // Daily loss % is relative to the configured capital base.
            // PAPER_BALANCE_SOL must equal the actual live trading capital so that
            // daily limit protects real money proportionally (see config.env).
            // For live-only mode (paper disabled), set PAPER_BALANCE_SOL to your
            // wallet balance — the bot does not auto-read on-chain balance.
            let initial_bal = self.paper_state.initial_balance_sol
                .max(self.trading_config.max_position_sol * self.trading_config.max_positions as f64); // safe floor
            let daily_loss_pct = self.daily_loss_sol / initial_bal * 100.0;
            let daily_limit_pct = self.trading_config.daily_max_loss_pct;

            println!(
                "[DAILY LIMIT] Today's loss: {:.5} SOL ({:.1}% / {:.1}% limit)",
                self.daily_loss_sol, daily_loss_pct, daily_limit_pct
            );

            if !self.daily_limit_paused && daily_loss_pct >= daily_limit_pct {
                self.daily_limit_paused = true;
                let msg = format!(
                    "🛑 **Daily Loss Limit Reached!**\n\
                    ═══════════════════════════════\n\
                    📉 Lost {:.5} SOL today ({:.1}% of balance)\n\
                    🎯 Limit: {:.1}% — protecting remaining capital\n\
                    ⏸ Buy scanning paused until next UTC day\n\
                    📊 Sell monitoring of open positions continues\n\
                    🌅 Will resume automatically at 00:00 UTC\n\
                    💡 Use /resume to override early",
                    self.daily_loss_sol, daily_loss_pct, daily_limit_pct,
                );
                let _ = self.send_message(&msg).await;
                println!("[DAILY LIMIT] ACTIVATED — buy scan paused until {}", (today + chrono::Duration::days(1)));
            }
        }

        // -------------------------------------------------------
        // CIRCUIT BREAKER — pause after N consecutive losses
        // -------------------------------------------------------
        if is_loss {
            self.consecutive_losses += 1;
            println!("[CIRCUIT] Consecutive losses: {}/{}", self.consecutive_losses, self.circuit_breaker_losses);
            if self.consecutive_losses >= self.circuit_breaker_losses {
                let pause_secs = self.circuit_breaker_pause_hours * 3600;
                self.circuit_breaker_until = Some(Instant::now() + Duration::from_secs(pause_secs));
                let msg = format!(
                    "🔴 **Circuit Breaker Activated!**\n\
                    ═══════════════════════════════\n\
                    ⚠️ {} consecutive losses detected\n\
                    ⏸ Buy scanning paused for {} hours\n\
                    📊 Sell monitoring continues normally\n\
                    💡 Use /resume to override early",
                    self.consecutive_losses,
                    self.circuit_breaker_pause_hours,
                );
                let _ = self.send_message(&msg).await;
                println!("[CIRCUIT] ACTIVATED — buy scan paused for {} hours", self.circuit_breaker_pause_hours);
            }
        } else {
            if self.consecutive_losses > 0 {
                println!("[CIRCUIT] Consecutive losses reset (profitable trade)");
            }
            self.consecutive_losses = 0;
        }
    }

    // ============================================================
    // PEAK HOURS FILTER
    // ============================================================

    fn is_peak_hours() -> bool {
        let hour = Utc::now().hour();
        // 13:00-17:00 UTC (London/NY overlap) and 20:00-00:00 UTC (US evening)
        matches!(hour, 13..=16 | 20..=23)
    }

    // Apply stricter off-peak thresholds — saves originals so they can be restored.
    // Never makes thresholds looser than the current peak-hour values.
    fn activate_off_peak(&mut self) {
        self.off_peak_saved = Some(OffPeakSavedState {
            min_score: self.trading_config.min_score_to_buy,
            paper_min_score: self.paper_config.min_score_to_buy,
            min_liquidity: self.trading_config.min_liquidity_usd,
            max_positions: self.trading_config.max_positions,
            paper_max_positions: self.paper_config.max_positions,
            momentum_max_pct: self.momentum_max_pct,
        });
        self.trading_config.min_score_to_buy = self.off_peak_min_score
            .max(self.trading_config.min_score_to_buy);
        self.paper_config.min_score_to_buy = self.off_peak_min_score
            .max(self.paper_config.min_score_to_buy);
        self.trading_config.min_liquidity_usd = self.off_peak_min_liquidity
            .max(self.trading_config.min_liquidity_usd);
        self.trading_config.max_positions = (self.off_peak_max_positions as usize)
            .min(self.trading_config.max_positions);
        // Mirror position limit to paper config so paper/live use identical constraints
        self.paper_config.max_positions = (self.off_peak_max_positions as usize)
            .min(self.paper_config.max_positions);
        self.momentum_max_pct = self.off_peak_momentum_max_pct
            .min(self.momentum_max_pct);
    }

    // Restore original thresholds after an off-peak scan cycle.
    fn deactivate_off_peak(&mut self) {
        if let Some(saved) = self.off_peak_saved.take() {
            self.trading_config.min_score_to_buy = saved.min_score;
            self.paper_config.min_score_to_buy = saved.paper_min_score;
            self.trading_config.min_liquidity_usd = saved.min_liquidity;
            self.trading_config.max_positions = saved.max_positions;
            self.paper_config.max_positions = saved.paper_max_positions;
            self.momentum_max_pct = saved.momentum_max_pct;
        }
    }

    // ============================================================
    // TELEGRAM COMMAND POLLING
    // ============================================================

    async fn poll_telegram_commands(&mut self) {
        // Exponential backoff when Telegram API is unavailable:
        // 0 failures → no extra wait | 1 → 30s | 2 → 60s | 3 → 120s | 4+ → 300s
        let backoff_secs: u64 = match self.tg_poll_failures {
            0 => 0,
            1 => 30,
            2 => 60,
            3 => 120,
            _ => 300,
        };
        if self.last_tg_poll.elapsed() < Duration::from_secs(backoff_secs) {
            return;
        }
        self.last_tg_poll = Instant::now();

        let url = format!(
            "https://api.telegram.org/bot{}/getUpdates?offset={}&limit=20&timeout=0",
            self.telegram_token,
            self.last_update_id + 1,
        );

        let resp = match self.client.get(&url).send().await {
            Ok(r) => r,
            Err(e) => {
                self.tg_poll_failures += 1;
                match self.tg_poll_failures {
                    1 => println!("[TG POLL] Request failed — backing off ({}s): {}", backoff_secs.max(30), e),
                    n if n % 10 == 0 => println!("[TG POLL] Still unreachable after {} attempts — next retry in {}s", n, 300_u64.min(backoff_secs * 2)),
                    _ => {}
                }
                return;
            }
        };

        // Handle rate-limit response (HTTP 429)
        if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            self.tg_poll_failures += 1;
            if self.tg_poll_failures == 1 {
                println!("[TG POLL] Rate limited by Telegram (429) — backing off {}s", backoff_secs.max(30));
            }
            return;
        }

        let json: serde_json::Value = match resp.json().await {
            Ok(j) => j,
            Err(e) => {
                self.tg_poll_failures += 1;
                if self.tg_poll_failures == 1 {
                    println!("[TG POLL] Failed to parse Telegram response: {e}");
                }
                return;
            }
        };

        // Restore from backoff
        if self.tg_poll_failures > 0 {
            println!("[TG POLL] Connection restored after {} failed attempt(s)", self.tg_poll_failures);
            self.tg_poll_failures = 0;
        }

        let updates = match json["result"].as_array() {
            Some(u) if !u.is_empty() => u.clone(),
            _ => return,
        };

        for update in &updates {
            let update_id = update["update_id"].as_i64().unwrap_or(0);
            if update_id > self.last_update_id {
                self.last_update_id = update_id;
            }

            // --- Handle inline button presses (callback_query) ---
            if !update["callback_query"].is_null() {
                let cb_id   = update["callback_query"]["id"].as_str().unwrap_or("");
                let cb_data = update["callback_query"]["data"].as_str().unwrap_or("");
                let cb_chat = update["callback_query"]["message"]["chat"]["id"].as_i64().unwrap_or(0);

                if cb_chat.to_string() != self.telegram_chat_id {
                    println!("[TG CALLBACK] Ignored — chat_id {} != configured {}", cb_chat, self.telegram_chat_id);
                    self.answer_callback_query(cb_id).await;
                    continue;
                }

                println!("[TG CALLBACK] Button pressed: {cb_data}");
                // Answer immediately to remove Telegram's loading spinner
                self.answer_callback_query(cb_id).await;

                match cb_data {
                    "/pause" => {
                        self.is_paused = true;
                        let _ = self.send_message(
                            "⏸ *Bot paused via button.*\nSell monitoring continues. Use /resume to restart buying."
                        ).await;
                    }
                    "/resume" => {
                        self.is_paused = false;
                        self.circuit_breaker_until = None;
                        self.consecutive_losses = 0;
                        let _ = self.send_message(
                            "▶️ *Bot resumed.*\nBuy scanning active. Circuit breaker cleared."
                        ).await;
                    }
                    "/status" | "/stats" => {
                        let msg = self.build_status_message();
                        let _ = self.send_message(&msg).await;
                    }
                    "/trades" => {
                        let msg = self.build_trades_message();
                        let _ = self.send_message(&msg).await;
                    }
                    _ => {}
                }
                continue;
            }

            // --- Handle text messages and channel posts ---
            let text = update["message"]["text"].as_str()
                .or_else(|| update["channel_post"]["text"].as_str())
                .unwrap_or("");
            let chat_id_num = update["message"]["chat"]["id"].as_i64()
                .or_else(|| update["channel_post"]["chat"]["id"].as_i64())
                .unwrap_or(0);

            // Skip non-message updates (e.g. edited messages)
            if chat_id_num == 0 {
                continue;
            }

            // Only respond to our configured chat
            if chat_id_num.to_string() != self.telegram_chat_id {
                println!("[TG CMD] Ignored message from unknown chat_id: {} (expected: {})", chat_id_num, self.telegram_chat_id);
                continue;
            }

            // Extract command (strip @botname suffix if present)
            let cmd = text.split_whitespace().next().unwrap_or("").split('@').next().unwrap_or("");
            if cmd.starts_with('/') {
                println!("[TG CMD] Received: {cmd}");
            }

            match cmd {
                "/status" => {
                    let msg = self.build_status_message();
                    if let Err(e) = self.send_message(&msg).await {
                        println!("[TG CMD] Failed to send /status reply: {e}");
                    }
                }
                "/pause" => {
                    self.is_paused = true;
                    let _ = self.send_message(
                        "⏸ *Bot paused manually.*\nSell monitoring continues. Use /resume to restart buying."
                    ).await;
                }
                "/resume" => {
                    self.is_paused = false;
                    self.circuit_breaker_until = None;
                    self.consecutive_losses = 0;
                    let _ = self.send_message(
                        "▶️ *Bot resumed.*\nBuy scanning active. Circuit breaker cleared."
                    ).await;
                }
                "/trades" => {
                    let msg = self.build_trades_message();
                    let _ = self.send_message(&msg).await;
                }
                "/score" => {
                    let drift = self.dynamic_min_score - self.base_min_score;
                    let drift_str = if drift.abs() < 0.1 {
                        "no drift".to_string()
                    } else if drift > 0.0 {
                        format!("+{:.1} above base (raised by auto-adjust)", drift)
                    } else {
                        format!("{:.1} below base", drift)
                    };
                    let trades_count = self.paper_state.closed_trades.len();
                    let msg = format!(
                        "🎯 *Score Threshold*\n\
                        Current (dynamic): *{:.1}*/100\n\
                        Base (config): {:.1}/100\n\
                        Drift: {}\n\n\
                        Closed trades tracked: {}\n\
                        Auto-adjusts every hour based on last 20 trades.\n\
                        _Max drift allowed: +10 above base (bug fix applied)_",
                        self.dynamic_min_score,
                        self.base_min_score,
                        drift_str,
                        trades_count,
                    );
                    let _ = self.send_message(&msg).await;
                }
                "/blacklist" => {
                    let parts: Vec<&str> = text.split_whitespace().collect();
                    if let Some(addr) = parts.get(1) {
                        self.data.blacklisted_tokens.insert(addr.to_string());
                        // Save immediately so the blacklist survives a crash before
                        // the next periodic save interval
                        let _ = self.save();
                        let _ = self.send_message(&format!(
                            "⛔ Token `{}` added to blacklist ({} total blocked)",
                            addr, self.data.blacklisted_tokens.len()
                        )).await;
                        println!("[BLACKLIST] Added & saved: {addr}");
                    } else {
                        let _ = self.send_message(&format!(
                            "⛔ *Blacklisted tokens:* {}\n\
                            Usage: `/blacklist <token_address>`",
                            self.data.blacklisted_tokens.len()
                        )).await;
                    }
                }
                "/helius" => {
                    let report = self.test_helius_keys().await;
                    let _ = self.send_message(&report).await;
                }
                _ => {}
            }
        }
    }

    fn build_status_message(&self) -> String {
        let cb_status = if self.daily_limit_paused {
            let initial_bal = self.paper_state.initial_balance_sol.max(0.01);
            let pct = self.daily_loss_sol / initial_bal * 100.0;
            format!("🛑 Daily limit ({:.1}% lost today — resumes at 00:00 UTC)", pct)
        } else if let Some(until) = self.circuit_breaker_until {
            if until > Instant::now() {
                let remaining_mins = until.duration_since(Instant::now()).as_secs() / 60;
                format!("🔴 Circuit breaker ({remaining_mins} min left)")
            } else {
                "🟢 Active".to_string()
            }
        } else if self.is_paused {
            "⏸ Manually paused".to_string()
        } else {
            "🟢 Active".to_string()
        };

        let initial_bal = self.paper_state.initial_balance_sol.max(0.01);
        let daily_loss_pct = self.daily_loss_sol / initial_bal * 100.0;
        let daily_protection_line = format!(
            "🛡️ Daily loss: {:.5} SOL ({:.1}% / {:.1}% limit)",
            self.daily_loss_sol, daily_loss_pct, self.trading_config.daily_max_loss_pct,
        );

        let total_pnl = self.paper_state.total_profit_sol - self.paper_state.total_loss_sol;
        let roi = if self.paper_state.initial_balance_sol > 0.0 {
            total_pnl / self.paper_state.initial_balance_sol * 100.0
        } else { 0.0 };

        let helius_health = if self.helius_consecutive_failures == 0 {
            "✅ OK".to_string()
        } else {
            format!("⚠️ {} consecutive fails", self.helius_consecutive_failures)
        };
        let best_score_str = if self.best_score_seen > 0.0 {
            format!("{:.1}/100", self.best_score_seen)
        } else {
            "none yet".to_string()
        };

        format!(
            "📊 **Basol Bot Status**\n\
            ═══════════════════════════════\n\
            🤖 Status: {}\n\
            💰 SOL Price: ${:.2}\n\
            🎯 Score Threshold: {:.1}/100 (base: {:.1})\n\
            📈 Consecutive Losses: {}/{}\n\
            🌍 Peak Hours Only: {}\n\n\
            🛡️ **Capital Protection:**\n\
            {}\n\
            🔰 Break-even stop after TP1: {}\n\n\
            🔬 **Scan Health:**\n\
            🏆 Best score seen: {}\n\
            ✅ Tokens qualified: {}\n\
            🔑 Helius API: {}\n\
            👁 Seen: {} tokens | ⛔ Blacklisted: {}\n\n\
            💼 **Paper Trading:**\n\
            💵 Balance: {:.4} SOL\n\
            📊 Open Positions: {}\n\
            🏆 Win Rate: {:.1}%\n\
            💹 Profit Factor: {:.2}\n\
            💰 Total P&L: {}{:.5} SOL ({}{:.1}% ROI)\n\
            📈 Total Trades: {}\n\n\
            🔒 **Live Trading:** {}",
            cb_status,
            self.sol_price_usd,
            self.dynamic_min_score,
            self.base_min_score,
            self.consecutive_losses, self.circuit_breaker_losses,
            if self.peak_hours_only {
                if self.off_peak_trading_enabled { "ON + off-peak STRICT" } else { "ON" }
            } else { "OFF" },
            daily_protection_line,
            if self.trading_config.breakeven_after_tp1 { "✅ ON" } else { "❌ OFF" },
            best_score_str,
            self.tokens_qualified_session,
            helius_health,
            self.data.seen_tokens.len(), self.data.blacklisted_tokens.len(),
            self.paper_state.current_balance_sol,
            self.paper_state.positions.len(),
            self.paper_state.win_rate(),
            self.paper_state.profit_factor(),
            if total_pnl >= 0.0 { "+" } else { "" }, total_pnl,
            if roi >= 0.0 { "+" } else { "" }, roi,
            self.paper_state.total_sells,
            if self.trading_config.trading_enabled && self.wallet.is_some() { "🟢 ACTIVE" } else { "🔴 INACTIVE" },
        )
    }

    fn build_trades_message(&self) -> String {
        let recent: Vec<_> = self.paper_state.closed_trades.iter().rev().take(10).collect();
        if recent.is_empty() {
            return "📋 **No closed trades yet.**\nPaper trading is warming up!".to_string();
        }
        let mut msg = format!(
            "📋 **Last {} Paper Trades:**\n═══════════════════════════════\n",
            recent.len()
        );
        for trade in &recent {
            let emoji = if trade.profit_percent >= 0.0 { "✅" } else { "❌" };
            let hold = if trade.hold_duration_minutes < 60 {
                format!("{}m", trade.hold_duration_minutes)
            } else {
                format!("{:.1}h", trade.hold_duration_minutes as f64 / 60.0)
            };
            msg.push_str(&format!(
                "{} **{}** | {}{:.1}% | {}{:.5} SOL | {} | {}\n",
                emoji,
                trade.symbol,
                if trade.profit_percent >= 0.0 { "+" } else { "" },
                trade.profit_percent,
                if trade.profit_sol >= 0.0 { "+" } else { "" },
                trade.profit_sol,
                hold,
                trade.exit_reason,
            ));
        }
        msg.push_str(&format!(
            "\n📊 Win Rate: {:.1}% | P.Factor: {:.2}\n\
            📈 Best: +{:.1}% ({}) | Worst: {:.1}% ({})",
            self.paper_state.win_rate(),
            self.paper_state.profit_factor(),
            self.paper_state.best_trade_pct, self.paper_state.best_trade_symbol,
            self.paper_state.worst_trade_pct, self.paper_state.worst_trade_symbol,
        ));
        msg
    }

    // ============================================================
    // AUTO-ADJUST SCORE THRESHOLD (based on rolling win rate)
    // ============================================================

    fn adjust_dynamic_score(&mut self) {
        // Only adjust every 60 minutes
        if self.last_score_adjust.elapsed() < Duration::from_secs(3600) {
            return;
        }
        self.last_score_adjust = Instant::now();

        if self.paper_state.closed_trades.len() < 5 {
            return; // Need at least 5 trades to make a meaningful adjustment
        }

        // Use last 20 paper trades for rolling win rate.
        // NOTE: if paper trading is disabled (PAPER_TRADING_ENABLED=false), closed_trades
        // is always empty and this function never adjusts the score. In live-only mode,
        // set PAPER_TRADING_ENABLED=true to enable score auto-adjustment.
        let recent: Vec<_> = self.paper_state.closed_trades.iter().rev().take(20).collect();
        let wins = recent.iter().filter(|t| t.profit_percent > 0.0).count();
        let win_rate = wins as f64 / recent.len() as f64 * 100.0;

        let old_score = self.dynamic_min_score;
        // Use the immutable base score from config — not the mutable trading_config value
        // which may have already been raised by a previous dynamic adjustment.
        // Without this fix the floor creeps up permanently and score can reach 95
        // after a losing streak, making it nearly impossible to ever trade again.
        let base_score = self.base_min_score;

        if win_rate < 40.0 {
            // Struggling — raise the bar to filter more aggressively (max +10 above base)
            self.dynamic_min_score = (self.dynamic_min_score + 2.0).min(base_score + 10.0).min(95.0);
        } else if win_rate > 60.0 {
            // Performing well — gently relax back toward the configured base
            self.dynamic_min_score = (self.dynamic_min_score - 1.0).max(base_score);
        }

        if (self.dynamic_min_score - old_score).abs() > 0.01 {
            println!(
                "[SCORE] Auto-adjusted: {:.1} → {:.1} (rolling win rate: {:.1}% over {} trades, base: {:.1})",
                old_score, self.dynamic_min_score, win_rate, recent.len(), base_score
            );
            // Propagate to both live and paper buy configs
            self.trading_config.min_score_to_buy = self.dynamic_min_score;
            self.paper_config.min_score_to_buy = self.dynamic_min_score;
        }
    }

    // ============================================================
    // SCAN HEALTH WARNING — fires every 6h if no trade has happened
    // ============================================================

    async fn send_health_warning_if_needed(&mut self) {
        // Fire if no paper trade has happened in the last 6h.
        // Uses last_trade_or_buy (reset on every buy) — correct even after many sessions.
        // NOTE: do NOT gate on paper_state.total_buys > 0 — that silences the warning
        // permanently after the very first trade, even when the bot later stops trading.
        let no_trade_hours = 6u64;
        if self.last_trade_or_buy.elapsed() < Duration::from_secs(no_trade_hours * 3600) {
            return;
        }
        // Don't spam — only once every 6h
        if self.last_health_warning.elapsed() < Duration::from_secs(no_trade_hours * 3600) {
            return;
        }
        self.last_health_warning = Instant::now();

        let helius_status = if self.helius_consecutive_failures == 0 {
            "✅ OK".to_string()
        } else {
            format!("⚠️ {} consecutive failures", self.helius_consecutive_failures)
        };

        let best_score_str = if self.best_score_seen > 0.0 {
            format!("{:.1}/100", self.best_score_seen)
        } else {
            "no token qualified yet".to_string()
        };

        let hours_running = self.last_trade_or_buy.elapsed().as_secs() / 3600;

        let msg = format!(
            "⚠️ **No Trade Alert**\n\
            ═══════════════════════════════\n\
            🕐 No paper trade in {}h+\n\n\
            📊 **Scan Health:**\n\
            🔬 Tokens passed full analysis: {}\n\
            🏆 Best score seen: {}\n\
            🎯 Current threshold: {:.1}/100\n\
            🔑 Helius API: {}\n\
            📦 Seen tokens total: {}\n\n\
            🔍 **Possible causes:**\n\
            • Score threshold too high for current market\n  → Try `/score` then lower `MIN_SCORE_TO_BUY` in config.env\n\
            • All tokens filtered by liquidity/momentum/bundled check\n\
            • DexScreener not returning new Solana tokens\n\n\
            💡 Send `/status` for full bot state\n\
            💡 Send `/helius` to recheck API keys",
            hours_running,
            self.tokens_qualified_session,
            best_score_str,
            self.dynamic_min_score,
            helius_status,
            self.data.seen_tokens.len(),
        );
        println!("[HEALTH] ⚠️ No trade warning sent to Telegram");
        let _ = self.send_message(&msg).await;
    }

    // ============================================================
    // SCHEDULED REPORTS
    // ============================================================

    async fn send_daily_report_if_needed(&mut self) {
        let today = Utc::now().format("%Y-%m-%d").to_string();
        if self.last_daily_report_date == today {
            return;
        }
        self.last_daily_report_date = today.clone();

        let total_pnl = self.paper_state.total_profit_sol - self.paper_state.total_loss_sol;
        let roi = if self.paper_state.initial_balance_sol > 0.0 {
            total_pnl / self.paper_state.initial_balance_sol * 100.0
        } else { 0.0 };

        let msg = format!(
            "📅 **Daily Report — {}**\n\
            ═══════════════════════════════\n\
            💼 **Paper Portfolio:**\n\
            💵 Balance: {:.4} SOL (started: {:.4})\n\
            📈 ROI: {}{:.2}%\n\
            📊 Open Positions: {}\n\n\
            📊 **All-time Paper Stats:**\n\
            Total Trades: {}\n\
            Win Rate: {:.1}%\n\
            Profit Factor: {:.2}\n\
            Total P&L: {}{:.5} SOL\n\
            Best Trade: +{:.1}% ({})\n\
            Worst Trade: {:.1}% ({})\n\n\
            🎯 Score Threshold: {:.1}/100\n\
            🛡 Circuit Breaker: {} consecutive losses\n\
            🔒 Live Trading: {}",
            today,
            self.paper_state.current_balance_sol,
            self.paper_state.initial_balance_sol,
            if roi >= 0.0 { "+" } else { "" }, roi,
            self.paper_state.positions.len(),
            self.paper_state.total_sells,
            self.paper_state.win_rate(),
            self.paper_state.profit_factor(),
            if total_pnl >= 0.0 { "+" } else { "" }, total_pnl,
            self.paper_state.best_trade_pct, self.paper_state.best_trade_symbol,
            self.paper_state.worst_trade_pct, self.paper_state.worst_trade_symbol,
            self.dynamic_min_score,
            self.consecutive_losses,
            if self.trading_config.trading_enabled && self.wallet.is_some() { "🟢 ON" } else { "🔴 OFF" },
        );
        let _ = self.send_message(&msg).await;
        println!("[DAILY] Report sent for {today}");
    }

    async fn send_weekly_report_if_needed(&mut self) {
        let now = Utc::now();
        // Only fire on Monday at or after 06:00 UTC
        if now.weekday() != chrono::Weekday::Mon || now.hour() < 6 {
            return;
        }
        let week_key = format!("{}-W{:02}", now.year(), now.iso_week().week());
        if self.last_weekly_report_date == week_key {
            return;
        }
        self.last_weekly_report_date = week_key.clone();

        let total_pnl = self.paper_state.total_profit_sol - self.paper_state.total_loss_sol;
        let roi = if self.paper_state.initial_balance_sol > 0.0 {
            total_pnl / self.paper_state.initial_balance_sol * 100.0
        } else { 0.0 };

        let msg = format!(
            "📅 **Weekly Report — {}**\n\
            ═══════════════════════════════\n\
            💼 **Paper Portfolio:**\n\
            💵 Balance: {:.4} SOL (started: {:.4})\n\
            📈 Total ROI: {}{:.2}%\n\
            📊 Open Positions: {}\n\n\
            📊 **All-time Paper Stats:**\n\
            Total Trades: {} | Win Rate: {:.1}%\n\
            Profit Factor: {:.2}\n\
            Total P&L: {}{:.5} SOL\n\
            Best: +{:.1}% ({}) | Worst: {:.1}% ({})\n\n\
            🎯 Score Threshold: {:.1}/100\n\
            👁 Tokens seen: {} | ⛔ Blacklisted: {}\n\n\
            💡 Run backtest: `cargo run -- --backtest`",
            week_key,
            self.paper_state.current_balance_sol,
            self.paper_state.initial_balance_sol,
            if roi >= 0.0 { "+" } else { "" }, roi,
            self.paper_state.positions.len(),
            self.paper_state.total_sells,
            self.paper_state.win_rate(),
            self.paper_state.profit_factor(),
            if total_pnl >= 0.0 { "+" } else { "" }, total_pnl,
            self.paper_state.best_trade_pct, self.paper_state.best_trade_symbol,
            self.paper_state.worst_trade_pct, self.paper_state.worst_trade_symbol,
            self.dynamic_min_score,
            self.data.seen_tokens.len(), self.data.blacklisted_tokens.len(),
        );
        let _ = self.send_message(&msg).await;
        println!("[WEEKLY] Report sent for {week_key}");
    }

    // ============================================================
    // PROFIT TRACKING
    // ============================================================

    async fn check_profits(&mut self) {
        let addresses: Vec<String> = self.data.tracked_tokens.keys().cloned().collect();
        for addr in addresses {
            self.dex_limiter.wait_if_needed().await;
            let url = format!("https://api.dexscreener.com/latest/dex/tokens/{addr}");
            let current_price = match self.client.get(&url).send().await {
                Ok(r) if r.status().is_success() => {
                    r.json::<PairResponse>().await.ok()
                        .and_then(|pr| pr.pairs)
                        .and_then(|ps| ps.into_iter().next())
                        .and_then(|p| p.price_usd)
                        .and_then(|p| p.parse::<f64>().ok())
                }
                _ => None,
            };

            if let Some(price) = current_price {
                let (initial_price, current_milestones, token_name, token_symbol) = {
                    let Some(token) = self.data.tracked_tokens.get_mut(&addr) else { continue };
                    if price > token.highest_price { token.highest_price = price; }
                    (
                        token.initial_price,
                        token.milestones_reached.clone(),
                        token.name.clone(),
                        token.symbol.clone(),
                    )
                };

                let pct = (price - initial_price) / initial_price * 100.0;
                let milestones = [50u32, 100, 200, 500, 1000, 2000];
                let new_milestones: Vec<u32> = milestones.iter()
                    .filter(|&&ms| pct >= ms as f64 && !current_milestones.contains(&ms))
                    .copied()
                    .collect();

                for ms in new_milestones {
                    if let Some(token) = self.data.tracked_tokens.get_mut(&addr) {
                        token.milestones_reached.push(ms);
                    }

                    let (emoji, title) = match ms {
                        50   => ("🎉", "Great Start!"),
                        100  => ("🚀💎", "Capital Doubled!"),
                        200  => ("🔥💰", "Exceptional Gain!"),
                        500  => ("⭐🏆", "Legendary Performance!"),
                        1000 => ("👑💎", "10x Golden Token!"),
                        _    => ("🌟🚀", "Insane Gain!"),
                    };
                    let hours = Utc::now()
                        .signed_duration_since(
                            self.data.tracked_tokens.get(&addr)
                                .map(|t| t.discovery_time)
                                .unwrap_or(Utc::now())
                        ).num_hours();
                    let msg = format!(
                        "{emoji} **{title}!** {emoji}\n═══════════════════════════════\n\n\
                        💎 Token: **{token_name}** `({token_symbol})`\n\
                        📈 Gain: **+{pct:.1}%**\n\
                        💰 Discovery Price: **${initial_price:.8}**\n\
                        💰 Current Price: **${price:.8}**\n\
                        ⏰ Since Discovery: **{hours} hours**\n\n\
                        🎉 **Congrats to all followers!**\n\
                        🤖 This opportunity was discovered by the bot"
                    );
                    let _ = self.send_message(&msg).await;

                    if pct >= 100.0 { self.data.performance_stats.tokens_reached_2x += 1; }
                    if pct >= 400.0 { self.data.performance_stats.tokens_reached_5x += 1; }
                    if pct >= 900.0 { self.data.performance_stats.tokens_reached_10x += 1; }
                    if pct > self.data.performance_stats.best_token_gain_percent {
                        self.data.performance_stats.best_token_gain_percent = pct;
                        self.data.performance_stats.best_token_symbol = token_symbol.clone();
                    }
                    sleep(Duration::from_secs(2)).await;
                }
            }
        }
    }

    // ============================================================
    // MAIN LOOP
    // ============================================================

    async fn run(&mut self) {
        println!("🚀 Basol Bot v3.0 starting...");
        println!("📊 Trading: {} | Max position: {:.2} SOL | TP: {:.1}% | SL: {:.1}%",
            if self.trading_config.trading_enabled && self.wallet.is_some() { "ACTIVE" } else { "INACTIVE" },
            self.trading_config.max_position_sol,
            self.trading_config.take_profit_percent,
            self.trading_config.stop_loss_percent,
        );

        self.load();

        // Test all Helius keys on startup and log results
        let key_test = self.test_helius_keys().await;
        println!("{}", key_test.replace("**", "").replace('`', ""));

        let startup_msg = format!(
            "🤖 **Basol Bot v3.0 Started!**\n\
            ═══════════════════════════════\n\
            📊 Trading Mode: {}\n\
            💰 Max Position: {:.2} SOL\n\
            📈 Take Profit: {:.1}% | 🛑 Stop Loss: {:.1}%\n\
            🔄 Trailing: after +{:.1}%, distance {:.1}%\n\
            🔍 Min Buy Score: {:.1}/100 (auto-adjusts)\n\
            💧 Min Liquidity: ${:.0}\n\n\
            🆕 **Active Features:**\n\
            🛡 Circuit breaker: pause after {} losses ({:.0}h)\n\
            ⏱ Momentum filter: skip if +{:.0}% h1 already\n\
            🌍 Peak hours only: {}\n\
            🔑 Helius keys: {} loaded (auto-rotation on 429)\n\
            📋 Commands: /status /pause /resume /trades /score /blacklist /helius",
            if self.trading_config.trading_enabled && self.wallet.is_some() { "🟢 ACTIVE" } else { "🔴 ANALYSIS ONLY" },
            self.trading_config.max_position_sol,
            self.trading_config.take_profit_percent,
            self.trading_config.stop_loss_percent,
            self.trading_config.trailing_start_percent,
            self.trading_config.trailing_distance_percent,
            self.dynamic_min_score,
            self.trading_config.min_liquidity_usd,
            self.circuit_breaker_losses,
            self.circuit_breaker_pause_hours as f64,
            self.momentum_max_pct,
            if self.peak_hours_only { "ON (13-17 & 20-00 UTC)" } else { "OFF" },
            self.helius_keys.key_count(),
        );
        let _ = self.send_message(&startup_msg).await;

        let mut last_profit_check = Instant::now();
        let mut scan_count = 0u64;

        loop {
            self.reset_daily_count_if_needed();

            // -------------------------------------------------------
            // AUTO-UPDATE SOL PRICE (every 5 minutes)
            // -------------------------------------------------------
            if self.last_price_update.elapsed() >= Duration::from_secs(SOL_PRICE_UPDATE_INTERVAL_SECS) {
                let new_price = self.fetch_sol_price().await;
                if (new_price - self.sol_price_usd).abs() > 0.01 {
                    println!(
                        "[SOL PRICE] Updated: ${:.2} → ${:.2}",
                        self.sol_price_usd, new_price
                    );
                }
                self.sol_price_usd = new_price;
                self.last_price_update = Instant::now();
            }

            // -------------------------------------------------------
            // TELEGRAM COMMAND POLLING (every scan cycle)
            // -------------------------------------------------------
            self.poll_telegram_commands().await;

            // -------------------------------------------------------
            // AUTO-ADJUST SCORE THRESHOLD (every hour, needs ≥5 trades)
            // -------------------------------------------------------
            self.adjust_dynamic_score();

            // -------------------------------------------------------
            // DAILY REPORT (fires at midnight UTC)
            // -------------------------------------------------------
            self.send_daily_report_if_needed().await;

            // -------------------------------------------------------
            // WEEKLY REPORT (fires Monday ≥06:00 UTC)
            // -------------------------------------------------------
            self.send_weekly_report_if_needed().await;

            // -------------------------------------------------------
            // CHECK & SELL ACTIVE POSITIONS (every 60 seconds)
            // -------------------------------------------------------
            if self.last_sell_check.elapsed() >= Duration::from_secs(SELL_CHECK_INTERVAL_SECS) {
                self.check_and_sell_positions().await;
                self.check_and_paper_sell().await;
                self.last_sell_check = Instant::now();
            }

            // -------------------------------------------------------
            // PAPER TRADING - Periodic report
            // -------------------------------------------------------
            if self.paper_config.enabled
                && self.last_paper_report.elapsed() >= Duration::from_secs(self.paper_config.report_interval_secs)
            {
                let mut prices: HashMap<String, f64> = HashMap::new();
                for addr in self.paper_state.positions.keys() {
                    if let Ok(resp) = self.client
                        .get(format!("https://api.dexscreener.com/latest/dex/tokens/{addr}"))
                        .send().await
                    {
                        if let Ok(pr) = resp.json::<PairResponse>().await {
                            if let Some(price) = pr.pairs.unwrap_or_default()
                                .into_iter().find(|p| p.chain_id == "solana")
                                .and_then(|p| p.price_usd)
                                .and_then(|p| p.parse::<f64>().ok())
                            {
                                prices.insert(addr.clone(), price);
                            }
                        }
                    }
                }
                self.send_paper_report(&prices).await;
                self.last_paper_report = Instant::now();
            }

            // -------------------------------------------------------
            // SCAN HEALTH WARNING — alert if no trade in 6h
            // -------------------------------------------------------
            self.send_health_warning_if_needed().await;

            // -------------------------------------------------------
            // CIRCUIT BREAKER — reset if timeout expired
            // -------------------------------------------------------
            if let Some(until) = self.circuit_breaker_until {
                if Instant::now() >= until {
                    self.circuit_breaker_until = None;
                    self.consecutive_losses = 0;
                    let _ = self.send_message(
                        "✅ **Circuit breaker reset.** Buy scanning resumed automatically."
                    ).await;
                    println!("[CIRCUIT] Expired — buy scanning resumed");
                }
            }
            let circuit_broken = self.circuit_breaker_until
                .map(|u| u > Instant::now())
                .unwrap_or(false);

            // Daily max loss protection — reset at UTC midnight if needed
            let today = Utc::now().date_naive();
            if self.daily_limit_paused && today != self.daily_loss_date {
                self.daily_limit_paused = false;
                self.daily_loss_sol = 0.0;
                self.daily_loss_date = today;
                println!("[DAILY LIMIT] New UTC day — daily loss counter reset, buying resumed");
                let _ = self.send_message(
                    "🌅 **New day — daily loss protection reset.**\nBuy scanning resumed for today."
                ).await;
            }

            // When paused, circuit-broken, or daily limit hit: skip LIVE buy only.
            // Paper trading continues scanning so simulation stays accurate.
            let live_buy_blocked = self.is_paused || circuit_broken || self.daily_limit_paused;
            if live_buy_blocked {
                let reason = if self.daily_limit_paused {
                    format!("daily loss limit ({:.1}% reached)", self.trading_config.daily_max_loss_pct)
                } else if circuit_broken {
                    format!("circuit breaker ({} consec. losses)", self.consecutive_losses)
                } else {
                    "manual pause".to_string()
                };
                println!("⏸ Live buy paused ({reason}) — paper trading + sell monitoring still active");
            }

            // -------------------------------------------------------
            // PEAK HOURS FILTER — skip buying outside high-volume windows
            // -------------------------------------------------------
            if self.peak_hours_only && !Self::is_peak_hours() {
                if !self.off_peak_trading_enabled {
                    println!("🌙 Off-peak hours ({:02}:00 UTC) — buy scan skipped", Utc::now().hour());
                    // Poll Telegram every 3 seconds even during off-peak
                    let mut waited = 0u64;
                    while waited < SCAN_INTERVAL_SECS {
                        let tick = 3u64.min(SCAN_INTERVAL_SECS - waited);
                        sleep(Duration::from_secs(tick)).await;
                        self.poll_telegram_commands().await;
                        waited += tick;
                    }
                    continue;
                }
                // Off-peak trading enabled — scan but with stricter thresholds
                println!(
                    "🌙 Off-peak ({:02}:00 UTC) — STRICT mode: score≥{:.0} liq≥${:.0} max {}pos momentum<{:.0}%",
                    Utc::now().hour(),
                    self.off_peak_min_score,
                    self.off_peak_min_liquidity,
                    self.off_peak_max_positions,
                    self.off_peak_momentum_max_pct,
                );
                self.activate_off_peak();
            }

            scan_count += 1;
            println!("\n{}", "=".repeat(50));
            println!("🔍 Scan #{} - {} - {} tokens seen, {} active positions",
                scan_count,
                Utc::now().format("%H:%M:%S"),
                self.data.seen_tokens.len(),
                self.positions.len(),
            );

            // -------------------------------------------------------
            // SCAN NEW TOKENS
            // -------------------------------------------------------
            let tokens = match self.get_new_solana_tokens().await {
                Ok(t) => t,
                Err(e) => {
                    println!("❌ Failed to fetch tokens: {e}");
                    sleep(Duration::from_secs(SCAN_INTERVAL_SECS)).await;
                    continue;
                }
            };

            let new_tokens: Vec<DexToken> = tokens.into_iter()
                .filter(|t| !self.data.seen_tokens.contains_key(&t.token_address))
                .collect();

            println!("📊 {} new tokens found for analysis", new_tokens.len());

            // Analyze new tokens — up to 20 per scan.
            // Tokens beyond take(20) stay unseen and will be retried next scan.
            // Tokens that ARE analyzed (pass or fail) get marked seen immediately
            // to avoid re-analysis on the next cycle.
            for token in new_tokens.iter().take(20) {
                // Mark as seen before analysis so transient failures don't cause
                // infinite retry loops; tokens beyond take(20) are NOT marked here.
                self.data.seen_tokens.insert(
                    token.token_address.clone(),
                    Utc::now().to_rfc3339(),
                );

                print!("🔬 Analyzing {} ({})... ",
                    token.name.as_deref().unwrap_or("?"),
                    token.symbol.as_deref().unwrap_or("?")
                );

                if let Some(analysis) = self.full_analyze(token).await {
                    println!("✅ Score: {:.1}/100", analysis.total_score);
                    // Track health stats
                    self.tokens_qualified_session += 1;
                    if analysis.total_score > self.best_score_seen {
                        self.best_score_seen = analysis.total_score;
                    }

                    // Telegram alert — gated by daily limit.
                    // Trading (live + paper) continues regardless of alert quota.
                    if self.data.daily_alert_count < MAX_DAILY_ALERTS {
                        let msg = self.format_alert(&analysis);
                        let dex_url = analysis.dex_urls.first().cloned().unwrap_or_default();
                        self.send_alert_with_buttons(&msg, &dex_url).await;

                        // Send image if available
                        if let Some(img) = &analysis.image_url {
                            let caption = format!("{} ({}) - Score: {:.1}/100", analysis.name, analysis.symbol, analysis.total_score);
                            self.send_photo(img, &caption).await;
                        }

                        self.data.daily_alert_count += 1;
                        self.data.performance_stats.total_alerts_sent += 1;
                    } else {
                        println!("⚠️ Daily Telegram limit ({MAX_DAILY_ALERTS}) reached — alert skipped, trade eval continues");
                    }

                    // Profit tracking
                    if let Some(price) = analysis.price_usd {
                        if price > 0.0 && !self.data.tracked_tokens.contains_key(&analysis.token_address) {
                            self.data.tracked_tokens.insert(
                                analysis.token_address.clone(),
                                TrackedToken {
                                    token_address: analysis.token_address.clone(),
                                    symbol: analysis.symbol.clone(),
                                    name: analysis.name.clone(),
                                    image_url: analysis.image_url.clone(),
                                    initial_price: price,
                                    highest_price: price,
                                    discovery_time: Utc::now(),
                                    milestones_reached: vec![],
                                },
                            );
                        }
                    }

                    // -----------------------------------------------
                    // AUTO BUY (live) - skip if paused/circuit broken
                    // -----------------------------------------------
                    if !live_buy_blocked {
                        self.check_and_buy(&analysis).await;
                    }

                    // -----------------------------------------------
                    // PAPER BUY - same gates as live buy so simulation
                    // mirrors exactly what live would do
                    // -----------------------------------------------
                    if !live_buy_blocked {
                        self.check_and_paper_buy(&analysis).await;
                    }

                    sleep(Duration::from_secs(3)).await;
                } else {
                    println!("⏭ Skip");
                }

                sleep(Duration::from_millis(500)).await;
            }

            // -------------------------------------------------------
            // CHECK PROFIT ON TRACKED TOKENS
            // -------------------------------------------------------
            if last_profit_check.elapsed() >= Duration::from_secs(PROFIT_CHECK_INTERVAL_SECS) {
                println!("\n💰 Checking profit on tracked tokens...");
                self.check_profits().await;
                last_profit_check = Instant::now();
            }

            // -------------------------------------------------------
            // PRUNE seen_tokens mid-run
            // Without this, seen_tokens grows unbounded and after ~6h every
            // token in DexScreener's ≤6h window is already marked seen → 0 new
            // tokens per scan. Pruning every ~5 minutes keeps the set fresh.
            // -------------------------------------------------------
            {
                let cutoff = Utc::now() - chrono::Duration::hours(8);
                self.data.seen_tokens.retain(|_, ts| {
                    ts.parse::<DateTime<Utc>>()
                        .map(|t| t > cutoff)
                        .unwrap_or(false)
                });
            }

            // -------------------------------------------------------
            // SAVE DATA
            // -------------------------------------------------------
            let save_interval = chrono::Duration::minutes(SAVE_INTERVAL_MINS);
            if Utc::now().signed_duration_since(self.last_save) >= save_interval {
                if let Err(e) = self.save() {
                    println!("❌ Failed to save: {e}");
                }
                self.last_save = Utc::now();
            }

            // Restore normal thresholds if off-peak overrides were applied this cycle
            self.deactivate_off_peak();

            // Wait for next scan — poll Telegram every 3 seconds so commands respond fast
            let mut waited = 0u64;
            while waited < SCAN_INTERVAL_SECS {
                let tick = 3u64.min(SCAN_INTERVAL_SECS - waited);
                sleep(Duration::from_secs(tick)).await;
                self.poll_telegram_commands().await;
                waited += tick;
            }
        }
    }
}

// ============================================================
// HELPERS
// ============================================================

fn format_usd(amount: f64) -> String {
    if amount >= 1_000_000.0 {
        format!("${:.2}M", amount / 1_000_000.0)
    } else if amount >= 1_000.0 {
        format!("${:.1}K", amount / 1_000.0)
    } else {
        format!("${amount:.2}")
    }
}

// ============================================================
// ENTRY POINT
// ============================================================

#[tokio::main]
async fn main() {
    // Load config.env first (highest priority — user config), then fall back to .env
    let _ = dotenvy::from_filename_override("config.env");
    let _ = dotenvy::dotenv_override();

    // --------------------------------------------------------
    // Check CLI arguments
    // --------------------------------------------------------
    let args: Vec<String> = std::env::args().collect();
    let is_backtest = args.iter().any(|a| a == "--backtest" || a == "-b");
    let is_compare  = args.iter().any(|a| a == "--compare" || a == "-c");
    let is_help     = args.iter().any(|a| a == "--help" || a == "-h");

    if is_help {
        println!("Basol Bot v3.0 — Solana Memecoin Auto Trader");
        println!();
        println!("USAGE:");
        println!("  cargo run                 → Run main bot (scan & analyze)");
        println!("  cargo run -- --backtest   → Backtest current strategy");
        println!("  cargo run -- --compare    → Compare 4 configuration presets at once");
        println!("  cargo run -- --help       → Show this help");
        println!();
        println!("ENVIRONMENT VARIABLES (core):");
        println!("  TRADING_ENABLED=false        Master switch for live trading");
        println!("  PAPER_TRADING_ENABLED=false  Simulated trading without real money");
        println!("  PAPER_BALANCE_SOL=0.1        Virtual paper trading balance");
        println!("  MIN_SCORE_TO_BUY=65.0        Minimum token score to trigger buy");
        println!("  MAX_POSITION_SOL=0.03        Max SOL per position");
        println!();
        println!("ENVIRONMENT VARIABLES (protection — v3.0):");
        println!("  CIRCUIT_BREAKER_LOSSES=4     Pause buying after N consecutive losses");
        println!("  CIRCUIT_BREAKER_PAUSE_HOURS=1  Pause duration in hours");
        println!("  PEAK_HOURS_ONLY=false        Only buy during 13-17 & 20-00 UTC");
        println!("  MOMENTUM_MAX_PCT=30.0        Skip tokens already up >N% in 1h");
        println!();
        println!("ENVIRONMENT VARIABLES (off-peak trading — stricter filters outside peak hours):");
        println!("  OFF_PEAK_TRADING_ENABLED=true   Allow trading outside peak hours with strict filters");
        println!("  OFF_PEAK_MIN_SCORE=75.0         Minimum score required off-peak (vs normal peak score)");
        println!("  OFF_PEAK_MIN_LIQUIDITY=15000.0  Minimum liquidity USD off-peak");
        println!("  OFF_PEAK_MAX_POSITIONS=1        Max open positions allowed off-peak");
        println!("  OFF_PEAK_MOMENTUM_MAX_PCT=10.0  Stricter momentum filter off-peak");
        println!();
        println!("TELEGRAM COMMANDS: /status /pause /resume /trades /score /blacklist");
        println!();
        println!("BACKTEST VARS:");
        println!("  BACKTEST_TOKEN_LIMIT=150     Number of tokens for backtest/compare");
        println!("  BACKTEST_MIN_AGE_HOURS=6     Minimum token age (hours)");
        println!("  BACKTEST_MAX_AGE_HOURS=72    Maximum token age (hours)");
        println!("  BACKTEST_MIN_LIQUIDITY=5000  Minimum liquidity in USD");
        return;
    }

    if is_backtest {
        run_backtest_mode().await;
        return;
    }

    if is_compare {
        run_compare_mode().await;
        return;
    }

    // --------------------------------------------------------
    // Normal mode: Scanner bot
    // --------------------------------------------------------
    println!("══════════════════════════════════════════");
    println!("   Basol Bot v3.0 — Solana Auto Trader    ");
    println!("══════════════════════════════════════════");
    println!("Config from environment:");
    println!("  TRADING_ENABLED       = {}", std::env::var("TRADING_ENABLED").unwrap_or("false".to_string()));
    println!("  PAPER_TRADING_ENABLED = {}", std::env::var("PAPER_TRADING_ENABLED").unwrap_or("false".to_string()));
    println!("  MAX_POSITION_SOL      = {}", std::env::var("MAX_POSITION_SOL").unwrap_or("0.03".to_string()));
    println!("  TAKE_PROFIT_PERCENT   = {}", std::env::var("TAKE_PROFIT_PERCENT").unwrap_or("25.0".to_string()));
    println!("  STOP_LOSS_PERCENT     = {}", std::env::var("STOP_LOSS_PERCENT").unwrap_or("5.0".to_string()));
    println!("  WALLET_PRIVATE_KEY    = {}", if std::env::var("WALLET_PRIVATE_KEY").is_ok() { "✅ SET" } else { "❌ NOT SET" });
    println!("── Protection (v3.0) ──────────────────────");
    println!("  CIRCUIT_BREAKER_LOSSES= {}", std::env::var("CIRCUIT_BREAKER_LOSSES").unwrap_or("4".to_string()));
    println!("  CIRCUIT_BREAKER_PAUSE = {}h", std::env::var("CIRCUIT_BREAKER_PAUSE_HOURS").unwrap_or("1".to_string()));
    println!("  PEAK_HOURS_ONLY       = {}", std::env::var("PEAK_HOURS_ONLY").unwrap_or("false".to_string()));
    println!("  MOMENTUM_MAX_PCT      = {}%", std::env::var("MOMENTUM_MAX_PCT").unwrap_or("30.0".to_string()));
    println!("══════════════════════════════════════════");
    println!();
    println!("  Tip: Run 'cargo run -- --backtest' to backtest strategy");
    println!("       Run 'cargo run -- --help' for full help");
    println!("══════════════════════════════════════════\n");

    let mut bot = SolanaBot::new();
    bot.run().await;
}

// ============================================================
// BACKTEST MODE
// ============================================================

// ============================================================
// COMPARE MODE - Compare multiple strategy presets
// ============================================================

async fn run_compare_mode() {
    println!("══════════════════════════════════════════════════");
    println!("   COMPARE MODE - 4 Strategy Preset Comparison    ");
    println!("══════════════════════════════════════════════════\n");

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .use_rustls_tls()
        .build()
    {
        Ok(c) => c,
        Err(e) => { eprintln!("[ERROR] Failed to build HTTP client: {e}"); return; }
    };

    let base_config = strategy::TradingConfig::from_env();
    let bt_config   = backtest::BacktestConfig::from_env();
    let tg_token    = std::env::var("TELEGRAM_BOT_TOKEN").ok();
    let tg_chat     = std::env::var("TELEGRAM_CHAT_ID").ok();

    match backtest::run_backtest_compare(&client, &base_config, &bt_config, None).await {
        Ok(result) => {
            // 1. Print table to console
            backtest::print_compare_table(&result);

            // 2. Save to JSON file
            if let Err(e) = backtest::save_compare_result(&result) {
                eprintln!("[COMPARE] Failed to save result: {e}");
            }

            // 3. Send to Telegram
            if let (Some(token), Some(chat)) = (tg_token, tg_chat) {
                let msg = backtest::format_compare_telegram(&result);
                println!("[COMPARE] Sending results to Telegram...");
                let tg_url = format!("https://api.telegram.org/bot{token}/sendMessage");
                let payload = serde_json::json!({
                    "chat_id": chat,
                    "text": msg,
                    "parse_mode": "Markdown"
                });
                match client.post(&tg_url).json(&payload).send().await {
                    Ok(r) if r.status().is_success() => println!("[COMPARE] ✅ Report sent to Telegram"),
                    Ok(r) => eprintln!("[COMPARE] Telegram error: {}", r.status()),
                    Err(e) => eprintln!("[COMPARE] Failed to send to Telegram: {e}"),
                }
            } else {
                println!("[COMPARE] Telegram not configured — results only in console and JSON file");
            }

            // 4. Best configuration suggestion
            println!();
            println!("💡 TIP: To apply the best strategy to the main bot, edit config.env:");
            if let Some(winner) = result.scenarios.first() {
                println!("   Strategy \"{}\" → {}", winner.name, winner.label);
                println!("   See compare_*.json for full details.");
            }
        }
        Err(e) => eprintln!("[COMPARE] ❌ Error: {e}"),
    }
}

async fn run_backtest_mode() {
    println!("══════════════════════════════════════════");
    println!("   BACKTESTING ENGINE - Solana Token Bot  ");
    println!("══════════════════════════════════════════\n");

    // Build HTTP client
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .use_rustls_tls()
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[ERROR] Failed to build HTTP client: {e}");
            return;
        }
    };

    let trading_config = strategy::TradingConfig::from_env();
    let bt_config      = backtest::BacktestConfig::from_env();

    // Load Telegram config (optional)
    let tg_token = std::env::var("TELEGRAM_BOT_TOKEN").ok();
    let tg_chat  = std::env::var("TELEGRAM_CHAT_ID").ok();

    match backtest::run_backtest(&client, &trading_config, &bt_config).await {
        Ok(result) => {
            // 1. Print report to console
            backtest::print_backtest_report(&result);

            // 2. Save to JSON file
            if let Err(e) = backtest::save_backtest_result(&result) {
                eprintln!("[BACKTEST] Failed to save result: {e}");
            }

            // 3. Send to Telegram if configured
            if let (Some(token), Some(chat)) = (tg_token, tg_chat) {
                let msg = backtest::format_backtest_telegram(&result);
                println!("[BACKTEST] Sending report to Telegram...");
                let tg_url = format!("https://api.telegram.org/bot{token}/sendMessage");
                let payload = serde_json::json!({
                    "chat_id": chat,
                    "text": msg,
                    "parse_mode": "Markdown"
                });
                match client.post(&tg_url).json(&payload).send().await {
                    Ok(r) if r.status().is_success() => println!("[BACKTEST] ✅ Report sent to Telegram"),
                    Ok(r) => eprintln!("[BACKTEST] Telegram error: {}", r.status()),
                    Err(e) => eprintln!("[BACKTEST] Failed to send to Telegram: {e}"),
                }
            } else {
                println!("[BACKTEST] Telegram not configured — report only in console and JSON file");
            }
        }
        Err(e) => {
            eprintln!("[BACKTEST] ❌ Error: {e}");
        }
    }
}
