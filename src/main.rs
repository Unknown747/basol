// ============================================================
// بوت تحليل Solana المتقدم - مع ميزة Auto Buy/Auto Sell
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
use backtest::{
    BacktestConfig, CompareResult,
    run_backtest, run_backtest_compare,
    print_backtest_report, print_compare_table,
    format_backtest_telegram, format_compare_telegram,
    save_backtest_result, save_compare_result,
};

use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::time::sleep;
use serde::{Deserialize, Serialize};
use reqwest::Client;
use chrono::{DateTime, Utc};
use std::fs;

// ============================================================
// CONFIG - ضع مفاتيحك هنا / Konfigurasi Bot
// ============================================================
const SCAN_INTERVAL_SECS: u64          = 30;
const PROFIT_CHECK_INTERVAL_SECS: u64  = 300;
const SELL_CHECK_INTERVAL_SECS: u64    = 60;    // Cek posisi setiap 60 detik
const SOL_PRICE_UPDATE_INTERVAL_SECS: u64 = 300; // Refresh harga SOL setiap 5 menit
const SAVE_INTERVAL_MINS: i64     = 10;
const MAX_TOKEN_AGE_HOURS: i64    = 6;
const MIN_SCORE_NEW_TOKEN: f64    = 80.0;
const MIN_SCORE_OLDER_TOKEN: f64  = 85.0;
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

#[derive(Debug, Deserialize)]
struct HeliusTokenInfo {
    #[serde(rename = "mintAuthority")]
    mint_authority: Option<String>,
    #[serde(rename = "freezeAuthority")]
    freeze_authority: Option<String>,
    decimals: Option<u8>,
    supply: Option<String>,
}

#[derive(Debug, Deserialize)]
struct HeliusTokenHolder {
    address: String,
    amount: String,
}

#[derive(Debug, Deserialize)]
struct HeliusTransaction {
    signature: String,
    timestamp: i64,
    #[serde(rename = "feePayer")]
    fee_payer: Option<String>,
    #[serde(rename = "tokenTransfers")]
    token_transfers: Option<Vec<TokenTransfer>>,
}

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

#[derive(Debug, Clone)]
struct WhaleAnalysis {
    smart_wallets_entered: u32,
    accumulation_pattern: bool,
    distribution_signs: bool,
    cold_storage_transfers: u32,
    largest_single_buy_usd: f64,
    score: f64,
    signals: Vec<String>,
}

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

#[derive(Debug, Clone)]
struct ContractSecurity {
    mint_authority_revoked: bool,
    freeze_authority_revoked: bool,
    transfer_fee_percent: f64,
    honeypot_risk: bool,
    score: f64,
    flags: Vec<String>,
    signals: Vec<String>,
}

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
    fn to_arabic(&self) -> &str {
        match self {
            LifecyclePhase::Launch       => "⚡ مرحلة الإطلاق (خطيرة)",
            LifecyclePhase::FirstDip     => "📉 التصحيح الأول (فرصة)",
            LifecyclePhase::Accumulation => "🟢 مرحلة التجميع (مثالية)",
            LifecyclePhase::Breakout     => "🔥 مرحلة الاختراق (متأخر)",
            LifecyclePhase::Mature       => "⬜ ناضج (متأخر جداً)",
        }
    }
}

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
            AlertLevel::Legendary => "👑 **فرصة أسطورية** 👑",
            AlertLevel::Golden    => "💎 **فرصة ذهبية** 💎",
            AlertLevel::Excellent => "🔥 **فرصة ممتازة** 🔥",
            AlertLevel::Normal    => "⭐ **فرصة عادية** ⭐",
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
// MAIN BOT STRUCT - dengan integrasi Trading
// ============================================================

struct SolanaBot {
    client: Client,
    data: BotPersistentData,
    dex_limiter: RateLimiter,
    helius_limiter: RateLimiter,
    tg_limiter: RateLimiter,
    last_save: DateTime<Utc>,
    is_paused: bool,

    // Config dari environment
    helius_api_key: String,
    telegram_token: String,
    telegram_chat_id: String,
    sol_price_usd: f64,
    last_price_update: Instant,

    // === FITUR BARU: Live Trading ===
    positions: HashMap<String, Position>,
    trading_config: TradingConfig,
    wallet: Option<WalletManager>,
    last_sell_check: Instant,

    // === FITUR BARU: Paper Trading ===
    paper_config: PaperConfig,
    paper_state: PaperTradingState,
    last_paper_report: Instant,
}

impl SolanaBot {
    fn new() -> Self {
        // Load .env jika ada
        let _ = dotenv::dotenv();

        // Baca konfigurasi wajib dari environment (panic dengan pesan jelas jika tidak ada)
        let helius_api_key = std::env::var("HELIUS_API_KEY")
            .expect("HELIUS_API_KEY wajib diset di .env (lihat .env.example)");
        let telegram_token = std::env::var("TELEGRAM_BOT_TOKEN")
            .expect("TELEGRAM_BOT_TOKEN wajib diset di .env (lihat .env.example)");
        let telegram_chat_id = std::env::var("TELEGRAM_CHAT_ID")
            .expect("TELEGRAM_CHAT_ID wajib diset di .env (lihat .env.example)");
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

        // Load wallet jika trading enabled
        let wallet = if trading_config.trading_enabled {
            match WalletManager::from_env() {
                Ok(w) => {
                    println!("[TRADING] Wallet berhasil diload: {}", w.public_key);
                    Some(w)
                }
                Err(e) => {
                    eprintln!("[TRADING] ⚠️ Gagal load wallet: {} - Trading dinonaktifkan", e);
                    None
                }
            }
        } else {
            println!("[TRADING] Trading dinonaktifkan (TRADING_ENABLED=false)");
            None
        };

        let trading_mode = if trading_config.trading_enabled && wallet.is_some() {
            "AKTIF"
        } else {
            "NON-AKTIF"
        };

        println!("[TRADING] Mode: {} | Max: {:.2} SOL | TP: {:.1}% | SL: {:.1}%",
            trading_mode,
            trading_config.max_position_sol,
            trading_config.take_profit_percent,
            trading_config.stop_loss_percent,
        );

        // Load paper trading config dan state
        let paper_config = PaperConfig::from_env();
        let paper_state = if paper_config.enabled {
            println!("[PAPER] Paper trading AKTIF | Virtual balance: {:.2} SOL", paper_config.virtual_balance_sol);
            load_paper_state(paper_config.virtual_balance_sol)
        } else {
            println!("[PAPER] Paper trading non-aktif (set PAPER_TRADING_ENABLED=true untuk mengaktifkan)");
            PaperTradingState::new(paper_config.virtual_balance_sol)
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
            },
            dex_limiter: RateLimiter::new(55, 60),
            helius_limiter: RateLimiter::new(40, 60),
            tg_limiter: RateLimiter::new(20, 60),
            last_save: Utc::now(),
            is_paused: false,
            helius_api_key,
            telegram_token,
            telegram_chat_id,
            sol_price_usd,
            // Set ke masa lalu agar fetch langsung dilakukan saat loop pertama
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
        }
    }

    fn initial_smart_wallets() -> Vec<String> {
        vec![]
    }

    // ============================================================
    // PERSISTENCE
    // ============================================================

    fn save(&self) -> Result<(), Box<dyn std::error::Error>> {
        // Sync positions ke data sebelum save
        let mut data_to_save = serde_json::to_value(&self.data)?;
        let pos_map: HashMap<String, PositionData> = self.positions.iter()
            .map(|(k, v)| (k.clone(), PositionData::from(v)))
            .collect();
        data_to_save["positions"] = serde_json::to_value(&pos_map)?;

        let json = serde_json::to_string_pretty(&data_to_save)?;
        fs::write("bot_data.json", json)?;
        println!("💾 تم حفظ البيانات - {} عملة مرصودة, {} متتبعة, {} posisi aktif",
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
                        // Load posisi aktif yang tersimpan
                        let saved_positions: HashMap<String, Position> = data.positions.iter()
                            .map(|(k, v)| (k.clone(), Position::from(v.clone())))
                            .collect();

                        self.data = data;
                        let cutoff = Utc::now() - chrono::Duration::days(30);
                        self.data.seen_tokens.retain(|_, ts| {
                            ts.parse::<DateTime<Utc>>()
                                .map(|t| t > cutoff)
                                .unwrap_or(false)
                        });
                        self.positions = saved_positions;
                        println!("📂 تم تحميل البيانات: {} عملة مرصودة, {} متتبعة, {} posisi aktif",
                            self.data.seen_tokens.len(),
                            self.data.tracked_tokens.len(),
                            self.positions.len());
                    }
                    Err(e) => println!("⚠️ خطأ في تحليل ملف البيانات: {}", e),
                }
            }
            Err(_) => println!("ℹ️ بدء جديد - لا توجد بيانات محفوظة"),
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
            return Err(format!("Telegram error: {}", err).into());
        }
        Ok(())
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
            println!("❌ خطأ في إرسال الصورة: {}", e);
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
                            text: "⏸ إيقاف مؤقت".to_string(),
                            url: None,
                            callback_data: Some("/pause".to_string()),
                        },
                        InlineButton {
                            text: "📈 الإحصاءات".to_string(),
                            url: None,
                            callback_data: Some("/stats".to_string()),
                        },
                    ],
                ],
            },
        };
        if let Err(e) = self.client.post(&url).json(&payload).send().await {
            println!("❌ خطأ في إرسال الرسالة مع الأزرار: {}", e);
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
        let url = format!("https://api.dexscreener.com/latest/dex/tokens/{}", address);
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

    async fn get_boosted_tokens(&mut self) -> Vec<String> {
        self.dex_limiter.wait_if_needed().await;
        let url = "https://api.dexscreener.com/token-boosts/latest/v1";
        #[derive(Deserialize)]
        struct BoostToken {
            #[serde(rename = "tokenAddress")] address: String,
            #[serde(rename = "chainId")] chain: String
        }
        match self.client.get(url).send().await {
            Ok(resp) if resp.status().is_success() => {
                resp.json::<Vec<BoostToken>>().await.unwrap_or_default()
                    .into_iter()
                    .filter(|t| t.chain == "solana")
                    .map(|t| t.address)
                    .collect()
            }
            _ => vec![],
        }
    }

    // ============================================================
    // HELIUS API
    // ============================================================

    async fn get_token_metadata(&mut self, address: &str) -> Option<HeliusTokenInfo> {
        self.helius_limiter.wait_if_needed().await;
        let url = format!(
            "https://api.helius.xyz/v0/token-metadata?api-key={}", self.helius_api_key
        );
        let body = serde_json::json!({ "mintAccounts": [address] });
        match self.client.post(&url).json(&body).send().await {
            Ok(resp) if resp.status().is_success() => {
                let arr: Vec<serde_json::Value> = resp.json().await.ok()?;
                let item = arr.into_iter().next()?;
                let _on_chain = item.get("onChainMetadata")?
                    .get("metadata")?
                    .get("tokenStandard");
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
                Some(HeliusTokenInfo {
                    mint_authority: mint_auth,
                    freeze_authority: freeze_auth,
                    decimals: None,
                    supply: None,
                })
            }
            _ => None,
        }
    }

    async fn get_token_holders(&mut self, address: &str) -> Vec<HeliusTokenHolder> {
        self.helius_limiter.wait_if_needed().await;
        let url = format!(
            "https://api.helius.xyz/v1/token-holders?api-key={}&mint={}&limit=50",
            self.helius_api_key, address
        );
        match self.client.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => {
                resp.json::<Vec<HeliusTokenHolder>>().await.unwrap_or_default()
            }
            _ => vec![],
        }
    }

    async fn get_token_transactions(&mut self, address: &str) -> Vec<HeliusTransaction> {
        self.helius_limiter.wait_if_needed().await;
        let url = format!(
            "https://api.helius.xyz/v0/addresses/{}/transactions?api-key={}&type=SWAP&limit=100",
            address, self.helius_api_key
        );
        match self.client.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => {
                resp.json::<Vec<HeliusTransaction>>().await.unwrap_or_default()
            }
            _ => vec![],
        }
    }

    // ============================================================
    // SOL PRICE - Auto refresh dari API publik
    // ============================================================

    /// Fetch harga SOL dari multiple sumber dengan fallback.
    /// Urutan: Jupiter Price API → Binance → CoinGecko → harga lama
    async fn fetch_sol_price(&self) -> f64 {
        // --- Sumber 1: Jupiter Price API v6 (paling akurat untuk Solana) ---
        if let Ok(resp) = self.client
            .get("https://price.jup.ag/v6/price?ids=SOL")
            .send()
            .await
        {
            if let Ok(data) = resp.json::<serde_json::Value>().await {
                if let Some(price) = data["data"]["SOL"]["price"].as_f64() {
                    if price > 0.0 {
                        println!("[SOL PRICE] Jupiter: ${:.2}", price);
                        return price;
                    }
                }
            }
        }

        // --- Sumber 2: Binance public ticker (no auth required) ---
        if let Ok(resp) = self.client
            .get("https://api.binance.com/api/v3/ticker/price?symbol=SOLUSDT")
            .send()
            .await
        {
            if let Ok(data) = resp.json::<serde_json::Value>().await {
                if let Some(price_str) = data["price"].as_str() {
                    if let Ok(price) = price_str.parse::<f64>() {
                        if price > 0.0 {
                            println!("[SOL PRICE] Binance: ${:.2}", price);
                            return price;
                        }
                    }
                }
            }
        }

        // --- Sumber 3: CoinGecko free API ---
        if let Ok(resp) = self.client
            .get("https://api.coingecko.com/api/v3/simple/price?ids=solana&vs_currencies=usd")
            .send()
            .await
        {
            if let Ok(data) = resp.json::<serde_json::Value>().await {
                if let Some(price) = data["solana"]["usd"].as_f64() {
                    if price > 0.0 {
                        println!("[SOL PRICE] CoinGecko: ${:.2}", price);
                        return price;
                    }
                }
            }
        }

        // --- Fallback: pakai harga terakhir yang tersimpan ---
        println!("[SOL PRICE] Semua sumber gagal, pakai harga lama: ${:.2}", self.sol_price_usd);
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
            sorted.sort_by(|a, b| b.partial_cmp(a).unwrap());
            sorted[..10].iter().sum()
        } else {
            amounts.iter().sum()
        };

        let top10_pct = if total_supply > 0.0 { top10_sum / total_supply * 100.0 } else { 0.0 };

        // Gini coefficient
        let gini = if amounts.len() > 1 {
            let mut sorted = amounts.clone();
            sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
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

        // Developer sold %
        let dev_sold = if !txns.is_empty() { 15.0 } else { 0.0 };

        // Flags
        if top10_pct > 50.0 {
            flags.push(format!("🔴 Top 10 holders: {:.1}% konsentrasi tinggi", top10_pct));
        } else {
            signals.push(format!("✅ Distribusi holder sehat: Top 10 = {:.1}%", top10_pct));
        }
        if sniper_count > 10 {
            flags.push(format!("🔴 {} sniper terdeteksi", sniper_count));
        }
        if bundled {
            flags.push("🔴 Bundled wallets terdeteksi".to_string());
        }
        if total_holders > 500 {
            signals.push(format!("✅ {} holder aktif", total_holders));
        }

        // Score (max 25)
        let mut score = 0.0f64;
        if top10_pct < 30.0 { score += 10.0; }
        else if top10_pct < 50.0 { score += 5.0; }
        if sniper_count < 5 { score += 5.0; }
        if !bundled { score += 5.0; }
        if total_holders > 200 { score += 5.0; }
        score = score.max(0.0).min(25.0);

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

        let lp_burned = total_usd > 50_000.0;
        let lp_locked = total_usd > 20_000.0;
        let independent = pairs.len() as u32;

        let price_impact_5k  = if total_usd > 0.0 { 5_000.0 / total_usd * 100.0 } else { 100.0 };
        let price_impact_10k = if total_usd > 0.0 { 10_000.0 / total_usd * 100.0 } else { 100.0 };
        let price_impact_25k = if total_usd > 0.0 { 25_000.0 / total_usd * 100.0 } else { 100.0 };

        let mut flags = vec![];
        let mut signals = vec![];

        if total_usd < 10_000.0 {
            flags.push(format!("🔴 Likuiditas rendah: ${:.0}", total_usd));
        } else if total_usd > 100_000.0 {
            signals.push(format!("✅ Likuiditas kuat: ${:.0}", total_usd));
        } else {
            signals.push(format!("✅ Likuiditas cukup: ${:.0}", total_usd));
        }
        if lp_burned { signals.push("✅ LP Burned/Locked terdeteksi".to_string()); }

        let mut score = 0.0f64;
        if total_usd > 100_000.0 { score += 10.0; }
        else if total_usd > 50_000.0 { score += 7.0; }
        else if total_usd > 20_000.0 { score += 5.0; }
        else if total_usd > 10_000.0 { score += 3.0; }
        if lp_burned { score += 5.0; }
        if independent > 2 { score += 5.0; }
        score = score.max(0.0).min(20.0);

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
        let mut cold_storage_transfers = 0u32;
        let mut has_distribution = false;
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

        let largest_buy = buy_amounts.iter().cloned().fold(0.0_f64, f64::max);
        let accumulation = buy_amounts.len() > sell_amounts.len() * 2;
        has_distribution = sell_amounts.len() > buy_amounts.len();

        if smart_entered >= 3 {
            signals.push(format!("🐋 {} محافظ ذكية دخلت!", smart_entered));
        }
        if accumulation && !has_distribution {
            signals.push("📈 نمط تجميع واضح بدون توزيع".to_string());
        }
        if cold_storage_transfers >= 2 {
            signals.push(format!("🏦 {} تحويل لتخزين بارد", cold_storage_transfers));
        }

        let mut score = 0.0f64;
        if smart_entered >= 3 { score += 12.0; }
        else if smart_entered >= 1 { score += 6.0; }
        if accumulation && !has_distribution { score += 5.0; }
        if cold_storage_transfers >= 2 { score += 3.0; }
        if has_distribution { score -= 5.0; }
        score = score.max(0.0).min(20.0);

        WhaleAnalysis {
            smart_wallets_entered: smart_entered,
            accumulation_pattern: accumulation,
            distribution_signs: has_distribution,
            cold_storage_transfers,
            largest_single_buy_usd: largest_buy,
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
            Some("🚀 Bull Flag - زخم قوي".to_string())
        } else if m5 > 5.0 && h1 > 20.0 && h6 > 50.0 {
            Some("📈 Ascending Triangle".to_string())
        } else if h1 < -10.0 && h6 > 20.0 {
            Some("🔄 Wyckoff - تجميع محتمل".to_string())
        } else {
            None
        };

        let mut signals = vec![];
        if m5 > 20.0 { signals.push(format!("⚡ {:.1}% خلال 5 دقائق!", m5)); }
        if h1 > 50.0 { signals.push(format!("🚀 {:.1}% خلال ساعة!", h1)); }
        if buy_ratio > 0.7 { signals.push(format!("📈 ضغط شراء: {:.0}% شراء", buy_ratio * 100.0)); }
        if total_txns > 100 { signals.push(format!("🔥 {} معاملة/ساعة", total_txns)); }
        if let Some(p) = &pattern { signals.push(p.clone()); }
        if vol24 > 500_000.0 { signals.push(format!("📊 حجم تداول: ${:.0}", vol24)); }

        let mut score = 0.0f64;
        if m5 > 20.0 { score += 3.0; } else if m5 > 10.0 { score += 1.5; }
        if h1 > 50.0 { score += 3.0; } else if h1 > 20.0 { score += 1.5; }
        if buy_ratio > 0.7 { score += 2.0; }
        if total_txns > 100 { score += 2.0; }
        score = score.max(0.0).min(10.0);

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
        let mut flags = vec![];
        let mut signals = vec![];

        let mint_revoked = meta.as_ref()
            .map(|m| m.mint_authority.is_none())
            .unwrap_or(false);
        let freeze_revoked = meta.as_ref()
            .map(|m| m.freeze_authority.is_none())
            .unwrap_or(false);

        if !mint_revoked {
            flags.push("🔴 صلاحية Mint غير ملغية - خطر سك عملات جديدة".to_string());
        } else {
            signals.push("✅ صلاحية Mint ملغية".to_string());
        }
        if !freeze_revoked {
            flags.push("⚠️ صلاحية Freeze نشطة - يمكن تجميد المحافظ".to_string());
        } else {
            signals.push("✅ صلاحية Freeze ملغية".to_string());
        }

        let mut score = 0.0f64;
        if mint_revoked { score += 6.0; }
        if freeze_revoked { score += 4.0; }

        ContractSecurity {
            mint_authority_revoked: mint_revoked,
            freeze_authority_revoked: freeze_revoked,
            transfer_fee_percent: 0.0,
            honeypot_risk: false,
            score,
            flags,
            signals,
        }
    }

    fn analyze_social(&self, token: &DexToken) -> SocialAnalysis {
        let desc = token.description.as_deref().unwrap_or("");
        let has_twitter = desc.contains("twitter") || desc.contains("x.com") || desc.contains("t.me");
        let has_telegram = desc.contains("t.me") || desc.contains("telegram");
        let social_hype = if has_twitter && has_telegram { 70.0 }
            else if has_twitter || has_telegram { 40.0 }
            else { 10.0 };

        let mut signals = vec![];
        if has_twitter { signals.push("🐦 تويتر موجود".to_string()); }
        if has_telegram { signals.push("📱 تيليجرام موجود".to_string()); }

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
        let pairs = self.get_pairs(addr).await;
        if pairs.is_empty() { return None; }

        let holder = self.analyze_holders(addr, &pairs).await;
        if holder.bundled_wallets_detected {
            println!("  ⏭ تخطي - محافظ مجمعة مكتشفة");
            return None;
        }

        let liquidity = self.analyze_liquidity(&pairs);
        if liquidity.total_usd < 5000.0 {
            println!("  ⏭ تخطي - سيولة منخفضة جداً");
            return None;
        }

        let whale     = self.analyze_whales(addr).await;
        let technical = self.analyze_technicals(&pairs);
        let security  = self.analyze_contract_security(addr).await;

        if !security.mint_authority_revoked {
            println!("  ⏭ تخطي - صلاحية Mint غير ملغية");
            return None;
        }

        let social    = self.analyze_social(token);
        let lifecycle = self.analyze_lifecycle(&pairs);

        let total = holder.score + liquidity.score + whale.score
            + technical.score + security.score + social.score;
        let total = (total + lifecycle.entry_timing_score * 0.5).min(100.0);

        let min_score = if lifecycle.age_minutes < 360 {
            MIN_SCORE_NEW_TOKEN
        } else {
            MIN_SCORE_OLDER_TOKEN
        };

        if total < min_score {
            println!("  ⚪ نقاط غير كافية: {:.1} < {:.1}", total, min_score);
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

        m.push_str("📊 **لوحة التحليل:**\n");
        m.push_str(&format!("⭐ الإجمالي: **{:.1}/100**\n", a.total_score));
        m.push_str(&format!("👥 الحاملون: **{:.1}/25**\n", a.holder_analysis.score));
        m.push_str(&format!("🌊 السيولة: **{:.1}/20**\n", a.liquidity_analysis.score));
        m.push_str(&format!("🐋 الحيتان: **{:.1}/20**\n", a.whale_analysis.score));
        m.push_str(&format!("📈 التقني: **{:.1}/10**\n", a.technical_analysis.score));
        m.push_str(&format!("🛡️ الأمان: **{:.1}/10**\n", a.contract_security.score));
        m.push_str(&format!("🌐 الاجتماعي: **{:.1}/15**\n\n", a.social_analysis.score));

        m.push_str("💰 **البيانات المالية:**\n");
        if let Some(price) = a.price_usd {
            m.push_str(&format!("💵 السعر: **${:.8}**\n", price));
        }
        if let Some(mc) = a.market_cap {
            m.push_str(&format!("🏛️ Market Cap: **{}**\n", format_usd(mc)));
        }
        m.push_str(&format!("🌊 السيولة: **{}**\n", format_usd(a.liquidity_analysis.total_usd)));
        m.push_str(&format!("📊 الحجم 24h: **{}**\n\n", format_usd(a.technical_analysis.volume_24h)));

        m.push_str("⏰ **توقيت الدخول:**\n");
        m.push_str(&format!("{}\n", a.lifecycle.phase.to_arabic()));
        m.push_str(&format!("⏱️ عمر العملة: **{} دقيقة**\n", a.lifecycle.age_minutes));
        m.push_str(&format!("📐 R/R Ratio: **1:{:.0}**\n\n", a.lifecycle.risk_reward_ratio));

        m.push_str(&format!("🎯 **المضاعف المحتمل: {}**\n\n", a.potential_multiplier));

        m.push_str("✅ **أهم الإشارات:**\n");
        for signal in &a.top_signals {
            m.push_str(&format!("▫️ {}\n", signal));
        }

        if !a.all_red_flags.is_empty() {
            m.push_str("\n⚠️ **تحذيرات:**\n");
            for flag in a.all_red_flags.iter().take(3) {
                m.push_str(&format!("▪️ {}\n", flag));
            }
        }

        // Tambahkan status auto buy jika trading enabled
        if self.trading_config.trading_enabled {
            m.push_str(&format!("\n🤖 **Auto Buy:** {} SOL",
                if a.total_score >= self.trading_config.min_score_to_buy {
                    format!("Akan beli {:.3} SOL", ((a.total_score - 75.0) / 25.0).max(0.0).min(1.0) * self.trading_config.max_position_sol)
                } else {
                    "Tidak memenuhi syarat".to_string()
                }
            ));
        }

        m.push_str("\n═══════════════════════════════\n");
        m.push_str("⚠️ **هذا تحليل آلي وليس نصيحة مالية**\n");
        m.push_str("🔍 قم ببحثك الخاص قبل الاستثمار\n");
        m
    }

    fn format_status(&self) -> String {
        let stats = &self.data.performance_stats;
        let pos_summary = if self.positions.is_empty() {
            "Tidak ada posisi aktif".to_string()
        } else {
            self.positions.values()
                .map(|p| format!("{} ({:.1}%)", p.symbol,
                    p.profit_percent(p.highest_price)))
                .collect::<Vec<_>>()
                .join(", ")
        };

        format!(
            "📊 **حالة البوت**\n═══════════════════════════════\n\
            🔍 العملات المرصودة: **{}**\n\
            📌 العملات المتتبعة: **{}**\n\
            📢 تنبيهات اليوم: **{}/{}**\n\
            🏆 إجمالي التنبيهات: **{}**\n\
            💹 عملات حققت 2x: **{}**\n\
            💹 عملات حققت 5x: **{}**\n\
            💹 عملات حققت 10x: **{}**\n\
            🥇 أفضل عملة: **{}** ({:.1}%)\n\
            🤖 حالة البوت: **{}**\n\n\
            💼 **Trading:**\n\
            🔄 Mode: **{}**\n\
            📊 Posisi Aktif: **{}**\n\
            💰 Total Buy: **{}** | Total Sell: **{}**\n\
            📈 Total Profit: **{:.4} SOL** | Loss: **{:.4} SOL**\n\
            📋 Posisi: {}",
            self.data.seen_tokens.len(),
            self.data.tracked_tokens.len(),
            self.data.daily_alert_count, MAX_DAILY_ALERTS,
            stats.total_alerts_sent,
            stats.tokens_reached_2x,
            stats.tokens_reached_5x,
            stats.tokens_reached_10x,
            stats.best_token_symbol, stats.best_token_gain_percent,
            if self.is_paused { "⏸ متوقف مؤقتاً" } else { "▶️ يعمل" },
            if self.trading_config.trading_enabled && self.wallet.is_some() { "🟢 AKTIF" } else { "🔴 NON-AKTIF" },
            self.positions.len(),
            stats.total_buys, stats.total_sells,
            stats.total_profit_sol, stats.total_loss_sol,
            pos_summary,
        )
    }

    // ============================================================
    // AUTO BUY - Eksekusi pembelian setelah analisis
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

        let decision = evaluate_buy_signal(&signal, &self.trading_config, &self.positions);
        log_buy_decision(&signal, &decision);

        if let BuyDecision::Buy { amount_sol, reason } = decision {
            println!("[AUTO BUY] Eksekusi buy {} - {:.4} SOL | {}", signal.symbol, amount_sol, reason);

            // Cek saldo wallet
            let balance = match self.wallet.as_ref().unwrap().get_sol_balance().await {
                Ok(b) => b,
                Err(e) => {
                    println!("[AUTO BUY] Gagal cek saldo: {}", e);
                    return;
                }
            };

            if balance < amount_sol + 0.01 {
                println!("[AUTO BUY] Saldo tidak cukup: {:.4} SOL (butuh {:.4} SOL)", balance, amount_sol + 0.01);
                return;
            }

            // Eksekusi buy
            match self.wallet.as_ref().unwrap()
                .buy_token(&signal.token_address, amount_sol, self.trading_config.default_slippage)
                .await
            {
                Ok(signature) => {
                    println!("[AUTO BUY] ✅ SUKSES! TX: {}", signature);

                    // Estimasi token yang diterima (pakai price USD)
                    let sol_price_usd = self.sol_price_usd;
                    let token_amount = if signal.current_price_usd > 0.0 {
                        (amount_sol * sol_price_usd) / signal.current_price_usd
                    } else { 0.0 };

                    // Buat posisi baru
                    let position = Position::new(
                        signal.token_address.clone(),
                        signal.symbol.clone(),
                        signal.name.clone(),
                        signal.current_price_usd,
                        amount_sol,
                        token_amount,
                        signal.total_score,
                    );
                    self.positions.insert(signal.token_address.clone(), position);
                    self.data.performance_stats.total_buys += 1;

                    // Kirim notifikasi Telegram
                    let msg = format_buy_notification(
                        &signal.token_address,
                        &signal.symbol,
                        &signal.name,
                        amount_sol,
                        signal.current_price_usd,
                        signal.total_score,
                        &signature,
                    );
                    let _ = self.send_message(&msg).await;
                }
                Err(e) => {
                    println!("[AUTO BUY] ❌ GAGAL: {}", e);
                    let err_msg = format!(
                        "❌ **AUTO BUY GAGAL**\nToken: {} ({})\nError: {}\nSilakan cek log untuk detail.",
                        signal.name, signal.symbol, e
                    );
                    let _ = self.send_message(&err_msg).await;
                }
            }
        }
    }

    // ============================================================
    // AUTO SELL - Cek dan eksekusi penjualan posisi
    // ============================================================

    async fn check_and_sell_positions(&mut self) {
        if !self.trading_config.trading_enabled || self.wallet.is_none() {
            return;
        }
        if self.positions.is_empty() {
            return;
        }

        println!("[AUTO SELL] Mengecek {} posisi aktif...", self.positions.len());

        // Ambil harga current untuk semua posisi
        let mut prices: HashMap<String, f64> = HashMap::new();
        let addresses: Vec<String> = self.positions.keys().cloned().collect();

        for addr in &addresses {
            self.dex_limiter.wait_if_needed().await;
            let url = format!("https://api.dexscreener.com/latest/dex/tokens/{}", addr);
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

        // Evaluasi semua posisi
        let decisions = evaluate_all_positions(
            &mut self.positions,
            &prices,
            &self.trading_config,
        );

        // Eksekusi sell untuk yang triggered
        for (addr, decision) in decisions {
            if let SellDecision::Sell { percentage, trigger } = decision {
                let (symbol, name, buy_price, amount_sol) = {
                    let pos = match self.positions.get(&addr) {
                        Some(p) => p,
                        None => continue,
                    };
                    (pos.symbol.clone(), pos.name.clone(), pos.buy_price_usd, pos.amount_in_sol)
                };

                let current_price = prices.get(&addr).copied().unwrap_or(0.0);

                println!(
                    "[AUTO SELL] Eksekusi sell {} - {:.1}% | {}",
                    symbol, percentage, trigger.description()
                );

                match self.wallet.as_ref().unwrap()
                    .sell_token(&addr, percentage, self.trading_config.default_slippage)
                    .await
                {
                    Ok(signature) => {
                        println!("[AUTO SELL] ✅ SUKSES! TX: {}", signature);

                        let profit_pct = if buy_price > 0.0 {
                            (current_price - buy_price) / buy_price * 100.0
                        } else { 0.0 };
                        let profit_sol = amount_sol * profit_pct / 100.0;

                        // Update stats
                        self.data.performance_stats.total_sells += 1;
                        if profit_sol >= 0.0 {
                            self.data.performance_stats.total_profit_sol += profit_sol;
                        } else {
                            self.data.performance_stats.total_loss_sol += profit_sol.abs();
                        }

                        // Kirim notifikasi
                        if let Some(pos) = self.positions.get(&addr) {
                            let msg = format_sell_notification(pos, current_price, &trigger, &signature);
                            let _ = self.send_message(&msg).await;
                        }

                        // Hapus posisi jika jual 100%
                        if percentage >= 100.0 {
                            self.positions.remove(&addr);
                            println!("[AUTO SELL] Posisi {} dihapus dari daftar aktif", symbol);
                        }
                    }
                    Err(e) => {
                        println!("[AUTO SELL] ❌ GAGAL sell {}: {}", symbol, e);
                        let err_msg = format!(
                            "❌ **AUTO SELL GAGAL**\n{}\nTrigger: {}\nError: {}",
                            symbol, trigger.description(), e
                        );
                        let _ = self.send_message(&err_msg).await;
                    }
                }

                // Delay antara sell
                sleep(Duration::from_secs(2)).await;
            }
        }
    }

    // ============================================================
    // PAPER TRADING - Simulasi Buy
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

        // Pakai logika sama dengan live trading, tapi evaluasi terhadap posisi paper
        let paper_positions_snapshot: HashMap<String, Position> = self.paper_state.positions.iter()
            .map(|(k, v)| {
                (k.clone(), Position::new(
                    v.token_address.clone(), v.symbol.clone(), v.name.clone(),
                    v.buy_price_usd, v.amount_sol, v.token_amount, v.score_at_entry,
                ))
            })
            .collect();

        let config = TradingConfig {
            trading_enabled: true,
            max_position_sol: self.paper_config.max_position_sol,
            take_profit_percent: self.paper_config.take_profit_percent,
            stop_loss_percent: self.paper_config.stop_loss_percent,
            trailing_start_percent: self.paper_config.trailing_start_percent,
            trailing_distance_percent: self.paper_config.trailing_distance_percent,
            min_score_to_buy: self.paper_config.min_score_to_buy,
            min_liquidity_usd: self.paper_config.min_liquidity_usd,
            default_slippage: 1.0,
            max_positions: self.paper_config.max_positions,
        };

        let decision = evaluate_buy_signal(&signal, &config, &paper_positions_snapshot);
        log_buy_decision(&signal, &decision);

        if let BuyDecision::Buy { amount_sol, .. } = decision {
            let price = signal.current_price_usd;
            let sol_price_usd = self.sol_price_usd;
            let token_amount = if price > 0.0 { (amount_sol * sol_price_usd) / price } else { 0.0 };

            match self.paper_state.execute_buy(
                signal.token_address.clone(),
                signal.symbol.clone(),
                signal.name.clone(),
                price,
                amount_sol,
                token_amount,
                signal.total_score,
                signal.liquidity_usd,
            ) {
                Ok(_sig) => {
                    let msg = format_paper_buy_notification(
                        &signal.symbol,
                        &signal.name,
                        &signal.token_address,
                        amount_sol,
                        price,
                        signal.total_score,
                        self.paper_state.current_balance_sol,
                        self.paper_state.positions.len(),
                    );
                    let _ = self.send_message(&msg).await;

                    // Auto-save setelah setiap buy
                    if let Err(e) = save_paper_state(&self.paper_state) {
                        println!("[PAPER] Gagal save state: {}", e);
                    }
                }
                Err(e) => println!("[PAPER BUY] Skip: {}", e),
            }
        }
    }

    // ============================================================
    // PAPER TRADING - Simulasi Sell (cek semua posisi)
    // ============================================================

    async fn check_and_paper_sell(&mut self) {
        if !self.paper_config.enabled || self.paper_state.positions.is_empty() {
            return;
        }

        println!("[PAPER SELL] Mengecek {} posisi paper...", self.paper_state.positions.len());

        // Ambil harga semua posisi terbuka
        let mut prices: HashMap<String, f64> = HashMap::new();
        let addrs: Vec<String> = self.paper_state.positions.keys().cloned().collect();

        for addr in &addrs {
            self.dex_limiter.wait_if_needed().await;
            let url = format!("https://api.dexscreener.com/latest/dex/tokens/{}", addr);
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

        // Evaluasi TP/SL/Trailing
        let to_sell = self.paper_state.evaluate_positions(
            &prices,
            self.paper_config.take_profit_percent,
            self.paper_config.stop_loss_percent,
            self.paper_config.trailing_start_percent,
            self.paper_config.trailing_distance_percent,
        );

        // Eksekusi sell paper
        for (addr, reason, sell_price) in to_sell {
            match self.paper_state.execute_sell(&addr, sell_price, 100.0, reason) {
                Ok(trade) => {
                    let balance = self.paper_state.current_balance_sol;
                    let msg = format_paper_sell_notification(&trade, balance);
                    let _ = self.send_message(&msg).await;

                    // Save setelah sell
                    if let Err(e) = save_paper_state(&self.paper_state) {
                        println!("[PAPER] Gagal save state: {}", e);
                    }
                }
                Err(e) => println!("[PAPER SELL] Error: {}", e),
            }
            sleep(Duration::from_secs(1)).await;
        }
    }

    // ============================================================
    // PAPER TRADING - Kirim laporan periodik
    // ============================================================

    async fn send_paper_report(&mut self, current_prices: &HashMap<String, f64>) {
        if !self.paper_config.enabled { return; }
        let report = format_paper_report(&self.paper_state, current_prices);
        println!("[PAPER] Mengirim laporan periodik...");
        let _ = self.send_message(&report).await;
    }

    // ============================================================
    // PROFIT TRACKING
    // ============================================================

    async fn check_profits(&mut self) {
        let addresses: Vec<String> = self.data.tracked_tokens.keys().cloned().collect();
        for addr in addresses {
            self.helius_limiter.wait_if_needed().await;
            let url = format!("https://api.dexscreener.com/latest/dex/tokens/{}", addr);
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
                    let token = self.data.tracked_tokens.get_mut(&addr).unwrap();
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
                        50   => ("🎉", "بداية موفقة"),
                        100  => ("🚀💎", "مضاعفة رأس المال"),
                        200  => ("🔥💰", "ربح استثنائي"),
                        500  => ("⭐🏆", "أداء أسطوري"),
                        1000 => ("👑💎", "عملة 10x الذهبية"),
                        _    => ("🌟🚀", "ربح خرافي"),
                    };
                    let hours = Utc::now()
                        .signed_duration_since(
                            self.data.tracked_tokens.get(&addr)
                                .map(|t| t.discovery_time)
                                .unwrap_or(Utc::now())
                        ).num_hours();
                    let msg = format!(
                        "{} **{}!** {}\n═══════════════════════════════\n\n\
                        💎 العملة: **{}** `({})`\n\
                        📈 الربح: **+{:.1}%**\n\
                        💰 سعر الاكتشاف: **${:.8}**\n\
                        💰 السعر الحالي: **${:.8}**\n\
                        ⏰ منذ الاكتشاف: **{} ساعة**\n\n\
                        🎉 **مبروك لجميع المتابعين!**\n\
                        🤖 تم اكتشاف هذه الفرصة بواسطة البوت",
                        emoji, title, emoji,
                        token_name, token_symbol,
                        pct, initial_price, price, hours
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
        println!("🚀 Bot Analisis Solana v2.0 dimulai...");
        println!("📊 Trading: {} | Max posisi: {:.2} SOL | TP: {:.1}% | SL: {:.1}%",
            if self.trading_config.trading_enabled && self.wallet.is_some() { "AKTIF" } else { "NON-AKTIF" },
            self.trading_config.max_position_sol,
            self.trading_config.take_profit_percent,
            self.trading_config.stop_loss_percent,
        );

        self.load();

        let startup_msg = format!(
            "🤖 **Bot Solana v2.0 Dimulai!**\n\
            ═══════════════════════════════\n\
            📊 Mode Trading: {}\n\
            💰 Max Posisi: {:.2} SOL\n\
            📈 Take Profit: {:.1}%\n\
            🛑 Stop Loss: {:.1}%\n\
            🔄 Trailing Stop: aktif setelah +{:.1}%, jarak {:.1}%\n\
            🔍 Min Skor Beli: {:.1}/100\n\
            💧 Min Likuiditas: ${:.0}",
            if self.trading_config.trading_enabled && self.wallet.is_some() { "🟢 AKTIF" } else { "🔴 ANALISIS ONLY" },
            self.trading_config.max_position_sol,
            self.trading_config.take_profit_percent,
            self.trading_config.stop_loss_percent,
            self.trading_config.trailing_start_percent,
            self.trading_config.trailing_distance_percent,
            self.trading_config.min_score_to_buy,
            self.trading_config.min_liquidity_usd,
        );
        let _ = self.send_message(&startup_msg).await;

        let mut last_profit_check = Instant::now();
        let mut scan_count = 0u64;

        loop {
            self.reset_daily_count_if_needed();

            // -------------------------------------------------------
            // AUTO UPDATE HARGA SOL (setiap 5 menit)
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
            // CEK & SELL POSISI AKTIF (setiap 60 detik)
            // -------------------------------------------------------
            if self.last_sell_check.elapsed() >= Duration::from_secs(SELL_CHECK_INTERVAL_SECS) {
                self.check_and_sell_positions().await;
                self.check_and_paper_sell().await;
                self.last_sell_check = Instant::now();
            }

            // -------------------------------------------------------
            // PAPER TRADING - Laporan periodik
            // -------------------------------------------------------
            if self.paper_config.enabled
                && self.last_paper_report.elapsed() >= Duration::from_secs(self.paper_config.report_interval_secs)
            {
                let mut prices: HashMap<String, f64> = HashMap::new();
                for addr in self.paper_state.positions.keys() {
                    if let Ok(resp) = self.client
                        .get(format!("https://api.dexscreener.com/latest/dex/tokens/{}", addr))
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

            if self.is_paused {
                println!("⏸ Bot dijeda, menunggu 30 detik...");
                sleep(Duration::from_secs(30)).await;
                continue;
            }

            scan_count += 1;
            println!("\n{}", "=".repeat(50));
            println!("🔍 Scan #{} - {} - {} token dilihat, {} posisi aktif",
                scan_count,
                Utc::now().format("%H:%M:%S"),
                self.data.seen_tokens.len(),
                self.positions.len(),
            );

            // -------------------------------------------------------
            // SCAN TOKEN BARU
            // -------------------------------------------------------
            let tokens = match self.get_new_solana_tokens().await {
                Ok(t) => t,
                Err(e) => {
                    println!("❌ Gagal ambil token: {}", e);
                    sleep(Duration::from_secs(SCAN_INTERVAL_SECS)).await;
                    continue;
                }
            };

            let new_tokens: Vec<DexToken> = tokens.into_iter()
                .filter(|t| !self.data.seen_tokens.contains_key(&t.token_address))
                .collect();

            println!("📊 {} token baru ditemukan untuk dianalisis", new_tokens.len());

            for token in &new_tokens {
                self.data.seen_tokens.insert(
                    token.token_address.clone(),
                    Utc::now().to_rfc3339(),
                );
            }

            // Analisis token baru
            for token in new_tokens.iter().take(10) {
                if self.data.daily_alert_count >= MAX_DAILY_ALERTS {
                    println!("⚠️ Batas alert harian tercapai ({}/{})", self.data.daily_alert_count, MAX_DAILY_ALERTS);
                    break;
                }

                print!("🔬 Menganalisis {} ({})... ",
                    token.name.as_deref().unwrap_or("?"),
                    token.symbol.as_deref().unwrap_or("?")
                );

                if let Some(analysis) = self.full_analyze(token).await {
                    println!("✅ Skor: {:.1}/100", analysis.total_score);

                    // Kirim alert Telegram
                    let msg = self.format_alert(&analysis);
                    let dex_url = analysis.dex_urls.first().cloned().unwrap_or_default();
                    self.send_alert_with_buttons(&msg, &dex_url).await;

                    // Kirim gambar jika ada
                    if let Some(img) = &analysis.image_url {
                        let caption = format!("{} ({}) - Skor: {:.1}/100", analysis.name, analysis.symbol, analysis.total_score);
                        self.send_photo(img, &caption).await;
                    }

                    // Tracking profit
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

                    self.data.daily_alert_count += 1;
                    self.data.performance_stats.total_alerts_sent += 1;

                    // -----------------------------------------------
                    // AUTO BUY (live) - cek apakah perlu beli
                    // -----------------------------------------------
                    self.check_and_buy(&analysis).await;

                    // -----------------------------------------------
                    // PAPER BUY (simulasi) - jalankan bersamaan
                    // -----------------------------------------------
                    self.check_and_paper_buy(&analysis).await;

                    sleep(Duration::from_secs(3)).await;
                } else {
                    println!("⏭ Skip");
                }

                sleep(Duration::from_millis(500)).await;
            }

            // -------------------------------------------------------
            // CEK PROFIT TOKEN YANG DITRACKING
            // -------------------------------------------------------
            if last_profit_check.elapsed() >= Duration::from_secs(PROFIT_CHECK_INTERVAL_SECS) {
                println!("\n💰 Mengecek profit token yang ditracking...");
                self.check_profits().await;
                last_profit_check = Instant::now();
            }

            // -------------------------------------------------------
            // SAVE DATA
            // -------------------------------------------------------
            let save_interval = chrono::Duration::minutes(SAVE_INTERVAL_MINS);
            if Utc::now().signed_duration_since(self.last_save) >= save_interval {
                if let Err(e) = self.save() {
                    println!("❌ Gagal save: {}", e);
                }
                self.last_save = Utc::now();
            }

            sleep(Duration::from_secs(SCAN_INTERVAL_SECS)).await;
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
        format!("${:.2}", amount)
    }
}

// ============================================================
// ENTRY POINT
// ============================================================

#[tokio::main]
async fn main() {
    // Load .env sebelum apapun
    let _ = dotenv::dotenv();

    // --------------------------------------------------------
    // Cek argumen CLI
    // --------------------------------------------------------
    let args: Vec<String> = std::env::args().collect();
    let is_backtest = args.iter().any(|a| a == "--backtest" || a == "-b");
    let is_compare  = args.iter().any(|a| a == "--compare" || a == "-c");
    let is_help     = args.iter().any(|a| a == "--help" || a == "-h");

    if is_help {
        println!("Bot Analisis Solana v2.0 + Auto Trade + Paper Trading + Backtest");
        println!();
        println!("PENGGUNAAN:");
        println!("  cargo run                 → Jalankan bot utama (scan & analisis)");
        println!("  cargo run -- --backtest   → Backtest strategi saat ini");
        println!("  cargo run -- --compare    → Bandingkan 8 preset konfigurasi sekaligus");
        println!("  cargo run -- --help       → Tampilkan bantuan ini");
        println!();
        println!("ENVIRONMENT VARIABLES:");
        println!("  TRADING_ENABLED=false        Master switch live trading");
        println!("  PAPER_TRADING_ENABLED=false  Simulasi trading tanpa uang nyata");
        println!("  BACKTEST_TOKEN_LIMIT=150     Jumlah token untuk backtest/compare");
        println!("  BACKTEST_MIN_AGE_HOURS=6     Umur minimum token (jam)");
        println!("  BACKTEST_MAX_AGE_HOURS=72    Umur maksimum token (jam)");
        println!("  BACKTEST_MIN_LIQUIDITY=5000  Likuiditas minimum USD");
        println!("  PAPER_BALANCE_SOL=10.0       Saldo virtual paper trading");
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
    // Mode normal: Bot scanner
    // --------------------------------------------------------
    println!("══════════════════════════════════════════");
    println!("   Bot Analisis Solana v2.0 + Auto Trade  ");
    println!("══════════════════════════════════════════");
    println!("Config dari environment:");
    println!("  TRADING_ENABLED      = {}", std::env::var("TRADING_ENABLED").unwrap_or("false".to_string()));
    println!("  PAPER_TRADING_ENABLED= {}", std::env::var("PAPER_TRADING_ENABLED").unwrap_or("false".to_string()));
    println!("  MAX_POSITION_SOL     = {}", std::env::var("MAX_POSITION_SOL").unwrap_or("0.5".to_string()));
    println!("  TAKE_PROFIT_PERCENT  = {}", std::env::var("TAKE_PROFIT_PERCENT").unwrap_or("40.0".to_string()));
    println!("  STOP_LOSS_PERCENT    = {}", std::env::var("STOP_LOSS_PERCENT").unwrap_or("15.0".to_string()));
    println!("  WALLET_PRIVATE_KEY   = {}", if std::env::var("WALLET_PRIVATE_KEY").is_ok() { "✅ SET" } else { "❌ NOT SET" });
    println!("══════════════════════════════════════════");
    println!();
    println!("  Tip: Jalankan 'cargo run -- --backtest' untuk backtest strategi");
    println!("       Jalankan 'cargo run -- --help' untuk bantuan lengkap");
    println!("══════════════════════════════════════════\n");

    let mut bot = SolanaBot::new();
    bot.run().await;
}

// ============================================================
// MODE BACKTEST
// ============================================================

// ============================================================
// MODE COMPARE - Bandingkan 8 konfigurasi sekaligus
// ============================================================

async fn run_compare_mode() {
    println!("══════════════════════════════════════════════════");
    println!("   COMPARE MODE - Perbandingan 8 Preset Strategi  ");
    println!("══════════════════════════════════════════════════\n");

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .use_rustls_tls()
        .build()
    {
        Ok(c) => c,
        Err(e) => { eprintln!("[ERROR] Gagal membuat HTTP client: {}", e); return; }
    };

    let base_config = strategy::TradingConfig::from_env();
    let bt_config   = backtest::BacktestConfig::from_env();
    let tg_token    = std::env::var("TELEGRAM_BOT_TOKEN").ok();
    let tg_chat     = std::env::var("TELEGRAM_CHAT_ID").ok();

    match backtest::run_backtest_compare(&client, &base_config, &bt_config, None).await {
        Ok(result) => {
            // 1. Print tabel ke console
            backtest::print_compare_table(&result);

            // 2. Simpan ke file JSON
            if let Err(e) = backtest::save_compare_result(&result) {
                eprintln!("[COMPARE] Gagal simpan hasil: {}", e);
            }

            // 3. Kirim ke Telegram
            if let (Some(token), Some(chat)) = (tg_token, tg_chat) {
                let msg = backtest::format_compare_telegram(&result);
                println!("[COMPARE] Mengirim hasil ke Telegram...");
                let tg_url = format!("https://api.telegram.org/bot{}/sendMessage", token);
                let payload = serde_json::json!({
                    "chat_id": chat,
                    "text": msg,
                    "parse_mode": "Markdown"
                });
                match client.post(&tg_url).json(&payload).send().await {
                    Ok(r) if r.status().is_success() => println!("[COMPARE] ✅ Laporan terkirim ke Telegram"),
                    Ok(r) => eprintln!("[COMPARE] Telegram error: {}", r.status()),
                    Err(e) => eprintln!("[COMPARE] Gagal kirim Telegram: {}", e),
                }
            } else {
                println!("[COMPARE] Telegram tidak dikonfigurasi - hasil hanya di console dan file JSON");
            }

            // 4. Saran konfigurasi terbaik
            println!();
            println!("💡 TIP: Untuk menerapkan strategi terbaik ke bot utama, tambahkan ke .env:");
            if let Some(winner) = result.scenarios.first() {
                let parts: Vec<&str> = winner.label.split('/').collect();
                for part in &parts {
                    let kv: Vec<&str> = part.splitn(2, |c: char| !c.is_alphabetic() && c != '_').collect();
                    if kv.len() >= 1 {
                        // Print konfigurasi
                    }
                }
                println!("   Strategi \"{}\" → {}", winner.name, winner.label);
                println!("   Lihat file compare_*.json untuk detail lengkap.");
            }
        }
        Err(e) => eprintln!("[COMPARE] ❌ Error: {}", e),
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
            eprintln!("[ERROR] Gagal membuat HTTP client: {}", e);
            return;
        }
    };

    let trading_config = strategy::TradingConfig::from_env();
    let bt_config      = backtest::BacktestConfig::from_env();

    // Ambil konfigurasi Telegram (opsional)
    let tg_token = std::env::var("TELEGRAM_BOT_TOKEN").ok();
    let tg_chat  = std::env::var("TELEGRAM_CHAT_ID").ok();

    match backtest::run_backtest(&client, &trading_config, &bt_config).await {
        Ok(result) => {
            // 1. Print laporan ke console
            backtest::print_backtest_report(&result);

            // 2. Simpan ke file JSON
            if let Err(e) = backtest::save_backtest_result(&result) {
                eprintln!("[BACKTEST] Gagal simpan hasil: {}", e);
            }

            // 3. Kirim ke Telegram jika konfigurasi tersedia
            if let (Some(token), Some(chat)) = (tg_token, tg_chat) {
                let msg = backtest::format_backtest_telegram(&result);
                println!("[BACKTEST] Mengirim laporan ke Telegram...");
                let tg_url = format!("https://api.telegram.org/bot{}/sendMessage", token);
                let payload = serde_json::json!({
                    "chat_id": chat,
                    "text": msg,
                    "parse_mode": "Markdown"
                });
                match client.post(&tg_url).json(&payload).send().await {
                    Ok(r) if r.status().is_success() => println!("[BACKTEST] ✅ Laporan terkirim ke Telegram"),
                    Ok(r) => eprintln!("[BACKTEST] Telegram error: {}", r.status()),
                    Err(e) => eprintln!("[BACKTEST] Gagal kirim Telegram: {}", e),
                }
            } else {
                println!("[BACKTEST] Telegram tidak dikonfigurasi - laporan hanya di console dan file JSON");
            }
        }
        Err(e) => {
            eprintln!("[BACKTEST] ❌ Error: {}", e);
        }
    }
}
