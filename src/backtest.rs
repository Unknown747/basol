// ============================================================
// BACKTESTING ENGINE
// Simulasi strategi menggunakan data historis DexScreener
// Jalankan: cargo run -- --backtest
// ============================================================

use crate::strategy::TradingConfig;
use anyhow::Result;
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ============================================================
// KONFIGURASI BACKTEST
// ============================================================

pub struct BacktestConfig {
    pub token_limit: usize,
    pub min_age_hours: i64,
    pub max_age_hours: i64,
    pub min_liquidity_usd: f64,
    pub min_volume_h24: f64,
    pub sol_price_usd: f64,
    pub telegram_token: Option<String>,
    pub telegram_chat: Option<String>,
}

impl BacktestConfig {
    pub fn from_env() -> Self {
        Self {
            token_limit: std::env::var("BACKTEST_TOKEN_LIMIT")
                .ok().and_then(|v| v.parse().ok()).unwrap_or(150),
            min_age_hours: std::env::var("BACKTEST_MIN_AGE_HOURS")
                .ok().and_then(|v| v.parse().ok()).unwrap_or(6),
            max_age_hours: std::env::var("BACKTEST_MAX_AGE_HOURS")
                .ok().and_then(|v| v.parse().ok()).unwrap_or(72),
            min_liquidity_usd: std::env::var("BACKTEST_MIN_LIQUIDITY")
                .ok().and_then(|v| v.parse().ok()).unwrap_or(5_000.0),
            min_volume_h24: std::env::var("BACKTEST_MIN_VOLUME")
                .ok().and_then(|v| v.parse().ok()).unwrap_or(10_000.0),
            sol_price_usd: std::env::var("SOL_PRICE_USD")
                .ok().and_then(|v| v.parse().ok()).unwrap_or(170.0),
            telegram_token: std::env::var("TELEGRAM_BOT_TOKEN").ok(),
            telegram_chat: std::env::var("TELEGRAM_CHAT_ID").ok(),
        }
    }
}

// ============================================================
// DEXSCREENER API STRUCTS
// ============================================================

#[derive(Debug, Deserialize, Clone)]
struct DsTokenProfile {
    #[serde(rename = "tokenAddress")]
    token_address: String,
    #[serde(rename = "chainId")]
    chain_id: String,
}

#[derive(Debug, Deserialize)]
struct DsProfileResponse(Vec<DsTokenProfile>);

#[derive(Debug, Deserialize, Clone)]
struct DsPair {
    #[serde(rename = "chainId")]
    chain_id: String,
    #[serde(rename = "baseToken")]
    base_token: DsBaseToken,
    #[serde(rename = "priceUsd")]
    price_usd: Option<String>,
    #[serde(rename = "pairCreatedAt")]
    pair_created_at: Option<i64>,
    liquidity: Option<DsLiquidity>,
    volume: Option<DsVolume>,
    #[serde(rename = "priceChange")]
    price_change: Option<DsPriceChange>,
    #[serde(rename = "marketCap")]
    market_cap: Option<f64>,
    txns: Option<DsTxns>,
}

#[derive(Debug, Deserialize, Clone)]
struct DsBaseToken {
    address: String,
    name: String,
    symbol: String,
}

#[derive(Debug, Deserialize, Clone)]
struct DsLiquidity {
    usd: Option<f64>,
}

#[derive(Debug, Deserialize, Clone)]
struct DsVolume {
    m5: Option<f64>,
    h1: Option<f64>,
    h6: Option<f64>,
    h24: Option<f64>,
}

#[derive(Debug, Deserialize, Clone)]
struct DsPriceChange {
    m5: Option<f64>,
    h1: Option<f64>,
    h6: Option<f64>,
    h24: Option<f64>,
}

#[derive(Debug, Deserialize, Clone)]
struct DsTxns {
    m5: Option<DsTxnCount>,
    h1: Option<DsTxnCount>,
    h24: Option<DsTxnCount>,
}

#[derive(Debug, Deserialize, Clone)]
struct DsTxnCount {
    buys: Option<u32>,
    sells: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct DsPairsResponse {
    pairs: Option<Vec<DsPair>>,
}

// ============================================================
// PRICE HISTORY - Rekonstruksi harga historis
// ============================================================

/// 5 titik harga yang bisa direkonstruksi dari DexScreener
#[derive(Debug, Clone)]
pub struct PriceTimeline {
    pub at_24h_ago: f64,    // "Harga entry" saat token ditemukan bot
    pub at_6h_ago: f64,     // Harga 6 jam setelah entry
    pub at_1h_ago: f64,     // Harga 1 jam terakhir
    pub at_5m_ago: f64,     // Harga 5 menit terakhir
    pub current: f64,       // Harga sekarang
}

impl PriceTimeline {
    /// Rekonstruksi dari data DexScreener menggunakan price change
    pub fn reconstruct(current_price: f64, price_change: &DsPriceChange) -> Self {
        // Harga di masa lalu = harga_sekarang / (1 + perubahan%)
        let at_24h = if let Some(ch) = price_change.h24 {
            current_price / (1.0 + ch / 100.0)
        } else {
            current_price
        };

        let at_6h = if let Some(ch) = price_change.h6 {
            current_price / (1.0 + ch / 100.0)
        } else {
            current_price
        };

        let at_1h = if let Some(ch) = price_change.h1 {
            current_price / (1.0 + ch / 100.0)
        } else {
            current_price
        };

        let at_5m = if let Some(ch) = price_change.m5 {
            current_price / (1.0 + ch / 100.0)
        } else {
            current_price
        };

        // Hindari harga negatif (bisa terjadi jika perubahan > 100%)
        Self {
            at_24h_ago: at_24h.abs().max(current_price * 0.001),
            at_6h_ago: at_6h.abs().max(current_price * 0.001),
            at_1h_ago: at_1h.abs().max(current_price * 0.001),
            at_5m_ago: at_5m.abs().max(current_price * 0.001),
            current: current_price,
        }
    }

    /// Harga tertinggi yang pernah dicapai dalam timeline
    pub fn peak_price(&self) -> f64 {
        [self.at_24h_ago, self.at_6h_ago, self.at_1h_ago, self.at_5m_ago, self.current]
            .iter().cloned().fold(f64::NEG_INFINITY, f64::max)
    }

    /// Harga terendah dalam timeline (setelah entry)
    pub fn trough_price(&self) -> f64 {
        [self.at_6h_ago, self.at_1h_ago, self.at_5m_ago, self.current]
            .iter().cloned().fold(f64::INFINITY, f64::min)
    }

    /// Max profit % yang bisa diraih (entry → puncak)
    pub fn max_profit_pct(&self, entry: f64) -> f64 {
        if entry == 0.0 { return 0.0; }
        (self.peak_price() - entry) / entry * 100.0
    }

    /// Max loss % yang pernah terjadi (entry → lembah)
    pub fn max_loss_pct(&self, entry: f64) -> f64 {
        if entry == 0.0 { return 0.0; }
        (self.trough_price() - entry) / entry * 100.0
    }
}

// ============================================================
// SIMULASI TRADE
// ============================================================

#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum ExitReason {
    TakeProfit,
    StopLoss,
    TrailingStop,
    HoldToEnd,
}

#[derive(Debug, Clone, Serialize)]
pub struct BacktestTrade {
    pub token_address: String,
    pub symbol: String,
    pub name: String,
    pub entry_price: f64,
    pub exit_price: f64,
    pub amount_sol: f64,
    pub profit_pct: f64,
    pub profit_sol: f64,
    pub exit_reason: ExitReason,
    pub score_estimated: f64,
    pub liquidity_usd: f64,
    pub volume_h24: f64,
    pub market_cap: Option<f64>,
    pub age_hours: f64,
    pub max_profit_achievable: f64,
    pub max_loss_seen: f64,
    pub skipped_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BacktestResult {
    pub run_time: DateTime<Utc>,
    pub tokens_analyzed: usize,
    pub tokens_bought: usize,
    pub tokens_skipped: usize,
    pub winning_trades: usize,
    pub losing_trades: usize,
    pub breakeven_trades: usize,
    pub total_profit_sol: f64,
    pub total_loss_sol: f64,
    pub net_pnl_sol: f64,
    pub roi_pct: f64,
    pub win_rate_pct: f64,
    pub profit_factor: f64,
    pub avg_profit_pct: f64,
    pub avg_loss_pct: f64,
    pub best_trade_pct: f64,
    pub best_trade_symbol: String,
    pub worst_trade_pct: f64,
    pub worst_trade_symbol: String,
    pub avg_hold_periods: String,
    pub total_sol_deployed: f64,
    pub config_tp: f64,
    pub config_sl: f64,
    pub config_trailing_start: f64,
    pub config_trailing_dist: f64,
    pub config_min_score: f64,
    pub trades: Vec<BacktestTrade>,
    pub skip_reasons: HashMap<String, usize>,
}

// ============================================================
// SCORING (versi cepat, tanpa Helius)
// ============================================================

fn estimate_score(pair: &DsPair) -> f64 {
    let mut score = 0.0;

    // Likuiditas (max 20)
    let liq = pair.liquidity.as_ref().and_then(|l| l.usd).unwrap_or(0.0);
    score += if liq >= 100_000.0 { 20.0 }
        else if liq >= 50_000.0 { 17.0 }
        else if liq >= 20_000.0 { 14.0 }
        else if liq >= 10_000.0 { 10.0 }
        else if liq >= 5_000.0  { 6.0 }
        else { 2.0 };

    // Volume (max 15)
    let vol_h24 = pair.volume.as_ref().and_then(|v| v.h24).unwrap_or(0.0);
    score += if vol_h24 >= 500_000.0 { 15.0 }
        else if vol_h24 >= 100_000.0 { 12.0 }
        else if vol_h24 >= 50_000.0  { 9.0 }
        else if vol_h24 >= 10_000.0  { 6.0 }
        else if vol_h24 >= 1_000.0   { 3.0 }
        else { 0.0 };

    // Momentum harga (max 20)
    if let Some(pc) = &pair.price_change {
        // Momentum 1 jam
        let m1h = pc.h1.unwrap_or(0.0);
        score += if m1h >= 50.0 { 10.0 }
            else if m1h >= 20.0 { 8.0 }
            else if m1h >= 5.0  { 5.0 }
            else if m1h >= -5.0 { 3.0 }
            else if m1h >= -15.0 { 1.0 }
            else { 0.0 };

        // Momentum 5 menit
        let m5m = pc.m5.unwrap_or(0.0);
        score += if m5m >= 5.0 { 10.0 }
            else if m5m >= 2.0 { 7.0 }
            else if m5m >= 0.0 { 4.0 }
            else if m5m >= -5.0 { 2.0 }
            else { 0.0 };
    }

    // Buy pressure (max 15)
    let buy_pressure = if let Some(txns) = &pair.txns {
        let h1 = txns.h1.as_ref();
        let buys = h1.and_then(|t| t.buys).unwrap_or(0) as f64;
        let sells = h1.and_then(|t| t.sells).unwrap_or(0) as f64;
        let total = buys + sells;
        if total > 0.0 { buys / total } else { 0.5 }
    } else { 0.5 };

    score += (buy_pressure * 15.0).min(15.0);

    // Market cap sehat (max 10)
    if let Some(mc) = pair.market_cap {
        score += if mc >= 50_000.0 && mc <= 5_000_000.0 { 10.0 }
            else if mc >= 10_000.0 && mc <= 20_000_000.0 { 7.0 }
            else if mc >= 1_000.0 { 4.0 }
            else { 0.0 };
    } else {
        score += 5.0; // Unknown MC - moderate score
    }

    // Mint authority: backtest tidak punya data on-chain (Helius tidak dipanggil).
    // Live bot skip token jika mint authority TIDAK direvoke.
    // Untuk konservatif, beri 0 → backtest undercount peluang, tapi tidak misleading.
    // score += 0.0;

    // Holder distribution (estimasi dari buy pressure, max 10)
    score += (buy_pressure * 10.0).min(10.0);

    score.min(100.0)
}

// ============================================================
// SIMULASI EXIT - Tentukan kapan posisi tertutup
// ============================================================

fn simulate_exit(
    timeline: &PriceTimeline,
    entry_price: f64,
    config: &TradingConfig,
) -> (f64, ExitReason) {
    let tp = config.take_profit_percent;
    let sl = config.stop_loss_percent;
    let trail_start = config.trailing_start_percent;
    let trail_dist = config.trailing_distance_percent;

    // Simulasikan urutan harga dari entry
    let checkpoints = [
        timeline.at_6h_ago,
        timeline.at_1h_ago,
        timeline.at_5m_ago,
        timeline.current,
    ];

    let mut highest = entry_price;
    let mut trailing_active = false;
    let mut trailing_stop = 0.0;

    for &price in &checkpoints {
        let pct = (price - entry_price) / entry_price * 100.0;

        // Update trailing
        if price > highest {
            highest = price;
        }

        // Cek take profit
        if pct >= tp {
            return (price, ExitReason::TakeProfit);
        }

        // Cek stop loss
        if pct <= -sl {
            return (price, ExitReason::StopLoss);
        }

        // Trailing stop logic
        if pct >= trail_start {
            if !trailing_active {
                trailing_active = true;
                trailing_stop = highest * (1.0 - trail_dist / 100.0);
            } else {
                let new_stop = highest * (1.0 - trail_dist / 100.0);
                if new_stop > trailing_stop {
                    trailing_stop = new_stop;
                }
            }
        }

        if trailing_active && price <= trailing_stop {
            return (price, ExitReason::TrailingStop);
        }
    }

    // Hold sampai akhir data
    (timeline.current, ExitReason::HoldToEnd)
}

// ============================================================
// FETCH DATA
// ============================================================

async fn fetch_recent_tokens(
    client: &Client,
    config: &BacktestConfig,
) -> Result<Vec<DsPair>> {
    println!("[BACKTEST] Mengambil daftar token terbaru dari DexScreener...");

    // Ambil token profiles terbaru
    let profiles_resp: Vec<DsTokenProfile> = client
        .get("https://api.dexscreener.com/token-profiles/latest/v1")
        .send().await?
        .json::<Vec<DsTokenProfile>>().await
        .unwrap_or_default();

    let solana_tokens: Vec<String> = profiles_resp.into_iter()
        .filter(|t| t.chain_id == "solana")
        .map(|t| t.token_address)
        .take(config.token_limit)
        .collect();

    println!("[BACKTEST] Ditemukan {} token Solana, mengambil data pair...", solana_tokens.len());

    let mut all_pairs: Vec<DsPair> = Vec::new();
    let now_ms = Utc::now().timestamp_millis();

    // Fetch pair data per batch (DexScreener max 30 token per request)
    for chunk in solana_tokens.chunks(30) {
        let addr_str = chunk.join(",");
        let url = format!("https://api.dexscreener.com/latest/dex/tokens/{}", addr_str);

        match client.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => {
                if let Ok(pr) = resp.json::<DsPairsResponse>().await {
                    let pairs = pr.pairs.unwrap_or_default();
                    for pair in pairs {
                        if pair.chain_id != "solana" { continue; }

                        // Filter berdasarkan umur
                        if let Some(created_ms) = pair.pair_created_at {
                            let age_hours = (now_ms - created_ms) / 3_600_000;
                            if age_hours < config.min_age_hours || age_hours > config.max_age_hours {
                                continue;
                            }
                        } else {
                            continue; // Skip token tanpa timestamp
                        }

                        // Filter likuiditas minimal
                        let liq = pair.liquidity.as_ref().and_then(|l| l.usd).unwrap_or(0.0);
                        if liq < config.min_liquidity_usd { continue; }

                        // Filter volume minimal
                        let vol = pair.volume.as_ref().and_then(|v| v.h24).unwrap_or(0.0);
                        if vol < config.min_volume_h24 { continue; }

                        all_pairs.push(pair);
                    }
                }
            }
            _ => {}
        }

        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    }

    println!("[BACKTEST] {} pair lolos filter → siap dianalisis", all_pairs.len());
    Ok(all_pairs)
}

// ============================================================
// INNER SIMULATION - Jalankan pada data yang sudah di-fetch
// ============================================================

fn simulate_on_pairs(
    pairs: &[DsPair],
    trading_config: &TradingConfig,
    run_time: DateTime<Utc>,
) -> BacktestResult {
    let now_ms = Utc::now().timestamp_millis();
    let mut trades: Vec<BacktestTrade> = Vec::new();
    let mut skip_reasons: HashMap<String, usize> = HashMap::new();
    let mut total_sol_deployed = 0.0_f64;

    for pair in pairs {
        let symbol = pair.base_token.symbol.clone();
        let name  = pair.base_token.name.clone();
        let addr  = pair.base_token.address.clone();

        let current_price = match pair.price_usd.as_ref().and_then(|p| p.parse::<f64>().ok()) {
            Some(p) if p > 0.0 => p,
            _ => {
                *skip_reasons.entry("Harga tidak tersedia".to_string()).or_insert(0) += 1;
                trades.push(BacktestTrade {
                    token_address: addr, symbol, name,
                    entry_price: 0.0, exit_price: 0.0,
                    amount_sol: 0.0, profit_pct: 0.0, profit_sol: 0.0,
                    exit_reason: ExitReason::HoldToEnd,
                    score_estimated: 0.0,
                    liquidity_usd: 0.0, volume_h24: 0.0,
                    market_cap: None, age_hours: 0.0,
                    max_profit_achievable: 0.0, max_loss_seen: 0.0,
                    skipped_reason: Some("Harga tidak tersedia".to_string()),
                });
                continue;
            }
        };

        let liq_usd = pair.liquidity.as_ref().and_then(|l| l.usd).unwrap_or(0.0);
        let vol_h24 = pair.volume.as_ref().and_then(|v| v.h24).unwrap_or(0.0);
        let age_hours = pair.pair_created_at
            .map(|t| (now_ms - t) as f64 / 3_600_000.0)
            .unwrap_or(0.0);

        let timeline = if let Some(pc) = &pair.price_change {
            PriceTimeline::reconstruct(current_price, pc)
        } else {
            PriceTimeline {
                at_24h_ago: current_price, at_6h_ago: current_price,
                at_1h_ago: current_price, at_5m_ago: current_price,
                current: current_price,
            }
        };

        let entry_price = if age_hours >= 24.0 { timeline.at_24h_ago }
            else if age_hours >= 6.0  { timeline.at_6h_ago }
            else if age_hours >= 1.0  { timeline.at_1h_ago }
            else                      { timeline.at_5m_ago };

        let score = estimate_score(pair);
        let mut skip_reason: Option<String> = None;

        if score < trading_config.min_score_to_buy {
            let r = format!("Skor {:.0} < minimum {:.0}", score, trading_config.min_score_to_buy);
            *skip_reasons.entry(r.clone()).or_insert(0) += 1;
            skip_reason = Some(r);
        } else if liq_usd < trading_config.min_liquidity_usd {
            let r = format!("Likuiditas ${:.0} < minimum ${:.0}", liq_usd, trading_config.min_liquidity_usd);
            *skip_reasons.entry(r.clone()).or_insert(0) += 1;
            skip_reason = Some(r);
        }

        if let Some(ref reason) = skip_reason {
            trades.push(BacktestTrade {
                token_address: addr, symbol, name,
                entry_price, exit_price: 0.0,
                amount_sol: 0.0, profit_pct: 0.0, profit_sol: 0.0,
                exit_reason: ExitReason::HoldToEnd,
                score_estimated: score,
                liquidity_usd: liq_usd, volume_h24: vol_h24,
                market_cap: pair.market_cap, age_hours,
                max_profit_achievable: timeline.max_profit_pct(entry_price),
                max_loss_seen: timeline.max_loss_pct(entry_price),
                skipped_reason: Some(reason.clone()),
            });
            continue;
        }

        let score_multiplier = ((score - 75.0) / 25.0).max(0.0).min(1.0);
        let amount_sol = (score_multiplier * trading_config.max_position_sol).max(0.05);
        let (exit_price, exit_reason) = simulate_exit(&timeline, entry_price, trading_config);

        let profit_pct = if entry_price > 0.0 {
            (exit_price - entry_price) / entry_price * 100.0
        } else { 0.0 };
        let profit_sol = amount_sol * profit_pct / 100.0;
        total_sol_deployed += amount_sol;

        trades.push(BacktestTrade {
            token_address: addr, symbol, name,
            entry_price, exit_price, amount_sol, profit_pct, profit_sol,
            exit_reason, score_estimated: score,
            liquidity_usd: liq_usd, volume_h24: vol_h24,
            market_cap: pair.market_cap, age_hours,
            max_profit_achievable: timeline.max_profit_pct(entry_price),
            max_loss_seen: timeline.max_loss_pct(entry_price),
            skipped_reason: None,
        });
    }

    // --- Hitung statistik ---
    let bought: Vec<&BacktestTrade> = trades.iter().filter(|t| t.skipped_reason.is_none()).collect();
    let skipped = trades.iter().filter(|t| t.skipped_reason.is_some()).count();
    let winning: Vec<_> = bought.iter().filter(|t| t.profit_pct > 0.5).cloned().collect();
    let losing: Vec<_>  = bought.iter().filter(|t| t.profit_pct < -0.5).cloned().collect();
    let breakeven = bought.len() - winning.len() - losing.len();

    let total_profit: f64 = bought.iter().map(|t| t.profit_sol.max(0.0)).sum();
    let total_loss: f64   = bought.iter().map(|t| (-t.profit_sol).max(0.0)).sum();
    let net_pnl = total_profit - total_loss;

    let roi = if total_sol_deployed > 0.0 { net_pnl / total_sol_deployed * 100.0 } else { 0.0 };
    let win_rate = if !bought.is_empty() { winning.len() as f64 / bought.len() as f64 * 100.0 } else { 0.0 };
    let profit_factor = if total_loss > 0.0 { total_profit / total_loss }
        else if total_profit > 0.0 { f64::INFINITY }
        else { 0.0 };

    let avg_profit = if !winning.is_empty() {
        winning.iter().map(|t| t.profit_pct).sum::<f64>() / winning.len() as f64
    } else { 0.0 };
    let avg_loss = if !losing.is_empty() {
        losing.iter().map(|t| t.profit_pct).sum::<f64>() / losing.len() as f64
    } else { 0.0 };

    let best  = bought.iter().max_by(|a, b| a.profit_pct.partial_cmp(&b.profit_pct).unwrap());
    let worst = bought.iter().min_by(|a, b| a.profit_pct.partial_cmp(&b.profit_pct).unwrap());

    let tp_count    = bought.iter().filter(|t| t.exit_reason == ExitReason::TakeProfit).count();
    let sl_count    = bought.iter().filter(|t| t.exit_reason == ExitReason::StopLoss).count();
    let tr_count    = bought.iter().filter(|t| t.exit_reason == ExitReason::TrailingStop).count();
    let hold_count  = bought.iter().filter(|t| t.exit_reason == ExitReason::HoldToEnd).count();

    BacktestResult {
        run_time,
        tokens_analyzed: trades.len(),
        tokens_bought: bought.len(),
        tokens_skipped: skipped,
        winning_trades: winning.len(),
        losing_trades: losing.len(),
        breakeven_trades: breakeven,
        total_profit_sol: total_profit,
        total_loss_sol: total_loss,
        net_pnl_sol: net_pnl,
        roi_pct: roi,
        win_rate_pct: win_rate,
        profit_factor,
        avg_profit_pct: avg_profit,
        avg_loss_pct: avg_loss,
        best_trade_pct:    best.map(|t| t.profit_pct).unwrap_or(0.0),
        best_trade_symbol: best.map(|t| t.symbol.clone()).unwrap_or_default(),
        worst_trade_pct:    worst.map(|t| t.profit_pct).unwrap_or(0.0),
        worst_trade_symbol: worst.map(|t| t.symbol.clone()).unwrap_or_default(),
        avg_hold_periods: format!(
            "TP:{} | SL:{} | Trail:{} | Hold:{}",
            tp_count, sl_count, tr_count, hold_count
        ),
        total_sol_deployed,
        config_tp:              trading_config.take_profit_percent,
        config_sl:              trading_config.stop_loss_percent,
        config_trailing_start:  trading_config.trailing_start_percent,
        config_trailing_dist:   trading_config.trailing_distance_percent,
        config_min_score:       trading_config.min_score_to_buy,
        trades,
        skip_reasons,
    }
}

// ============================================================
// SCENARIO - Preset konfigurasi untuk compare mode
// ============================================================

#[derive(Debug, Clone)]
pub struct CompareScenario {
    pub name: String,
    pub label: String,
    pub config: TradingConfig,
}

fn build_trading_config(
    tp: f64, sl: f64, trail_start: f64, trail_dist: f64, min_score: f64,
) -> TradingConfig {
    let max_pos = std::env::var("MAX_POSITION_SOL")
        .ok().and_then(|v| v.parse().ok()).unwrap_or(0.5_f64);
    TradingConfig {
        trading_enabled: true,
        max_position_sol: max_pos,
        min_position_sol: (max_pos * 0.1_f64).max(0.01_f64),
        take_profit_percent: tp,
        stop_loss_percent: sl,
        trailing_start_percent: trail_start,
        trailing_distance_percent: trail_dist,
        min_score_to_buy: min_score,
        min_liquidity_usd: std::env::var("MIN_LIQUIDITY_USD")
            .ok().and_then(|v| v.parse().ok()).unwrap_or(10_000.0),
        default_slippage: 1.0,
        max_positions: 999, // tidak ada batas di backtest
        max_hold_minutes: 0,
        time_exit_threshold_pct: 5.0,
    }
}

pub fn default_scenarios(base_config: &TradingConfig) -> Vec<CompareScenario> {
    vec![
        // 1. Konfigurasi saat ini (dari .env)
        CompareScenario {
            name: "Saat Ini (.env)".to_string(),
            label: format!("TP{:.0}/SL{:.0}/Skor{:.0}/Trail{:.0}-{:.0}",
                base_config.take_profit_percent,
                base_config.stop_loss_percent,
                base_config.min_score_to_buy,
                base_config.trailing_start_percent,
                base_config.trailing_distance_percent),
            config: TradingConfig {
                trading_enabled: true,
                max_positions: 999,
                ..*base_config
            },
        },
        // 2. Scalping - ambil untung cepat, cut loss cepat
        CompareScenario {
            name: "Scalping".to_string(),
            label: "TP15/SL8/Skor80/Trail10-3".to_string(),
            config: build_trading_config(15.0, 8.0, 10.0, 3.0, 80.0),
        },
        // 3. Agresif - target moderat, toleransi longgar
        CompareScenario {
            name: "Agresif".to_string(),
            label: "TP25/SL12/Skor80/Trail15-4".to_string(),
            config: build_trading_config(25.0, 12.0, 15.0, 4.0, 80.0),
        },
        // 4. Default Optimal - balance antara TP dan SL
        CompareScenario {
            name: "Default Optimal".to_string(),
            label: "TP40/SL15/Skor85/Trail20-5".to_string(),
            config: build_trading_config(40.0, 15.0, 20.0, 5.0, 85.0),
        },
        // 5. Konservatif - filter ketat, biarkan profit jalan
        CompareScenario {
            name: "Konservatif".to_string(),
            label: "TP60/SL20/Skor88/Trail30-7".to_string(),
            config: build_trading_config(60.0, 20.0, 30.0, 7.0, 88.0),
        },
        // 6. Ultra-selektif - hanya token terbaik, target besar
        CompareScenario {
            name: "Ultra-Selektif".to_string(),
            label: "TP80/SL15/Skor92/Trail40-5".to_string(),
            config: build_trading_config(80.0, 15.0, 40.0, 5.0, 92.0),
        },
        // 7. Trailing-Focused - andalkan trailing stop sepenuhnya
        CompareScenario {
            name: "Trailing-Focused".to_string(),
            label: "TP200/SL15/Skor85/Trail15-5".to_string(),
            config: build_trading_config(200.0, 15.0, 15.0, 5.0, 85.0),
        },
        // 8. Tight SL - lindungi modal lebih ketat
        CompareScenario {
            name: "Tight Stop-Loss".to_string(),
            label: "TP40/SL8/Skor85/Trail20-3".to_string(),
            config: build_trading_config(40.0, 8.0, 20.0, 3.0, 85.0),
        },
    ]
}

// ============================================================
// HASIL COMPARE - Ringkasan per skenario
// ============================================================

#[derive(Debug, Clone, Serialize)]
pub struct ScenarioResult {
    pub rank: usize,
    pub name: String,
    pub label: String,
    pub tokens_bought: usize,
    pub win_rate_pct: f64,
    pub roi_pct: f64,
    pub net_pnl_sol: f64,
    pub profit_factor: f64,
    pub avg_profit_pct: f64,
    pub avg_loss_pct: f64,
    pub best_trade_pct: f64,
    pub worst_trade_pct: f64,
    pub tp_exits: usize,
    pub sl_exits: usize,
    pub trail_exits: usize,
    pub hold_exits: usize,
    pub score: f64,    // skor komposit untuk ranking
}

#[derive(Debug, Serialize)]
pub struct CompareResult {
    pub run_time: DateTime<Utc>,
    pub tokens_dataset: usize,
    pub scenarios: Vec<ScenarioResult>,
    pub winner: String,
    pub winner_metric: String,
}

fn composite_score(r: &BacktestResult) -> f64 {
    // Skor komposit: gabungan ROI, win rate, dan profit factor
    // ROI berbobot 50%, win rate 30%, profit factor 20%
    let pf = r.profit_factor.min(10.0); // cap di 10 untuk menghindari infinite
    (r.roi_pct * 0.5) + (r.win_rate_pct * 0.3) + (pf * 10.0 * 0.2)
}

// ============================================================
// ENGINE UTAMA
// ============================================================

pub async fn run_backtest(
    client: &Client,
    trading_config: &TradingConfig,
    bt_config: &BacktestConfig,
) -> Result<BacktestResult> {
    let run_time = Utc::now();
    println!("\n{}", "=".repeat(60));
    println!(" BACKTESTING ENGINE - Solana Token Bot");
    println!("{}", "=".repeat(60));
    println!(" Konfigurasi Strategi:");
    println!("   Min Skor     : {:.0}", trading_config.min_score_to_buy);
    println!("   Min Likuiditas: ${:.0}", trading_config.min_liquidity_usd);
    println!("   Take Profit  : +{:.0}%", trading_config.take_profit_percent);
    println!("   Stop Loss    : -{:.0}%", trading_config.stop_loss_percent);
    println!("   Trailing Aktif: +{:.0}% | Jarak: {:.0}%",
        trading_config.trailing_start_percent,
        trading_config.trailing_distance_percent);
    println!("   Token limit  : {}", bt_config.token_limit);
    println!("   Umur token   : {}-{} jam", bt_config.min_age_hours, bt_config.max_age_hours);
    println!("{}\n", "=".repeat(60));

    let pairs = fetch_recent_tokens(client, bt_config).await?;

    if pairs.is_empty() {
        anyhow::bail!("Tidak ada token yang ditemukan. Coba perluas filter (BACKTEST_MIN_AGE_HOURS / BACKTEST_MIN_LIQUIDITY).");
    }

    let result = simulate_on_pairs(&pairs, trading_config, run_time);
    Ok(result)
}

// ============================================================
// COMPARE MODE - Fetch sekali, jalankan semua skenario
// ============================================================

pub async fn run_backtest_compare(
    client: &Client,
    base_config: &TradingConfig,
    bt_config: &BacktestConfig,
    custom_scenarios: Option<Vec<CompareScenario>>,
) -> Result<CompareResult> {
    let run_time = Utc::now();

    println!("\n{}", "=".repeat(65));
    println!(" COMPARE MODE - Perbandingan Konfigurasi Strategi");
    println!("{}", "=".repeat(65));
    println!(" Dataset: {} token | Umur: {}-{} jam | Min Liq: ${:.0}",
        bt_config.token_limit, bt_config.min_age_hours,
        bt_config.max_age_hours, bt_config.min_liquidity_usd);
    println!("{}\n", "=".repeat(65));

    // Fetch data SEKALI, dipakai untuk semua skenario
    println!("[COMPARE] Mengambil dataset token (1x fetch untuk semua skenario)...");
    let pairs = fetch_recent_tokens(client, bt_config).await?;

    if pairs.is_empty() {
        anyhow::bail!("Tidak ada token ditemukan. Perluas filter terlebih dahulu.");
    }

    println!("[COMPARE] Dataset: {} pair siap → menjalankan {} skenario...\n",
        pairs.len(),
        custom_scenarios.as_ref().map(|s| s.len()).unwrap_or(8));

    let scenarios = custom_scenarios.unwrap_or_else(|| default_scenarios(base_config));
    let mut scenario_results: Vec<ScenarioResult> = Vec::new();

    for (i, scenario) in scenarios.iter().enumerate() {
        print!("[COMPARE] ({}/{}) Skenario \"{}\" ... ",
            i + 1, scenarios.len(), scenario.name);

        let result = simulate_on_pairs(&pairs, &scenario.config, run_time);

        let pf_capped = result.profit_factor.min(10.0);
        let score = composite_score(&result);

        let tp_exits    = result.trades.iter().filter(|t| t.exit_reason == ExitReason::TakeProfit && t.skipped_reason.is_none()).count();
        let sl_exits    = result.trades.iter().filter(|t| t.exit_reason == ExitReason::StopLoss && t.skipped_reason.is_none()).count();
        let trail_exits = result.trades.iter().filter(|t| t.exit_reason == ExitReason::TrailingStop && t.skipped_reason.is_none()).count();
        let hold_exits  = result.trades.iter().filter(|t| t.exit_reason == ExitReason::HoldToEnd && t.skipped_reason.is_none()).count();

        println!("ROI:{:+.1}% | WR:{:.0}% | PF:{:.1}",
            result.roi_pct, result.win_rate_pct, pf_capped);

        scenario_results.push(ScenarioResult {
            rank: 0, // akan diisi setelah sort
            name: scenario.name.clone(),
            label: scenario.label.clone(),
            tokens_bought: result.tokens_bought,
            win_rate_pct: result.win_rate_pct,
            roi_pct: result.roi_pct,
            net_pnl_sol: result.net_pnl_sol,
            profit_factor: result.profit_factor,
            avg_profit_pct: result.avg_profit_pct,
            avg_loss_pct: result.avg_loss_pct,
            best_trade_pct: result.best_trade_pct,
            worst_trade_pct: result.worst_trade_pct,
            tp_exits, sl_exits, trail_exits, hold_exits,
            score,
        });
    }

    // Sort berdasarkan composite score
    scenario_results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
    for (i, s) in scenario_results.iter_mut().enumerate() {
        s.rank = i + 1;
    }

    let winner = scenario_results.first()
        .map(|s| s.name.clone())
        .unwrap_or_default();
    let winner_metric = scenario_results.first()
        .map(|s| format!("ROI:{:+.1}% | WR:{:.0}% | PF:{:.2}",
            s.roi_pct, s.win_rate_pct, s.profit_factor.min(10.0)))
        .unwrap_or_default();

    Ok(CompareResult {
        run_time,
        tokens_dataset: pairs.len(),
        scenarios: scenario_results,
        winner,
        winner_metric,
    })
}

// ============================================================
// PRINT LAPORAN KE CONSOLE
// ============================================================

pub fn print_backtest_report(r: &BacktestResult) {
    let pf_str = if r.profit_factor.is_infinite() {
        "∞".to_string()
    } else {
        format!("{:.2}", r.profit_factor)
    };

    println!("\n{}", "=".repeat(60));
    println!(" HASIL BACKTEST - {}", r.run_time.format("%Y-%m-%d %H:%M UTC"));
    println!("{}", "=".repeat(60));

    println!("\n📊 RINGKASAN");
    println!("   Token dianalisis  : {}", r.tokens_analyzed);
    println!("   Token dibeli      : {}", r.tokens_bought);
    println!("   Token di-skip     : {}", r.tokens_skipped);
    println!("   SOL yang digunakan: {:.4} SOL (virtual)", r.total_sol_deployed);

    println!("\n💰 KINERJA");
    println!("   Net P&L     : {}{:.4} SOL", if r.net_pnl_sol >= 0.0 { "+" } else { "" }, r.net_pnl_sol);
    println!("   ROI         : {}{:.1}%", if r.roi_pct >= 0.0 { "+" } else { "" }, r.roi_pct);
    println!("   Profit      : +{:.4} SOL", r.total_profit_sol);
    println!("   Loss        : -{:.4} SOL", r.total_loss_sol);

    println!("\n📈 STATISTIK");
    println!("   Win Rate    : {:.1}%", r.win_rate_pct);
    println!("   Profit Factor: {}", pf_str);
    println!("   Avg Profit  : +{:.1}%", r.avg_profit_pct);
    println!("   Avg Loss    : {:.1}%", r.avg_loss_pct);
    println!("   Menang: {} | Kalah: {} | Impas: {}",
        r.winning_trades, r.losing_trades, r.breakeven_trades);

    println!("\n🏆 TRADE TERBAIK & TERBURUK");
    if !r.best_trade_symbol.is_empty() {
        println!("   Best : {} → +{:.1}%", r.best_trade_symbol, r.best_trade_pct);
    }
    if !r.worst_trade_symbol.is_empty() {
        println!("   Worst: {} → {:.1}%", r.worst_trade_symbol, r.worst_trade_pct);
    }

    println!("\n🚪 DISTRIBUSI EXIT");
    println!("   {}", r.avg_hold_periods);

    println!("\n⚙️ KONFIGURASI YANG DIUJI");
    println!("   Min Skor     : {:.0}", r.config_min_score);
    println!("   Take Profit  : +{:.0}%", r.config_tp);
    println!("   Stop Loss    : -{:.0}%", r.config_sl);
    println!("   Trailing Aktif: +{:.0}% | Jarak: {:.0}%", r.config_trailing_start, r.config_trailing_dist);

    // Top 10 trades
    let mut sorted = r.trades.iter()
        .filter(|t| t.skipped_reason.is_none())
        .collect::<Vec<_>>();
    sorted.sort_by(|a, b| b.profit_pct.partial_cmp(&a.profit_pct).unwrap());

    if !sorted.is_empty() {
        println!("\n🏅 TOP 10 TRADE (berdasarkan P&L):");
        for (i, t) in sorted.iter().take(10).enumerate() {
            let exit_label = match t.exit_reason {
                ExitReason::TakeProfit   => "TP",
                ExitReason::StopLoss     => "SL",
                ExitReason::TrailingStop => "TRAIL",
                ExitReason::HoldToEnd    => "HOLD",
            };
            println!(
                "   {:2}. {:8} | {:>7.1}% | {:<5} | Skor:{:.0} | Liq:${:.0}",
                i + 1, t.symbol,
                t.profit_pct, exit_label,
                t.score_estimated, t.liquidity_usd
            );
        }
    }

    // Skip reason summary
    if !r.skip_reasons.is_empty() {
        println!("\n⏭ ALASAN SKIP TERBANYAK:");
        let mut reasons: Vec<_> = r.skip_reasons.iter().collect();
        reasons.sort_by(|a, b| b.1.cmp(a.1));
        for (reason, count) in reasons.iter().take(5) {
            println!("   {} × {}", count, reason);
        }
    }

    println!("\n{}", "=".repeat(60));
}

// ============================================================
// FORMAT LAPORAN TELEGRAM
// ============================================================

pub fn format_backtest_telegram(r: &BacktestResult) -> String {
    let roi_emoji = if r.roi_pct >= 20.0 { "🚀" }
        else if r.roi_pct >= 0.0 { "✅" }
        else if r.roi_pct >= -10.0 { "⚠️" }
        else { "❌" };

    let pf_str = if r.profit_factor.is_infinite() {
        "∞".to_string()
    } else {
        format!("{:.2}", r.profit_factor)
    };

    let mut sorted_trades = r.trades.iter()
        .filter(|t| t.skipped_reason.is_none())
        .collect::<Vec<_>>();
    sorted_trades.sort_by(|a, b| b.profit_pct.partial_cmp(&a.profit_pct).unwrap());

    let top5: String = sorted_trades.iter().take(5)
        .enumerate()
        .map(|(i, t)| {
            let exit = match t.exit_reason {
                ExitReason::TakeProfit   => "TP",
                ExitReason::StopLoss     => "SL",
                ExitReason::TrailingStop => "TRAIL",
                ExitReason::HoldToEnd    => "HOLD",
            };
            let em = if t.profit_pct >= 0.0 { "✅" } else { "❌" };
            format!("{}. {} {} {}{:.1}% [{}]",
                i + 1, em, t.symbol,
                if t.profit_pct >= 0.0 { "+" } else { "" },
                t.profit_pct, exit)
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        "📊 **LAPORAN BACKTEST**\n\
        ═══════════════════════════════\n\
        🕐 {}\n\n\
        📋 **Ringkasan:**\n\
        🔍 Token dianalisis: **{}**\n\
        🛒 Token dibeli: **{}** | ⏭ Skip: **{}**\n\
        💼 SOL digunakan: **{:.4} SOL** (virtual)\n\n\
        {} **Kinerja:**\n\
        💰 Net P&L: **{}{:.4} SOL**\n\
        📈 ROI: **{}{:.1}%**\n\
        💚 Total Profit: **+{:.4} SOL**\n\
        ❤️ Total Loss: **-{:.4} SOL**\n\n\
        🎯 **Statistik:**\n\
        ✅ Menang: **{}** | ❌ Kalah: **{}** | ➖ Impas: **{}**\n\
        📊 Win Rate: **{:.1}%**\n\
        ⚖️ Profit Factor: **{}**\n\
        📈 Avg Profit: **+{:.1}%** | 📉 Avg Loss: **{:.1}%**\n\n\
        🏆 Best: **{}** (+{:.1}%) | Worst: **{}** ({:.1}%)\n\n\
        🚪 Exit: {}\n\n\
        ⚙️ **Konfigurasi:**\n\
        Min Skor: {:.0} | TP: +{:.0}% | SL: -{:.0}%\n\
        Trailing: +{:.0}% → jarak {:.0}%\n\n\
        🏅 **Top 5 Trade:**\n\
        {}\n\n\
        ═══════════════════════════════\n\
        ⚠️ _Backtest bukan jaminan performa masa depan_",
        r.run_time.format("%Y-%m-%d %H:%M UTC"),
        r.tokens_analyzed, r.tokens_bought, r.tokens_skipped,
        r.total_sol_deployed,
        roi_emoji,
        if r.net_pnl_sol >= 0.0 { "+" } else { "" }, r.net_pnl_sol,
        if r.roi_pct >= 0.0 { "+" } else { "" }, r.roi_pct,
        r.total_profit_sol, r.total_loss_sol,
        r.winning_trades, r.losing_trades, r.breakeven_trades,
        r.win_rate_pct, pf_str,
        r.avg_profit_pct, r.avg_loss_pct,
        r.best_trade_symbol, r.best_trade_pct,
        r.worst_trade_symbol, r.worst_trade_pct,
        r.avg_hold_periods,
        r.config_min_score, r.config_tp, r.config_sl,
        r.config_trailing_start, r.config_trailing_dist,
        if top5.is_empty() { "Tidak ada trade".to_string() } else { top5 },
    )
}

// ============================================================
// PRINT TABEL COMPARE KE CONSOLE
// ============================================================

pub fn print_compare_table(r: &CompareResult) {
    let sep = "─".repeat(100);
    let double = "═".repeat(100);

    println!("\n{}", double);
    println!(" HASIL COMPARE - {} | Dataset: {} token",
        r.run_time.format("%Y-%m-%d %H:%M UTC"), r.tokens_dataset);
    println!("{}", double);

    // Header
    println!(" {:>3} {:<22} {:>6} {:>7} {:>8} {:>8} {:>7} {:>8} {:>8} {:>6}",
        "Rank", "Nama Strategi", "Trade", "WinRate", "ROI%", "P&L(SOL)", "PF", "AvgProfit", "AvgLoss", "TP|SL|TR");
    println!("{}", sep);

    for s in &r.scenarios {
        let rank_badge = if s.rank == 1 { "🥇".to_string() }
            else if s.rank == 2 { "🥈".to_string() }
            else if s.rank == 3 { "🥉".to_string() }
            else { format!(" {:>2}.", s.rank) };

        let roi_sign = if s.roi_pct >= 0.0 { "+" } else { "" };
        let pnl_sign = if s.net_pnl_sol >= 0.0 { "+" } else { "" };
        let pf_str = if s.profit_factor.is_infinite() { "  ∞".to_string() }
            else { format!("{:>7.2}", s.profit_factor.min(99.99)) };

        println!(" {} {:<22} {:>6} {:>6.1}% {:>7.1}% {:>8.4} {:>7} {:>7.1}% {:>7.1}% {:>3}|{:>3}|{:>3}",
            rank_badge, s.name,
            s.tokens_bought,
            s.win_rate_pct,
            format!("{}{:.1}", roi_sign, s.roi_pct),
            format!("{}{:.4}", pnl_sign, s.net_pnl_sol),
            pf_str,
            s.avg_profit_pct,
            s.avg_loss_pct,
            s.tp_exits, s.sl_exits, s.trail_exits,
        );

        // Sub-baris konfigurasi (indent)
        println!("     ↳ {}", s.label);
    }

    println!("{}", sep);

    // Pemenang
    println!("\n🏆 STRATEGI TERBAIK: \"{}\"", r.winner);
    println!("   {}", r.winner_metric);

    // Keterangan metrik ranking
    println!("\n📝 Ranking berdasarkan skor komposit:");
    println!("   ROI (50%) + Win Rate (30%) + Profit Factor (20%)");
    println!("   TP=Take Profit | SL=Stop Loss | TR=Trailing Stop exits");
    println!("{}", double);
}

// ============================================================
// FORMAT COMPARE UNTUK TELEGRAM
// ============================================================

pub fn format_compare_telegram(r: &CompareResult) -> String {
    let rows: String = r.scenarios.iter().map(|s| {
        let badge = if s.rank == 1 { "🥇" }
            else if s.rank == 2 { "🥈" }
            else if s.rank == 3 { "🥉" }
            else { "▫️" };

        let pf_str = if s.profit_factor.is_infinite() { "∞".to_string() }
            else { format!("{:.2}", s.profit_factor.min(99.99)) };

        format!(
            "{} **{}** — `{}`\n\
             ROI: **{}{:.1}%** | WR: **{:.0}%** | PF: **{}**\n\
             Avg: {:+.1}%/{:.1}% | TP:{} SL:{} TR:{}",
            badge, s.name, s.label,
            if s.roi_pct >= 0.0 { "+" } else { "" }, s.roi_pct,
            s.win_rate_pct, pf_str,
            s.avg_profit_pct, s.avg_loss_pct,
            s.tp_exits, s.sl_exits, s.trail_exits,
        )
    }).collect::<Vec<_>>().join("\n\n");

    format!(
        "📊 **PERBANDINGAN STRATEGI**\n\
        ═══════════════════════════════\n\
        🕐 {}\n\
        🗂 Dataset: **{}** token\n\n\
        {}\n\n\
        ═══════════════════════════════\n\
        🏆 **Pemenang: {}**\n\
        {}\n\n\
        📝 Ranking: ROI(50%) + WinRate(30%) + PF(20%)\n\
        ⚠️ _Backtest bukan jaminan performa masa depan_",
        r.run_time.format("%Y-%m-%d %H:%M UTC"),
        r.tokens_dataset,
        rows,
        r.winner,
        r.winner_metric,
    )
}

// ============================================================
// SAVE HASIL KE FILE
// ============================================================

pub fn save_backtest_result(result: &BacktestResult) -> Result<()> {
    let filename = format!("backtest_{}.json", result.run_time.format("%Y%m%d_%H%M%S"));
    let json = serde_json::to_string_pretty(result)?;
    std::fs::write(&filename, json)?;
    println!("[BACKTEST] Hasil disimpan ke {}", filename);
    Ok(())
}

pub fn save_compare_result(result: &CompareResult) -> Result<()> {
    let filename = format!("compare_{}.json", result.run_time.format("%Y%m%d_%H%M%S"));
    let json = serde_json::to_string_pretty(result)?;
    std::fs::write(&filename, json)?;
    println!("[COMPARE] Hasil disimpan ke {}", filename);
    Ok(())
}
