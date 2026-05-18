// ============================================================
// BACKTESTING ENGINE
// Simulate strategy using historical DexScreener data
// Run: cargo run -- --backtest
// ============================================================

use crate::strategy::TradingConfig;
use anyhow::Result;
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ============================================================
// BACKTEST CONFIGURATION
// ============================================================

#[allow(dead_code)]
pub struct BacktestConfig {
    pub token_limit: usize,
    pub min_age_hours: i64,
    pub max_age_hours: i64,
    pub min_liquidity_usd: f64,
    pub min_volume_h24: f64,
    pub sol_price_usd: f64,
    pub telegram_token: Option<String>,
    pub telegram_chat: Option<String>,
    /// Override for the score threshold used during backtesting.
    /// Backtest scoring (estimate_score) has a max of ~90 because it cannot
    /// call Helius (no on-chain data for security/holder checks). Using the
    /// live MIN_SCORE_TO_BUY (85) directly would cause almost zero tokens to
    /// qualify. Set BACKTEST_MIN_SCORE to a lower equivalent (default: 62).
    pub min_score: f64,
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
            min_score: std::env::var("BACKTEST_MIN_SCORE")
                .ok().and_then(|v| v.parse().ok()).unwrap_or(62.0),
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

#[allow(dead_code)]
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

#[allow(dead_code)]
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

#[allow(dead_code)]
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
// PRICE HISTORY - Historical price reconstruction
// ============================================================

/// 5 price points reconstructable from DexScreener
#[derive(Debug, Clone)]
pub struct PriceTimeline {
    pub at_24h_ago: f64,    // "Entry price" when bot discovered the token
    pub at_6h_ago: f64,     // Price 6 hours after entry
    pub at_1h_ago: f64,     // Price 1 hour ago
    pub at_5m_ago: f64,     // Price 5 minutes ago
    pub current: f64,       // Current price
}

impl PriceTimeline {
    /// Reconstruct from DexScreener data using price change percentages
    fn reconstruct(current_price: f64, price_change: &DsPriceChange) -> Self {
        // Past price = current_price / (1 + change%)
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

        // Avoid negative prices (can happen if change > 100%)
        Self {
            at_24h_ago: at_24h.abs().max(current_price * 0.001),
            at_6h_ago: at_6h.abs().max(current_price * 0.001),
            at_1h_ago: at_1h.abs().max(current_price * 0.001),
            at_5m_ago: at_5m.abs().max(current_price * 0.001),
            current: current_price,
        }
    }

    /// Highest price ever reached in the timeline
    pub fn peak_price(&self) -> f64 {
        [self.at_24h_ago, self.at_6h_ago, self.at_1h_ago, self.at_5m_ago, self.current]
            .iter().cloned().fold(f64::NEG_INFINITY, f64::max)
    }

    /// Lowest price in the timeline (after entry)
    pub fn trough_price(&self) -> f64 {
        [self.at_6h_ago, self.at_1h_ago, self.at_5m_ago, self.current]
            .iter().cloned().fold(f64::INFINITY, f64::min)
    }

    /// Max achievable profit % (entry → peak)
    pub fn max_profit_pct(&self, entry: f64) -> f64 {
        if entry == 0.0 { return 0.0; }
        (self.peak_price() - entry) / entry * 100.0
    }

    /// Max loss % that occurred (entry → trough)
    pub fn max_loss_pct(&self, entry: f64) -> f64 {
        if entry == 0.0 { return 0.0; }
        (self.trough_price() - entry) / entry * 100.0
    }
}

// ============================================================
// TRADE SIMULATION
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
// SCORING (fast version, without Helius)
// ============================================================

fn estimate_score(pair: &DsPair) -> f64 {
    let mut score = 0.0;

    // Liquidity (max 20)
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

    // Price momentum (max 20)
    if let Some(pc) = &pair.price_change {
        // 1h momentum
        let m1h = pc.h1.unwrap_or(0.0);
        score += if m1h >= 50.0 { 10.0 }
            else if m1h >= 20.0 { 8.0 }
            else if m1h >= 5.0  { 5.0 }
            else if m1h >= -5.0 { 3.0 }
            else if m1h >= -15.0 { 1.0 }
            else { 0.0 };

        // 5m momentum
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

    // Healthy market cap (max 10)
    if let Some(mc) = pair.market_cap {
        score += if (50_000.0..=5_000_000.0).contains(&mc) { 10.0 }
            else if (10_000.0..=20_000_000.0).contains(&mc) { 7.0 }
            else if mc >= 1_000.0 { 4.0 }
            else { 0.0 };
    } else {
        score += 5.0; // Unknown MC - moderate score
    }

    // Mint authority: backtest has no on-chain data (Helius not called).
    // Live bot skips tokens with mint authority NOT revoked.
    // Conservatively give 0 → backtest undercounts opportunities, but not misleading.
    // score += 0.0;

    // Holder distribution (estimated from buy pressure, max 10)
    score += (buy_pressure * 10.0).min(10.0);

    score.min(100.0)
}

// ============================================================
// EXIT SIMULATION - Determine when position closes
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
    let tp1 = config.tp1_percent;
    let tp1_sell = config.tp1_sell_percent;   // % of position sold at TP1
    let tp2 = config.tp2_percent;
    let tp2_sell = config.tp2_sell_percent;   // % of remainder sold at TP2

    // Simulate price sequence from entry
    let checkpoints = [
        timeline.at_6h_ago,
        timeline.at_1h_ago,
        timeline.at_5m_ago,
        timeline.current,
    ];

    let mut highest = entry_price;
    let mut trailing_active = false;
    let mut trailing_stop = 0.0;

    // 3-stage TP state — mirrors paper_trading evaluate_positions logic exactly
    let mut tp1_fired = false;
    let mut tp2_fired = false;

    // Weighted average exit price accumulator for partial sells
    // weight = fraction of original position sold at that price
    let mut weighted_price_sum = 0.0_f64;
    let mut weight_total = 0.0_f64;

    for &price in &checkpoints {
        let pct = (price - entry_price) / entry_price * 100.0;

        if price > highest {
            highest = price;
        }

        // --- TP1 PARTIAL ---
        if tp1 > 0.0 && !tp1_fired && pct >= tp1 {
            tp1_fired = true;
            let frac = tp1_sell / 100.0;
            weighted_price_sum += price * frac;
            weight_total += frac;
            // Continue evaluating rest of position
        }

        // --- TP2 PARTIAL ---
        if tp2 > 0.0 && tp1_fired && !tp2_fired && pct >= tp2 {
            tp2_fired = true;
            // TP2 sells tp2_sell% of the *remainder* after TP1
            let remaining_frac = 1.0 - weight_total;
            let frac = remaining_frac * tp2_sell / 100.0;
            weighted_price_sum += price * frac;
            weight_total += frac;
        }

        // --- STOP LOSS (full close of remaining) ---
        if pct <= -sl {
            let remaining = (1.0 - weight_total).max(0.0);
            weighted_price_sum += price * remaining;
            weight_total += remaining;
            let avg = if weight_total > 0.0 { weighted_price_sum / weight_total } else { price };
            return (avg, ExitReason::StopLoss);
        }

        // --- TRAILING STOP ---
        if pct >= trail_start {
            if !trailing_active {
                trailing_active = true;
                // Anchor to highest_price — matches live and paper trading exactly
                trailing_stop = highest * (1.0 - trail_dist / 100.0);
            } else {
                let new_stop = highest * (1.0 - trail_dist / 100.0);
                if new_stop > trailing_stop {
                    trailing_stop = new_stop;
                }
            }
        }
        if trailing_active && price <= trailing_stop {
            let remaining = (1.0 - weight_total).max(0.0);
            weighted_price_sum += price * remaining;
            weight_total += remaining;
            let avg = if weight_total > 0.0 { weighted_price_sum / weight_total } else { price };
            return (avg, ExitReason::TrailingStop);
        }

        // --- FINAL TP (full close of remaining) ---
        // TP1 disabled → always eligible; TP1+TP2 → must fire both first
        let tp_final_eligible = if tp1 > 0.0 {
            if tp2 > 0.0 { tp2_fired } else { tp1_fired }
        } else { true };
        if tp_final_eligible && pct >= tp {
            let remaining = (1.0 - weight_total).max(0.0);
            weighted_price_sum += price * remaining;
            weight_total += remaining;
            let avg = if weight_total > 0.0 { weighted_price_sum / weight_total } else { price };
            return (avg, ExitReason::TakeProfit);
        }
    }

    // Hold to end of data — close any remaining position at current price
    let remaining = (1.0 - weight_total).max(0.0);
    if remaining > 0.0 {
        weighted_price_sum += timeline.current * remaining;
        weight_total += remaining;
    }
    let avg = if weight_total > 0.0 { weighted_price_sum / weight_total } else { timeline.current };
    (avg, ExitReason::HoldToEnd)
}

// ============================================================
// FETCH DATA
// ============================================================

async fn fetch_recent_tokens(
    client: &Client,
    config: &BacktestConfig,
) -> Result<Vec<DsPair>> {
    println!("[BACKTEST] Fetching latest token list from DexScreener...");

    // Fetch latest token profiles
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

    println!("[BACKTEST] Found {} Solana tokens, fetching pair data...", solana_tokens.len());

    let mut all_pairs: Vec<DsPair> = Vec::new();
    let now_ms = Utc::now().timestamp_millis();

    // Fetch pair data in batches (DexScreener max 30 tokens per request)
    for chunk in solana_tokens.chunks(30) {
        let addr_str = chunk.join(",");
        let url = format!("https://api.dexscreener.com/latest/dex/tokens/{addr_str}");

        match client.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => {
                if let Ok(pr) = resp.json::<DsPairsResponse>().await {
                    let pairs = pr.pairs.unwrap_or_default();
                    for pair in pairs {
                        if pair.chain_id != "solana" { continue; }

                        // Filter by age
                        if let Some(created_ms) = pair.pair_created_at {
                            let age_hours = (now_ms - created_ms) / 3_600_000;
                            if age_hours < config.min_age_hours || age_hours > config.max_age_hours {
                                continue;
                            }
                        } else {
                            continue; // Skip tokens without timestamp
                        }

                        // Filter by minimum liquidity
                        let liq = pair.liquidity.as_ref().and_then(|l| l.usd).unwrap_or(0.0);
                        if liq < config.min_liquidity_usd { continue; }

                        // Filter by minimum volume
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

    println!("[BACKTEST] {} pairs passed filters → ready for analysis", all_pairs.len());
    Ok(all_pairs)
}

// ============================================================
// INNER SIMULATION - Run on already-fetched data
// ============================================================

// score_threshold_override: overrides trading_config.min_score_to_buy for backtest.
// Backtest estimate_score() max is ~90 (no Helius data for security/holders),
// so using the live threshold (89) would qualify almost nothing.
fn simulate_on_pairs(
    pairs: &[DsPair],
    trading_config: &TradingConfig,
    run_time: DateTime<Utc>,
    score_threshold_override: Option<f64>,
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
                *skip_reasons.entry("Price unavailable".to_string()).or_insert(0) += 1;
                trades.push(BacktestTrade {
                    token_address: addr, symbol, name,
                    entry_price: 0.0, exit_price: 0.0,
                    amount_sol: 0.0, profit_pct: 0.0, profit_sol: 0.0,
                    exit_reason: ExitReason::HoldToEnd,
                    score_estimated: 0.0,
                    liquidity_usd: 0.0, volume_h24: 0.0,
                    market_cap: None, age_hours: 0.0,
                    max_profit_achievable: 0.0, max_loss_seen: 0.0,
                    skipped_reason: Some("Price unavailable".to_string()),
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
        let effective_min_score = score_threshold_override.unwrap_or(trading_config.min_score_to_buy);
        let mut skip_reason: Option<String> = None;

        if score < effective_min_score {
            let r = format!("Score {score:.0} < minimum {effective_min_score:.0}");
            *skip_reasons.entry(r.clone()).or_insert(0) += 1;
            skip_reason = Some(r);
        } else if liq_usd < trading_config.min_liquidity_usd {
            let r = format!("Liquidity ${:.0} < minimum ${:.0}", liq_usd, trading_config.min_liquidity_usd);
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

        let score_multiplier = ((score - 75.0) / 25.0).clamp(0.0, 1.0);
        // Use configured min_position_sol (not hardcoded 0.05) so backtest matches live trading
        let amount_sol = (score_multiplier * trading_config.max_position_sol)
            .max(trading_config.min_position_sol);
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

    // --- Calculate statistics ---
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
    // Cap at 99.99 — f64::INFINITY breaks JSON serialization (serde_json → null)
    let profit_factor = if total_loss > 0.0 { (total_profit / total_loss).min(99.99) }
        else if total_profit > 0.0 { 99.99 }
        else { 0.0 };

    let avg_profit = if !winning.is_empty() {
        winning.iter().map(|t| t.profit_pct).sum::<f64>() / winning.len() as f64
    } else { 0.0 };
    let avg_loss = if !losing.is_empty() {
        losing.iter().map(|t| t.profit_pct).sum::<f64>() / losing.len() as f64
    } else { 0.0 };

    let best  = bought.iter().max_by(|a, b| a.profit_pct.total_cmp(&b.profit_pct));
    let worst = bought.iter().min_by(|a, b| a.profit_pct.total_cmp(&b.profit_pct));

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
        best_trade_pct: best.map(|t| t.profit_pct).unwrap_or(0.0),
        best_trade_symbol: best.map(|t| t.symbol.clone()).unwrap_or_default(),
        worst_trade_pct: worst.map(|t| t.profit_pct).unwrap_or(0.0),
        worst_trade_symbol: worst.map(|t| t.symbol.clone()).unwrap_or_default(),
        avg_hold_periods: format!("TP:{tp_count} SL:{sl_count} Trail:{tr_count} Hold:{hold_count}"),
        total_sol_deployed,
        config_tp: trading_config.take_profit_percent,
        config_sl: trading_config.stop_loss_percent,
        config_trailing_start: trading_config.trailing_start_percent,
        config_trailing_dist: trading_config.trailing_distance_percent,
        config_min_score: trading_config.min_score_to_buy,
        trades,
        skip_reasons,
    }
}

// ============================================================
// PUBLIC ENTRY POINTS
// ============================================================

pub async fn run_backtest(
    client: &Client,
    trading_config: &TradingConfig,
    bt_config: &BacktestConfig,
) -> Result<BacktestResult> {
    let run_time = Utc::now();
    let pairs = fetch_recent_tokens(client, bt_config).await?;
    println!(
        "[BACKTEST] Score threshold: {:.0} (live: {:.0}, backtest override: BACKTEST_MIN_SCORE)",
        bt_config.min_score, trading_config.min_score_to_buy
    );
    Ok(simulate_on_pairs(&pairs, trading_config, run_time, Some(bt_config.min_score)))
}

// ============================================================
// COMPARE MODE - Compare multiple strategy presets
// ============================================================

#[derive(Debug, Clone, Serialize)]
pub struct ScenarioResult {
    pub name: String,
    pub label: String,
    pub result: BacktestResult,
}

#[derive(Debug, Clone, Serialize)]
pub struct CompareResult {
    pub run_time: DateTime<Utc>,
    pub scenarios: Vec<ScenarioResult>,
}

pub async fn run_backtest_compare(
    client: &Client,
    _base_config: &TradingConfig,
    bt_config: &BacktestConfig,
    _extra: Option<()>,
) -> Result<CompareResult> {
    let run_time = Utc::now();
    println!("[COMPARE] Fetching token data (shared across all scenarios)...");
    let pairs = fetch_recent_tokens(client, bt_config).await?;

    let scenarios_config: Vec<(&str, TradingConfig)> = vec![
        ("Conservative", TradingConfig { trading_enabled: true, max_position_sol: 0.05, min_position_sol: 0.05, take_profit_percent: 30.0, stop_loss_percent: 10.0, trailing_start_percent: 15.0, trailing_distance_percent: 5.0, min_score_to_buy: 85.0, min_liquidity_usd: 10_000.0, default_slippage: 1.5, max_positions: 3, max_hold_minutes: 60, time_exit_threshold_pct: 5.0, tp1_percent: 0.0, tp1_sell_percent: 33.0, tp2_percent: 0.0, tp2_sell_percent: 50.0, breakeven_after_tp1: true, daily_max_loss_pct: 8.0 }),
        ("Scalping",     TradingConfig { trading_enabled: true, max_position_sol: 0.05, min_position_sol: 0.05, take_profit_percent: 35.0, stop_loss_percent: 8.0,  trailing_start_percent: 12.0, trailing_distance_percent: 3.0, min_score_to_buy: 87.0, min_liquidity_usd: 5_000.0,  default_slippage: 1.5, max_positions: 2, max_hold_minutes: 40, time_exit_threshold_pct: 3.0, tp1_percent: 12.0, tp1_sell_percent: 33.0, tp2_percent: 20.0, tp2_sell_percent: 50.0, breakeven_after_tp1: true, daily_max_loss_pct: 8.0 }),
        ("Aggressive",   TradingConfig { trading_enabled: true, max_position_sol: 0.1,  min_position_sol: 0.05, take_profit_percent: 50.0, stop_loss_percent: 15.0, trailing_start_percent: 25.0, trailing_distance_percent: 8.0, min_score_to_buy: 80.0, min_liquidity_usd: 5_000.0,  default_slippage: 2.0, max_positions: 5, max_hold_minutes: 0,  time_exit_threshold_pct: 5.0, tp1_percent: 0.0, tp1_sell_percent: 33.0, tp2_percent: 0.0, tp2_sell_percent: 50.0, breakeven_after_tp1: true, daily_max_loss_pct: 8.0 }),
        ("Balanced",     TradingConfig { trading_enabled: true, max_position_sol: 0.05, min_position_sol: 0.05, take_profit_percent: 40.0, stop_loss_percent: 12.0, trailing_start_percent: 20.0, trailing_distance_percent: 5.0, min_score_to_buy: 85.0, min_liquidity_usd: 8_000.0,  default_slippage: 1.5, max_positions: 3, max_hold_minutes: 50, time_exit_threshold_pct: 4.0, tp1_percent: 0.0, tp1_sell_percent: 33.0, tp2_percent: 0.0, tp2_sell_percent: 50.0, breakeven_after_tp1: true, daily_max_loss_pct: 8.0 }),
    ];

    let mut scenarios = Vec::new();
    for (name, config) in &scenarios_config {
        println!("[COMPARE] Running scenario: {name}");
        let result = simulate_on_pairs(&pairs, config, run_time, Some(bt_config.min_score));
        let label = format!("TP{}/SL{}/Trail{}", config.take_profit_percent, config.stop_loss_percent, config.trailing_start_percent);
        scenarios.push(ScenarioResult { name: name.to_string(), label, result });
    }

    // Sort by net P&L descending
    scenarios.sort_by(|a, b| b.result.net_pnl_sol.total_cmp(&a.result.net_pnl_sol));

    Ok(CompareResult { run_time, scenarios })
}

// ============================================================
// REPORTING
// ============================================================

pub fn print_backtest_report(result: &BacktestResult) {
    println!("\n{}", "═".repeat(60));
    println!("  BACKTEST RESULTS — {}", result.run_time.format("%Y-%m-%d %H:%M UTC"));
    println!("{}", "═".repeat(60));
    println!("  Config: TP={:.0}% | SL={:.0}% | Trail@{:.0}% dist{:.0}% | MinScore={:.0}",
        result.config_tp, result.config_sl,
        result.config_trailing_start, result.config_trailing_dist,
        result.config_min_score);
    println!("{}", "─".repeat(60));
    println!("  Tokens analyzed : {}", result.tokens_analyzed);
    println!("  Tokens bought   : {} | Skipped: {}", result.tokens_bought, result.tokens_skipped);
    println!("  Win / Loss / BE : {} / {} / {}", result.winning_trades, result.losing_trades, result.breakeven_trades);
    println!("  Win Rate        : {:.1}%", result.win_rate_pct);
    println!("  Profit Factor   : {:.2}", result.profit_factor);
    println!("{}", "─".repeat(60));
    println!("  Total Profit    : +{:.5} SOL", result.total_profit_sol);
    println!("  Total Loss      : -{:.5} SOL", result.total_loss_sol);
    println!("  Net P&L         : {}{:.5} SOL", if result.net_pnl_sol >= 0.0 { "+" } else { "" }, result.net_pnl_sol);
    println!("  ROI             : {}{:.2}%", if result.roi_pct >= 0.0 { "+" } else { "" }, result.roi_pct);
    println!("  SOL Deployed    : {:.4} SOL", result.total_sol_deployed);
    println!("{}", "─".repeat(60));
    println!("  Best Trade      : +{:.1}% ({})", result.best_trade_pct, result.best_trade_symbol);
    println!("  Worst Trade     : {:.1}% ({})", result.worst_trade_pct, result.worst_trade_symbol);
    println!("  Avg Profit      : +{:.1}%", result.avg_profit_pct);
    println!("  Avg Loss        : {:.1}%", result.avg_loss_pct);
    println!("  Exits           : {}", result.avg_hold_periods);
    println!("{}", "═".repeat(60));

    if !result.skip_reasons.is_empty() {
        println!("\n  Skip Reasons:");
        let mut reasons: Vec<_> = result.skip_reasons.iter().collect();
        reasons.sort_by(|a, b| b.1.cmp(a.1));
        for (reason, count) in reasons.iter().take(5) {
            println!("    • {count} × {reason}");
        }
    }
}

pub fn print_compare_table(result: &CompareResult) {
    println!("\n{}", "═".repeat(80));
    println!("  STRATEGY COMPARISON — {}", result.run_time.format("%Y-%m-%d %H:%M UTC"));
    println!("{}", "═".repeat(80));
    println!("  {:<15} {:>8} {:>8} {:>8} {:>10} {:>8}",
        "Strategy", "WinRate", "PF", "ROI%", "NetPnL", "Trades");
    println!("{}", "─".repeat(80));
    for s in &result.scenarios {
        println!("  {:<15} {:>7.1}% {:>8.2} {:>7.1}% {:>+10.5} {:>8}",
            s.name,
            s.result.win_rate_pct,
            s.result.profit_factor,
            s.result.roi_pct,
            s.result.net_pnl_sol,
            s.result.tokens_bought,
        );
    }
    println!("{}", "═".repeat(80));
}

pub fn format_backtest_telegram(result: &BacktestResult) -> String {
    format!(
        "📊 **BACKTEST RESULTS**\n\
        ═══════════════════════════════\n\
        ⚙️ Config: TP={:.0}% | SL={:.0}% | Score≥{:.0}\n\n\
        📈 Tokens analyzed: **{}**\n\
        🛒 Traded: **{}** | Skipped: **{}**\n\
        ✅ Win: **{}** | ❌ Loss: **{}** | ➖ BE: **{}**\n\
        📊 Win Rate: **{:.1}%** | Profit Factor: **{:.2}**\n\n\
        💰 Net P&L: **{}{:.5} SOL**\n\
        📈 ROI: **{}{:.2}%**\n\
        🥇 Best: **+{:.1}%** ({}) | 💔 Worst: **{:.1}%** ({})",
        result.config_tp, result.config_sl, result.config_min_score,
        result.tokens_analyzed,
        result.tokens_bought, result.tokens_skipped,
        result.winning_trades, result.losing_trades, result.breakeven_trades,
        result.win_rate_pct, result.profit_factor,
        if result.net_pnl_sol >= 0.0 { "+" } else { "" }, result.net_pnl_sol,
        if result.roi_pct >= 0.0 { "+" } else { "" }, result.roi_pct,
        result.best_trade_pct, result.best_trade_symbol,
        result.worst_trade_pct, result.worst_trade_symbol,
    )
}

pub fn format_compare_telegram(result: &CompareResult) -> String {
    let mut msg = format!(
        "📊 **STRATEGY COMPARISON**\n\
        ═══════════════════════════════\n\
        🕐 {}\n\n",
        result.run_time.format("%Y-%m-%d %H:%M UTC")
    );

    for (i, s) in result.scenarios.iter().enumerate() {
        let medal = match i { 0 => "🥇", 1 => "🥈", 2 => "🥉", _ => "  " };
        msg.push_str(&format!(
            "{} **{}**: Win {:.1}% | PF {:.2} | Net {}{:.5} SOL\n",
            medal, s.name,
            s.result.win_rate_pct,
            s.result.profit_factor,
            if s.result.net_pnl_sol >= 0.0 { "+" } else { "" },
            s.result.net_pnl_sol,
        ));
    }

    msg
}

pub fn save_backtest_result(result: &BacktestResult) -> Result<(), Box<dyn std::error::Error>> {
    let filename = format!("backtest_{}.json", result.run_time.format("%Y%m%d_%H%M%S"));
    let json = serde_json::to_string_pretty(result)?;
    std::fs::write(&filename, json)?;
    println!("[BACKTEST] Results saved to {filename}");
    Ok(())
}

pub fn save_compare_result(result: &CompareResult) -> Result<(), Box<dyn std::error::Error>> {
    let filename = format!("compare_{}.json", result.run_time.format("%Y%m%d_%H%M%S"));
    let json = serde_json::to_string_pretty(result)?;
    std::fs::write(&filename, json)?;
    println!("[COMPARE] Results saved to {filename}");
    Ok(())
}
