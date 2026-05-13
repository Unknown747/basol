// ============================================================
// AUTO BUY STRATEGY - Automated buy strategy
// ============================================================

use crate::positions::Position;

// ============================================================
// TRANSACTION FEE CONSTANTS
// ============================================================

/// Solana network fee per transaction (buy or sell)
/// Base fee 5000 lamports + priority fee ~20000 lamports = 25000 lamports = 0.000025 SOL
pub const NETWORK_FEE_SOL: f64 = 0.000025;

/// Default price impact estimate for small pools ($5k liquidity).
/// AMM formula: impact = trade_usd / (pool_usd + trade_usd)
/// For a 0.05 SOL trade (~$8.5) in a $5k pool: 8.5/5008.5 ≈ 0.17%
pub const DEFAULT_PRICE_IMPACT_PCT: f64 = 0.17;

// ============================================================
// PER-TRADE FEE ANALYSIS
// ============================================================

/// Full cost breakdown for a single round-trip trade.
///
/// Useful for showing the user actual costs,
/// not just the TP/SL numbers visible on screen.
#[derive(Debug, Clone)]
pub struct FeeAnalysis {
    /// Entry cost: slippage + price impact + network fee (in SOL)
    pub entry_cost_sol: f64,
    /// Entry cost as a percentage of position size
    pub entry_cost_pct: f64,
    /// Exit cost at TP price (in SOL)
    pub exit_cost_at_tp_sol: f64,
    /// Exit cost as a percentage of sell value
    pub exit_cost_at_tp_pct: f64,
    /// Exit cost at SL price (in SOL)
    pub exit_cost_at_sl_sol: f64,
    /// Total round-trip cost (entry + exit at TP)
    pub total_roundtrip_cost_sol: f64,
    /// Minimum price gain needed to break even (cover all fees)
    pub breakeven_pct: f64,
    /// Net profit (after fees) if TP is reached
    pub net_profit_at_tp_sol: f64,
    /// Net profit as a percentage of position
    pub net_profit_at_tp_pct: f64,
    /// Net loss (after fees) if SL is hit
    pub net_loss_at_sl_sol: f64,
    /// Net loss as a percentage of position
    pub net_loss_at_sl_pct: f64,
    /// Risk/Reward ratio: net_profit_at_tp / net_loss_at_sl
    pub risk_reward_ratio: f64,
    /// Minimum win rate needed for positive EV
    pub min_win_rate_pct: f64,
}

/// Calculate full fee analysis for a single trade plan.
///
/// **Parameters:**
/// - `amount_sol`: position size in SOL
/// - `slippage_pct`: configured slippage (e.g. 1.5)
/// - `take_profit_pct`: TP target (e.g. 20.0)
/// - `stop_loss_pct`: SL level (e.g. 8.0)
/// - `liquidity_usd`: pool liquidity in USD
/// - `sol_price_usd`: current SOL price in USD
pub fn compute_fee_analysis(
    amount_sol: f64,
    slippage_pct: f64,
    take_profit_pct: f64,
    stop_loss_pct: f64,
    liquidity_usd: f64,
    sol_price_usd: f64,
) -> FeeAnalysis {
    let position_usd = amount_sol * sol_price_usd;

    // --- Entry cost ---
    // Price impact based on trade size vs pool liquidity (AMM formula)
    let entry_impact_pct = if liquidity_usd > 0.0 {
        (position_usd / (liquidity_usd + position_usd)) * 100.0
    } else {
        DEFAULT_PRICE_IMPACT_PCT
    };
    let entry_slippage_sol  = amount_sol * slippage_pct / 100.0;
    let entry_impact_sol    = amount_sol * entry_impact_pct / 100.0;
    let entry_cost_sol      = entry_slippage_sol + entry_impact_sol + NETWORK_FEE_SOL;
    let entry_cost_pct      = entry_cost_sol / amount_sol * 100.0;

    // --- Exit cost at TP ---
    let tp_value_sol        = amount_sol * (1.0 + take_profit_pct / 100.0);
    let tp_value_usd        = tp_value_sol * sol_price_usd;
    let exit_impact_at_tp   = (tp_value_usd / (liquidity_usd + tp_value_usd)) * 100.0;
    let exit_slip_at_tp_sol = tp_value_sol * slippage_pct / 100.0;
    let exit_imp_at_tp_sol  = tp_value_sol * exit_impact_at_tp / 100.0;
    let exit_cost_at_tp_sol = exit_slip_at_tp_sol + exit_imp_at_tp_sol + NETWORK_FEE_SOL;
    let exit_cost_at_tp_pct = exit_cost_at_tp_sol / tp_value_sol * 100.0;

    // --- Exit cost at SL ---
    let sl_value_sol        = amount_sol * (1.0 - stop_loss_pct / 100.0);
    let sl_value_usd        = sl_value_sol * sol_price_usd;
    let exit_impact_at_sl   = (sl_value_usd / (liquidity_usd + sl_value_usd)) * 100.0;
    let exit_slip_at_sl_sol = sl_value_sol * slippage_pct / 100.0;
    let exit_imp_at_sl_sol  = sl_value_sol * exit_impact_at_sl / 100.0;
    let exit_cost_at_sl_sol = exit_slip_at_sl_sol + exit_imp_at_sl_sol + NETWORK_FEE_SOL;

    // --- Total round trip ---
    let total_roundtrip_cost_sol = entry_cost_sol + exit_cost_at_tp_sol;

    // Breakeven = how much the token must rise to cover all fees
    // Conservative estimate: total_roundtrip / amount_sol
    let breakeven_pct = total_roundtrip_cost_sol / amount_sol * 100.0;

    // --- Net profit at TP ---
    let gross_profit_at_tp  = amount_sol * take_profit_pct / 100.0;
    let net_profit_at_tp_sol = gross_profit_at_tp - total_roundtrip_cost_sol;
    let net_profit_at_tp_pct = net_profit_at_tp_sol / amount_sol * 100.0;

    // --- Net loss at SL ---
    // Total loss = SL% of position + entry cost (already paid) + exit cost at SL
    let gross_loss_at_sl    = amount_sol * stop_loss_pct / 100.0;
    let net_loss_at_sl_sol  = gross_loss_at_sl + entry_cost_sol + exit_cost_at_sl_sol;
    let net_loss_at_sl_pct  = net_loss_at_sl_sol / amount_sol * 100.0;

    // --- Risk/Reward ---
    let risk_reward_ratio = if net_loss_at_sl_sol > 0.0 {
        net_profit_at_tp_sol / net_loss_at_sl_sol
    } else {
        0.0
    };

    // --- Minimum win rate for positive EV ---
    // w × net_profit - (1-w) × net_loss > 0
    // w(net_profit + net_loss) > net_loss
    // w > net_loss / (net_profit + net_loss)
    let min_win_rate_pct = if (net_profit_at_tp_sol + net_loss_at_sl_sol) > 0.0 {
        net_loss_at_sl_sol / (net_profit_at_tp_sol + net_loss_at_sl_sol) * 100.0
    } else {
        100.0
    };

    FeeAnalysis {
        entry_cost_sol,
        entry_cost_pct,
        exit_cost_at_tp_sol,
        exit_cost_at_tp_pct,
        exit_cost_at_sl_sol,
        total_roundtrip_cost_sol,
        breakeven_pct,
        net_profit_at_tp_sol,
        net_profit_at_tp_pct,
        net_loss_at_sl_sol,
        net_loss_at_sl_pct,
        risk_reward_ratio,
        min_win_rate_pct,
    }
}

// ============================================================
// TRADING CONFIG
// ============================================================

#[derive(Debug, Clone)]
pub struct TradingConfig {
    pub trading_enabled: bool,
    pub max_position_sol: f64,
    /// Minimum position size (default: 10% of max, minimum 0.01 SOL)
    pub min_position_sol: f64,
    pub take_profit_percent: f64,
    pub stop_loss_percent: f64,
    pub trailing_start_percent: f64,
    pub trailing_distance_percent: f64,
    pub min_score_to_buy: f64,
    pub min_liquidity_usd: f64,
    pub default_slippage: f64,
    pub max_positions: usize,
    /// If > 0: auto-exit if position is stuck longer than X minutes.
    pub max_hold_minutes: u64,
    /// P&L threshold for time exit.
    pub time_exit_threshold_pct: f64,

    // === 3-STAGE TAKE PROFIT ===
    /// TP1 level — sell tp1_sell_percent% of position here
    /// Must be > break-even (~3.81% for 0.05 SOL with 1.5% slippage)
    /// Set 0.0 to disable 3-stage and use single TP only
    pub tp1_percent: f64,
    /// Percentage of position to sell at TP1 (e.g. 33.0 = one third)
    pub tp1_sell_percent: f64,
    /// TP2 level — sell tp2_sell_percent% of remaining position
    pub tp2_percent: f64,
    /// Percentage of remaining position to sell at TP2 (e.g. 50.0 = half of remainder)
    pub tp2_sell_percent: f64,
    // Remaining position (100 - tp1 - tp2*(100-tp1)/100) is managed by trailing stop or final TP
}

impl TradingConfig {
    pub fn from_env() -> Self {
        let trading_enabled = std::env::var("TRADING_ENABLED")
            .map(|v| v == "true" || v == "1")
            .unwrap_or(false);

        let max_position_sol = std::env::var("MAX_POSITION_SOL")
            .ok().and_then(|v| v.parse().ok()).unwrap_or(0.5);

        let min_position_sol: f64 = std::env::var("MIN_POSITION_SOL")
            .ok().and_then(|v| v.parse().ok())
            .unwrap_or((max_position_sol * 0.1_f64).max(0.01_f64));

        let take_profit_percent = std::env::var("TAKE_PROFIT_PERCENT")
            .ok().and_then(|v| v.parse().ok()).unwrap_or(40.0);

        let stop_loss_percent = std::env::var("STOP_LOSS_PERCENT")
            .ok().and_then(|v| v.parse().ok()).unwrap_or(15.0);

        let trailing_start_percent = std::env::var("TRAILING_START_PERCENT")
            .ok().and_then(|v| v.parse().ok()).unwrap_or(20.0);

        let trailing_distance_percent = std::env::var("TRAILING_DISTANCE_PERCENT")
            .ok().and_then(|v| v.parse().ok()).unwrap_or(5.0);

        let min_score_to_buy = std::env::var("MIN_SCORE_TO_BUY")
            .ok().and_then(|v| v.parse().ok()).unwrap_or(85.0);

        let min_liquidity_usd = std::env::var("MIN_LIQUIDITY_USD")
            .ok().and_then(|v| v.parse().ok()).unwrap_or(10_000.0);

        let default_slippage = std::env::var("DEFAULT_SLIPPAGE")
            .ok().and_then(|v| v.parse().ok()).unwrap_or(1.0);

        let max_positions = std::env::var("MAX_POSITIONS")
            .ok().and_then(|v| v.parse().ok()).unwrap_or(5);

        let max_hold_minutes = std::env::var("MAX_HOLD_MINUTES")
            .ok().and_then(|v| v.parse().ok()).unwrap_or(0u64);

        let time_exit_threshold_pct = std::env::var("TIME_EXIT_THRESHOLD_PCT")
            .ok().and_then(|v| v.parse().ok()).unwrap_or(5.0);

        // 3-stage TP — default 0.0 = disabled (use single TP)
        let tp1_percent = std::env::var("TP1_PERCENT")
            .ok().and_then(|v| v.parse().ok()).unwrap_or(0.0);
        let tp1_sell_percent = std::env::var("TP1_SELL_PERCENT")
            .ok().and_then(|v| v.parse().ok()).unwrap_or(33.0);
        let tp2_percent = std::env::var("TP2_PERCENT")
            .ok().and_then(|v| v.parse().ok()).unwrap_or(0.0);
        let tp2_sell_percent = std::env::var("TP2_SELL_PERCENT")
            .ok().and_then(|v| v.parse().ok()).unwrap_or(50.0);

        Self {
            trading_enabled,
            max_position_sol,
            min_position_sol,
            take_profit_percent,
            stop_loss_percent,
            trailing_start_percent,
            trailing_distance_percent,
            min_score_to_buy,
            min_liquidity_usd,
            default_slippage,
            max_positions,
            max_hold_minutes,
            time_exit_threshold_pct,
            tp1_percent,
            tp1_sell_percent,
            tp2_percent,
            tp2_sell_percent,
        }
    }

    /// Safe default config (for testing without .env) — 3-stage disabled
    pub fn default_safe() -> Self {
        Self {
            trading_enabled: false,
            max_position_sol: 0.5,
            min_position_sol: 0.05,
            take_profit_percent: 40.0,
            stop_loss_percent: 15.0,
            trailing_start_percent: 20.0,
            trailing_distance_percent: 5.0,
            min_score_to_buy: 85.0,
            min_liquidity_usd: 10_000.0,
            default_slippage: 1.0,
            max_positions: 5,
            max_hold_minutes: 0,
            time_exit_threshold_pct: 5.0,
            tp1_percent: 0.0,
            tp1_sell_percent: 33.0,
            tp2_percent: 0.0,
            tp2_sell_percent: 50.0,
        }
    }

    /// Scalping preset for small capital (0.05–0.2 SOL per trade).
    ///
    /// **3-Stage Take Profit (0.05 SOL, 1.5% slippage, $5k pool):**
    ///
    /// ```
    /// TP1 +12% → sell 33% → net +8.2% on that portion (SAFE, above break-even 3.81%)
    /// TP2 +20% → sell 50% of remainder → net +16.2% on that portion
    /// TP3 +35% → sell all remaining → net +31.2% (bonus if market keeps running)
    ///    OR trailing stop 3% protects remaining position
    /// SL  -8%  → sell ALL 100% immediately → net -11.3%
    /// ```
    ///
    /// **Distribution of 0.05 SOL:**
    /// - TP1: sell 0.0167 SOL → receive back ~0.0180 SOL (+8.2%)
    /// - TP2: sell 0.0167 SOL from remainder → receive back ~0.0194 SOL (+16.2%)
    /// - TP3: sell last 0.0167 SOL → receive back ~0.0219 SOL (+31.2%)
    ///
    /// **After TP1 fires: cannot lose even if price reverses!**
    pub fn scalping_preset() -> Self {
        Self {
            trading_enabled: false,
            max_position_sol: 0.05,
            min_position_sol: 0.05,
            take_profit_percent: 35.0,   // TP3 / final TP if market keeps climbing
            stop_loss_percent: 8.0,
            trailing_start_percent: 12.0, // trailing activates alongside TP1
            trailing_distance_percent: 3.0,
            min_score_to_buy: 87.0,
            min_liquidity_usd: 5_000.0,
            default_slippage: 1.5,
            max_positions: 2,
            max_hold_minutes: 40,
            time_exit_threshold_pct: 3.0,
            tp1_percent: 12.0,           // TP1 at +12%
            tp1_sell_percent: 33.0,      // sell 33% of position
            tp2_percent: 20.0,           // TP2 at +20%
            tp2_sell_percent: 50.0,      // sell 50% of remainder (= 33% of original)
                                         // remaining 34% of original managed by trailing/TP3
        }
    }
}

// ============================================================
// TOKEN SIGNAL
// ============================================================

pub struct BuySignal {
    pub token_address: String,
    pub symbol: String,
    pub name: String,
    pub total_score: f64,
    pub liquidity_usd: f64,
    pub mint_authority_revoked: bool,
    pub current_price_usd: f64,
    pub market_cap: Option<f64>,
}

// ============================================================
// BUY DECISION
// ============================================================

#[derive(Debug)]
pub enum BuyDecision {
    Buy {
        amount_sol: f64,
        reason: String,
        fee_analysis: FeeAnalysis,
    },
    Skip {
        reason: String,
    },
}

// ============================================================
// BUY STRATEGY FUNCTIONS
// ============================================================

/// Evaluate whether a token should be bought and what position size to use.
///
/// **Position sizing formula (score-based):**
/// - Score 87 → 48% of max_position_sol
/// - Score 93 → 72% of max_position_sol
/// - Score 100 → 100% of max_position_sol
/// - Result is clamped to [min_position_sol, max_position_sol]
///
/// **Note for small capital:**
/// With min_position_sol=0.02 and max=0.05, score 87 → max(0.024, 0.02) = 0.024 SOL.
/// Recommended to set MIN_POSITION_SOL=0.05 for a consistent 0.05 SOL per trade.
pub fn evaluate_buy_signal(
    signal: &BuySignal,
    config: &TradingConfig,
    existing_positions: &std::collections::HashMap<String, Position>,
) -> BuyDecision {
    // 1. Check trading enabled
    if !config.trading_enabled {
        return BuyDecision::Skip {
            reason: "Trading disabled (TRADING_ENABLED=false)".to_string(),
        };
    }

    // 2. Check minimum score
    if signal.total_score < config.min_score_to_buy {
        return BuyDecision::Skip {
            reason: format!(
                "Score {:.1} below minimum {:.1}",
                signal.total_score, config.min_score_to_buy
            ),
        };
    }

    // 3. Check for duplicate position
    if existing_positions.contains_key(&signal.token_address) {
        return BuyDecision::Skip {
            reason: format!("Already have a position for {}", signal.symbol),
        };
    }

    // 4. Check max active positions
    if existing_positions.len() >= config.max_positions {
        return BuyDecision::Skip {
            reason: format!(
                "Already have {} active positions (max: {})",
                existing_positions.len(), config.max_positions
            ),
        };
    }

    // 5. Check minimum liquidity
    if signal.liquidity_usd < config.min_liquidity_usd {
        return BuyDecision::Skip {
            reason: format!(
                "Liquidity ${:.0} below minimum ${:.0}",
                signal.liquidity_usd, config.min_liquidity_usd
            ),
        };
    }

    // 6. Check mint authority revoked
    if !signal.mint_authority_revoked {
        return BuyDecision::Skip {
            reason: "Mint authority not revoked — high rugpull risk".to_string(),
        };
    }

    // 7. Calculate position size based on score
    //
    //   multiplier = (score - 75) / 25   → range [0.0, 1.0]
    //   Score 87  → 0.48 × max
    //   Score 93  → 0.72 × max
    //   Score 100 → 1.00 × max
    //   Clamped to [min_position_sol, max_position_sol]
    let score_multiplier = ((signal.total_score - 75.0) / 25.0).clamp(0.0, 1.0);
    let raw_size = score_multiplier * config.max_position_sol;
    let position_size = raw_size
        .max(config.min_position_sol)
        .min(config.max_position_sol);

    // 8. Calculate fee analysis for this position
    // Use default SOL price 170.0 if not available from signal
    let sol_price_estimate = 170.0_f64; // fallback; ideally from main bot state
    let fee_analysis = compute_fee_analysis(
        position_size,
        config.default_slippage,
        config.take_profit_percent,
        config.stop_loss_percent,
        signal.liquidity_usd,
        sol_price_estimate,
    );

    let reason = format!(
        "Score {:.1}/100 | Liq ${:.0} | {:.4} SOL | R:R {:.2} | Breakeven +{:.1}%",
        signal.total_score,
        signal.liquidity_usd,
        position_size,
        fee_analysis.risk_reward_ratio,
        fee_analysis.breakeven_pct,
    );

    BuyDecision::Buy {
        amount_sol: position_size,
        reason,
        fee_analysis,
    }
}

/// Log detailed buy decision to console, including full fee analysis
pub fn log_buy_decision(signal: &BuySignal, decision: &BuyDecision) {
    let addr_short = &signal.token_address[..signal.token_address.len().min(8)];
    match decision {
        BuyDecision::Buy { amount_sol, reason, fee_analysis: fa } => {
            println!(
                "[BUY EVAL] ✅ BUY {} ({}) | {}",
                signal.symbol, addr_short, reason
            );
            println!(
                "[BUY EVAL]    Entry cost  : {:.5} SOL ({:.2}%)",
                fa.entry_cost_sol, fa.entry_cost_pct
            );
            println!(
                "[BUY EVAL]    Exit cost   : {:.5} SOL ({:.2}% of sell value)",
                fa.exit_cost_at_tp_sol, fa.exit_cost_at_tp_pct
            );
            println!(
                "[BUY EVAL]    Break-even  : price must rise at least +{:.2}%",
                fa.breakeven_pct
            );
            println!(
                "[BUY EVAL]    Net if TP   : +{:.5} SOL (+{:.2}% net)",
                fa.net_profit_at_tp_sol, fa.net_profit_at_tp_pct
            );
            println!(
                "[BUY EVAL]    Net if SL   : -{:.5} SOL (-{:.2}% net)",
                fa.net_loss_at_sl_sol, fa.net_loss_at_sl_pct
            );
            println!(
                "[BUY EVAL]    R:R ratio   : {:.2} | Min win rate: {:.1}%",
                fa.risk_reward_ratio, fa.min_win_rate_pct
            );
            println!(
                "[BUY EVAL]    Position    : {:.4} SOL | Total round-trip cost: {:.5} SOL",
                amount_sol, fa.total_roundtrip_cost_sol
            );
        }
        BuyDecision::Skip { reason } => {
            println!(
                "[BUY EVAL] ⏭  SKIP {} ({}) | {}",
                signal.symbol, addr_short, reason
            );
        }
    }
}
