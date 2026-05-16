// ============================================================
// PAPER TRADING - Simulated trading without real money
// Use to test strategies before going live
// ============================================================

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ============================================================
// MAINNET CONSTANTS - identical to real transaction costs
// ============================================================

/// Solana network fee per transaction (base fee 5000 lamports + priority fee ~20000 lamports)
/// Total ~25000 lamports = 0.000025 SOL per tx — conservative/realistic figure
pub const NETWORK_FEE_SOL: f64 = 0.000025;

// ============================================================
// CONFIG
// ============================================================

pub struct PaperConfig {
    pub enabled: bool,
    pub virtual_balance_sol: f64,
    pub max_position_sol: f64,
    pub take_profit_percent: f64,
    pub stop_loss_percent: f64,
    pub trailing_start_percent: f64,
    pub trailing_distance_percent: f64,
    pub min_score_to_buy: f64,
    pub min_liquidity_usd: f64,
    pub default_slippage: f64,
    pub max_positions: usize,
    pub report_interval_secs: u64,
}

impl PaperConfig {
    pub fn from_env() -> Self {
        Self {
            enabled: std::env::var("PAPER_TRADING_ENABLED")
                .map(|v| v == "true" || v == "1")
                .unwrap_or(false),
            virtual_balance_sol: std::env::var("PAPER_BALANCE_SOL")
                .ok().and_then(|v| v.parse().ok()).unwrap_or(10.0),
            max_position_sol: std::env::var("MAX_POSITION_SOL")
                .ok().and_then(|v| v.parse().ok()).unwrap_or(0.5),
            take_profit_percent: std::env::var("TAKE_PROFIT_PERCENT")
                .ok().and_then(|v| v.parse().ok()).unwrap_or(40.0),
            stop_loss_percent: std::env::var("STOP_LOSS_PERCENT")
                .ok().and_then(|v| v.parse().ok()).unwrap_or(15.0),
            trailing_start_percent: std::env::var("TRAILING_START_PERCENT")
                .ok().and_then(|v| v.parse().ok()).unwrap_or(20.0),
            trailing_distance_percent: std::env::var("TRAILING_DISTANCE_PERCENT")
                .ok().and_then(|v| v.parse().ok()).unwrap_or(5.0),
            min_score_to_buy: std::env::var("MIN_SCORE_TO_BUY")
                .ok().and_then(|v| v.parse().ok()).unwrap_or(85.0),
            min_liquidity_usd: std::env::var("MIN_LIQUIDITY_USD")
                .ok().and_then(|v| v.parse().ok()).unwrap_or(10_000.0),
            // Default slippage matches live trading configuration
            default_slippage: std::env::var("DEFAULT_SLIPPAGE")
                .ok().and_then(|v| v.parse().ok()).unwrap_or(1.0),
            max_positions: std::env::var("MAX_POSITIONS")
                .ok().and_then(|v| v.parse().ok()).unwrap_or(5),
            report_interval_secs: std::env::var("PAPER_REPORT_INTERVAL_SECS")
                .ok().and_then(|v| v.parse().ok()).unwrap_or(3600),
        }
    }
}

// ============================================================
// PAPER POSITION - Simulated position
// ============================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaperPosition {
    pub token_address: String,
    pub symbol: String,
    pub name: String,
    pub buy_price_usd: f64,
    /// SOL still held (reduced on partial sell)
    pub amount_sol: f64,
    /// Token amount still held (reduced on partial sell)
    pub token_amount: f64,
    pub highest_price: f64,
    pub trailing_stop_active: bool,
    pub trailing_stop_price: f64,
    pub entry_time: DateTime<Utc>,
    pub score_at_entry: f64,
    pub liquidity_at_entry: f64,
    /// TP1 partial already executed?
    pub tp1_fired: bool,
    /// TP2 partial already executed?
    pub tp2_fired: bool,
}

impl PaperPosition {
    pub fn profit_percent(&self, current_price: f64) -> f64 {
        if self.buy_price_usd == 0.0 { return 0.0; }
        (current_price - self.buy_price_usd) / self.buy_price_usd * 100.0
    }

    pub fn profit_sol(&self, current_price: f64) -> f64 {
        self.amount_sol * self.profit_percent(current_price) / 100.0
    }

    pub fn age_minutes(&self) -> i64 {
        Utc::now().signed_duration_since(self.entry_time).num_minutes()
    }
}

// ============================================================
// PAPER TRADE HISTORY
// ============================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TradeResult {
    Profit,
    Loss,
    BreakEven,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaperTrade {
    pub token_address: String,
    pub symbol: String,
    pub name: String,
    pub buy_price: f64,
    pub sell_price: f64,
    pub amount_sol: f64,
    pub profit_percent: f64,
    pub profit_sol: f64,
    pub buy_time: DateTime<Utc>,
    pub sell_time: DateTime<Utc>,
    pub hold_duration_minutes: i64,
    pub exit_reason: String,
    pub score_at_entry: f64,
    pub result: TradeResult,
}

// ============================================================
// PAPER TRADING ENGINE
// ============================================================

#[derive(Debug, Serialize, Deserialize)]
pub struct PaperTradingState {
    pub initial_balance_sol: f64,
    pub current_balance_sol: f64,
    pub positions: HashMap<String, PaperPosition>,
    pub closed_trades: Vec<PaperTrade>,
    pub total_buys: u32,
    pub total_sells: u32,
    pub winning_trades: u32,
    pub losing_trades: u32,
    pub total_profit_sol: f64,
    pub total_loss_sol: f64,
    pub best_trade_pct: f64,
    pub worst_trade_pct: f64,
    pub best_trade_symbol: String,
    pub worst_trade_symbol: String,
    pub start_time: DateTime<Utc>,
}

impl PaperTradingState {
    pub fn new(initial_balance: f64) -> Self {
        Self {
            initial_balance_sol: initial_balance,
            current_balance_sol: initial_balance,
            positions: HashMap::new(),
            closed_trades: Vec::new(),
            total_buys: 0,
            total_sells: 0,
            winning_trades: 0,
            losing_trades: 0,
            total_profit_sol: 0.0,
            total_loss_sol: 0.0,
            best_trade_pct: 0.0,
            worst_trade_pct: 0.0,
            best_trade_symbol: String::new(),
            worst_trade_symbol: String::new(),
            start_time: Utc::now(),
        }
    }

    /// Calculate total equity (balance + value of open positions)
    pub fn total_equity(&self, current_prices: &HashMap<String, f64>) -> f64 {
        let open_pnl: f64 = self.positions.values()
            .map(|pos| {
                let price = current_prices.get(&pos.token_address).copied().unwrap_or(pos.buy_price_usd);
                pos.amount_sol + pos.profit_sol(price)
            })
            .sum();
        self.current_balance_sol + open_pnl
    }

    /// Overall Return on Investment
    pub fn roi_percent(&self, current_prices: &HashMap<String, f64>) -> f64 {
        if self.initial_balance_sol == 0.0 { return 0.0; }
        (self.total_equity(current_prices) - self.initial_balance_sol)
            / self.initial_balance_sol * 100.0
    }

    /// Win rate based on closed trades
    pub fn win_rate(&self) -> f64 {
        let total = self.winning_trades + self.losing_trades;
        if total == 0 { return 0.0; }
        self.winning_trades as f64 / total as f64 * 100.0
    }

    /// Profit factor (gross profit / gross loss). Capped at 99.99 — f64::INFINITY
    /// breaks JSON serialization (serde_json encodes it as `null`).
    pub fn profit_factor(&self) -> f64 {
        if self.total_loss_sol == 0.0 {
            return if self.total_profit_sol > 0.0 { 99.99 } else { 0.0 };
        }
        (self.total_profit_sol / self.total_loss_sol).min(99.99)
    }

    /// Calculate price impact using constant product AMM formula (xy=k)
    /// Same model used by Jupiter for Solana pools
    /// amount_sol: SOL invested
    /// liquidity_usd: total pool liquidity in USD
    /// sol_price_usd: current SOL price
    pub fn calc_price_impact_pct(amount_sol: f64, liquidity_usd: f64, sol_price_usd: f64) -> f64 {
        if liquidity_usd <= 0.0 || sol_price_usd <= 0.0 {
            return 0.0;
        }
        // SOL reserve on pool side ≈ half of total liquidity (assuming 50/50 pool)
        let sol_reserve = liquidity_usd / 2.0 / sol_price_usd;
        // AMM formula: impact = amount_in / (reserve_in + amount_in)
        let impact = amount_sol / (sol_reserve + amount_sol);
        // Cap at 50% to avoid unrealistic values
        (impact * 100.0).min(50.0)
    }

    /// Execute paper buy — 100% simulates mainnet conditions
    /// Includes: network fee, slippage, and AMM pool price impact
    pub fn execute_buy(
        &mut self,
        token_address: String,
        symbol: String,
        name: String,
        quoted_price_usd: f64,    // price visible on DEX (before slippage/impact)
        amount_sol: f64,
        slippage_percent: f64,    // configured slippage (from DEFAULT_SLIPPAGE env)
        sol_price_usd: f64,       // current SOL price
        score: f64,
        liquidity_usd: f64,
    ) -> Result<String, String> {
        // Check balance covers amount + network fee
        let total_needed = amount_sol + NETWORK_FEE_SOL;
        if self.current_balance_sol < total_needed {
            return Err(format!(
                "Insufficient virtual balance: {:.6} SOL (need {:.6} SOL including fee)",
                self.current_balance_sol, total_needed
            ));
        }

        if self.positions.contains_key(&token_address) {
            return Err(format!("Already have a position for {}", symbol));
        }

        // === MAINNET SIMULATION: Network fee ===
        self.current_balance_sol -= NETWORK_FEE_SOL;

        // === MAINNET SIMULATION: Price impact (constant product AMM) ===
        let price_impact_pct = Self::calc_price_impact_pct(amount_sol, liquidity_usd, sol_price_usd);

        // === MAINNET SIMULATION: Slippage on buy (price rises = worse for buyer) ===
        // Effective price = quoted * (1 + slippage%) * (1 + impact%)
        let total_cost_pct = slippage_percent + price_impact_pct;
        let effective_buy_price = quoted_price_usd * (1.0 + total_cost_pct / 100.0);

        // === Token amount based on effective price (not quoted price) ===
        let token_amount = if effective_buy_price > 0.0 {
            (amount_sol * sol_price_usd) / effective_buy_price
        } else {
            0.0
        };

        self.current_balance_sol -= amount_sol;
        self.total_buys += 1;

        println!(
            "[PAPER BUY] ✅ {} ({}) | {:.4} SOL @ quoted=${:.8} → effective=${:.8}\n\
             [PAPER BUY]    Slippage: {:.2}% | Price Impact: {:.2}% | Fee: {:.6} SOL | Tokens: {:.2}",
            name, symbol, amount_sol,
            quoted_price_usd, effective_buy_price,
            slippage_percent, price_impact_pct,
            NETWORK_FEE_SOL, token_amount,
        );

        let position = PaperPosition {
            token_address: token_address.clone(),
            symbol: symbol.clone(),
            name: name.clone(),
            buy_price_usd: effective_buy_price,
            amount_sol,
            token_amount,
            highest_price: effective_buy_price,
            trailing_stop_active: false,
            trailing_stop_price: 0.0,
            entry_time: Utc::now(),
            score_at_entry: score,
            liquidity_at_entry: liquidity_usd,
            tp1_fired: false,
            tp2_fired: false,
        };

        self.positions.insert(token_address.clone(), position);

        let id = if token_address.len() >= 8 { &token_address[..8] } else { &token_address };
        Ok(format!("PAPER_{}_slip{:.1}_impact{:.2}", id, slippage_percent, price_impact_pct))
    }

    /// Execute paper sell — supports partial sell (3-stage TP) and full close.
    ///
    /// - `percentage`: percentage of position to sell (1–100). 100 = full close.
    /// - `tp_stage`: 0 = full close, 1 = TP1 partial, 2 = TP2 partial.
    ///   Partial sell reduces amount_sol/token_amount and marks tp1/tp2_fired,
    ///   position remains active. Full close removes position from active list.
    pub fn execute_sell(
        &mut self,
        token_address: &str,
        quoted_sell_price: f64,
        percentage: f64,
        slippage_percent: f64,
        exit_reason: String,
        tp_stage: u8,
    ) -> Result<PaperTrade, String> {
        let is_partial = tp_stage > 0 && percentage < 100.0;

        // Snapshot needed fields (without moving ownership yet)
        let (sym, name, buy_price, pos_amount_sol, pos_tokens,
             liquidity, score, entry_time, age_min) = {
            let pos = self.positions.get(token_address)
                .ok_or_else(|| format!("Position not found: {}", token_address))?;
            (
                pos.symbol.clone(), pos.name.clone(), pos.buy_price_usd,
                pos.amount_sol, pos.token_amount,
                pos.liquidity_at_entry, pos.score_at_entry,
                pos.entry_time, pos.age_minutes(),
            )
        };

        // Calculate how much is actually being sold (% of CURRENT position)
        let sold_sol    = pos_amount_sol * percentage / 100.0;
        let sold_tokens = pos_tokens     * percentage / 100.0;

        // Price impact from the portion being sold — constant product AMM formula:
        // impact = trade_usd / (pool_usd + trade_usd). Selling into the pool
        // pushes the price down, so we calculate the fraction of pool displaced.
        let sell_value_usd = sold_tokens * quoted_sell_price;
        let sell_impact_pct = if liquidity > 0.0 {
            (sell_value_usd / (liquidity + sell_value_usd) * 100.0).min(30.0)
        } else { 0.0 };

        let total_cost_pct      = slippage_percent + sell_impact_pct;
        let effective_sell_price = quoted_sell_price * (1.0 - total_cost_pct / 100.0);

        // Profit calculated against effective buy price
        let profit_pct = if buy_price > 0.0 {
            (effective_sell_price - buy_price) / buy_price * 100.0
        } else { 0.0 };
        let profit_sol = sold_sol * profit_pct / 100.0;

        // Proceeds = (capital of sold portion + profit) - network fee
        let gross_proceeds = sold_sol + profit_sol;
        let net_proceeds   = (gross_proceeds - NETWORK_FEE_SOL).max(0.0);

        self.current_balance_sol += net_proceeds;
        self.total_sells += 1;

        // Only count win/loss on full position close (tp_stage == 0).
        // Partial sells (TP1=1, TP2=2) must NOT increment these counters, otherwise
        // a single 3-stage exit would inflate the win rate by counting 2-3 times.
        let result = if profit_pct > 0.5 {
            if tp_stage == 0 { self.winning_trades += 1; }
            self.total_profit_sol += profit_sol.max(0.0);
            TradeResult::Profit
        } else if profit_pct < -0.5 {
            if tp_stage == 0 { self.losing_trades += 1; }
            self.total_loss_sol += profit_sol.abs();
            TradeResult::Loss
        } else {
            TradeResult::BreakEven
        };

        if profit_pct > self.best_trade_pct {
            self.best_trade_pct = profit_pct;
            self.best_trade_symbol = sym.clone();
        }
        if profit_pct < self.worst_trade_pct {
            self.worst_trade_pct = profit_pct;
            self.worst_trade_symbol = sym.clone();
        }

        let stage_label = match tp_stage {
            1 => " [TP1 33%]",
            2 => " [TP2 50%rem]",
            _ => "",
        };

        println!(
            "[PAPER SELL] {} {} ({}) | {:.0}%{} | ${:.8} → ${:.8}\n\
             [PAPER SELL]    Slip: {:.2}% | Impact: {:.2}% | P&L: {}{:.1}% ({}{:.5} SOL) | Balance: {:.4} SOL",
            if profit_pct >= 0.0 { "✅" } else { "❌" },
            name, sym, percentage, stage_label,
            quoted_sell_price, effective_sell_price,
            slippage_percent, sell_impact_pct,
            if profit_pct >= 0.0 { "+" } else { "" }, profit_pct,
            if profit_sol >= 0.0 { "+" } else { "" }, profit_sol,
            self.current_balance_sol,
        );

        let trade = PaperTrade {
            token_address: token_address.to_string(),
            symbol: sym,
            name,
            buy_price,
            sell_price: effective_sell_price,
            amount_sol: sold_sol,
            profit_percent: profit_pct,
            profit_sol,
            buy_time: entry_time,
            sell_time: Utc::now(),
            hold_duration_minutes: age_min,
            exit_reason,
            score_at_entry: score,
            result,
        };
        self.closed_trades.push(trade.clone());

        if is_partial {
            // Reduce position — do not remove
            let remaining = 1.0 - percentage / 100.0;
            if let Some(pos) = self.positions.get_mut(token_address) {
                pos.amount_sol   *= remaining;
                pos.token_amount *= remaining;
                match tp_stage {
                    1 => pos.tp1_fired = true,
                    2 => pos.tp2_fired = true,
                    _ => {}
                }
            }
        } else {
            // Full close — remove position
            self.positions.remove(token_address);
        }

        Ok(trade)
    }

    /// Evaluate all positions — supports 3-stage TP, SL, trailing stop, time exit.
    ///
    /// Returns: `Vec<(addr, reason, price, sell_percent, tp_stage)>`
    /// - `sell_percent`: percentage of position to sell (1–100)
    /// - `tp_stage`: 0 = full close, 1 = TP1, 2 = TP2
    #[allow(clippy::too_many_arguments)]
    pub fn evaluate_positions(
        &mut self,
        prices: &HashMap<String, f64>,
        take_profit: f64,
        stop_loss: f64,
        trailing_start: f64,
        trailing_distance: f64,
        tp1_pct: f64,
        tp1_sell_pct: f64,
        tp2_pct: f64,
        tp2_sell_pct: f64,
        max_hold_minutes: u64,
        time_exit_threshold: f64,
    ) -> Vec<(String, String, f64, f64, u8)> {
        let mut to_sell: Vec<(String, String, f64, f64, u8)> = Vec::new();

        for (addr, pos) in self.positions.iter_mut() {
            let current_price = match prices.get(addr) {
                Some(&p) => p,
                None => continue,
            };

            if current_price > pos.highest_price {
                pos.highest_price = current_price;
            }

            let profit_pct  = pos.profit_percent(current_price);
            let age_minutes = pos.age_minutes();

            // 1. STOP LOSS — cut loss, sell ALL
            if profit_pct <= -stop_loss {
                to_sell.push((
                    addr.clone(),
                    format!("Stop Loss {:.1}%", profit_pct),
                    current_price, 100.0, 0,
                ));
                continue;
            }

            // 2. TP1 PARTIAL — sell tp1_sell_pct% if not yet fired
            if tp1_pct > 0.0 && !pos.tp1_fired && profit_pct >= tp1_pct {
                println!(
                    "[PAPER TP1] 🎯 {} - profit +{:.1}% >= +{:.1}% | Sell {:.0}%",
                    pos.symbol, profit_pct, tp1_pct, tp1_sell_pct
                );
                to_sell.push((
                    addr.clone(),
                    format!("TP1 Partial {:.0}% @ +{:.1}%", tp1_sell_pct, profit_pct),
                    current_price, tp1_sell_pct, 1,
                ));
                continue;
            }

            // 3. TP2 PARTIAL — sell tp2_sell_pct% of remainder if TP1 already fired
            if tp2_pct > 0.0 && pos.tp1_fired && !pos.tp2_fired && profit_pct >= tp2_pct {
                println!(
                    "[PAPER TP2] 🎯 {} - profit +{:.1}% >= +{:.1}% | Sell {:.0}% of remainder",
                    pos.symbol, profit_pct, tp2_pct, tp2_sell_pct
                );
                to_sell.push((
                    addr.clone(),
                    format!("TP2 Partial {:.0}% @ +{:.1}%", tp2_sell_pct, profit_pct),
                    current_price, tp2_sell_pct, 2,
                ));
                continue;
            }

            // 4. TRAILING STOP — protect remaining position
            if profit_pct >= trailing_start {
                if !pos.trailing_stop_active {
                    pos.trailing_stop_active = true;
                    // Use highest_price (not current_price) — identical to live trading
                    // activate_trailing_stop() in positions.rs which anchors to the peak.
                    pos.trailing_stop_price  = pos.highest_price * (1.0 - trailing_distance / 100.0);
                    println!(
                        "[PAPER TRAILING] {} - Active at ${:.8} | Peak: ${:.8} | Profit: +{:.1}%",
                        pos.symbol, pos.trailing_stop_price, pos.highest_price, profit_pct
                    );
                } else {
                    let new_stop = current_price * (1.0 - trailing_distance / 100.0);
                    if new_stop > pos.trailing_stop_price {
                        pos.trailing_stop_price = new_stop;
                    }
                }
                // Check hit OUTSIDE if/else — matches live evaluate_position() which calls
                // is_trailing_stop_hit() after both the activation AND update branches.
                // Previous code only checked inside the else branch, causing a one-cycle
                // (60s) delay on the first activation vs live trading.
                if current_price <= pos.trailing_stop_price {
                    to_sell.push((
                        addr.clone(),
                        format!("Trailing Stop (profit: +{:.1}%)", profit_pct),
                        current_price, 100.0, 0,
                    ));
                    continue;
                }
            }

            // 5. FINAL TP — sell all remaining
            // TP1 disabled → always eligible (single TP)
            // TP1 enabled, TP2 disabled → eligible after TP1 fired
            // TP1+TP2 enabled → eligible after TP2 fired (full 3-stage)
            let tp3_eligible = if tp1_pct > 0.0 {
                if tp2_pct > 0.0 { pos.tp2_fired } else { pos.tp1_fired }
            } else { true };
            if tp3_eligible && profit_pct >= take_profit {
                to_sell.push((
                    addr.clone(),
                    format!("Final TP +{:.1}%", profit_pct),
                    current_price, 100.0, 0,
                ));
                continue;
            }

            // 6. TIME EXIT — free up idle capital
            if max_hold_minutes > 0 && age_minutes >= max_hold_minutes as i64 {
                if profit_pct < time_exit_threshold {
                    to_sell.push((
                        addr.clone(),
                        format!("Time Exit {} min | P&L: {:.1}%", age_minutes, profit_pct),
                        current_price, 100.0, 0,
                    ));
                    continue;
                }
            }
        }

        to_sell
    }
}

// ============================================================
// PERSISTENCE
// ============================================================

pub fn save_paper_state(state: &PaperTradingState) -> Result<(), Box<dyn std::error::Error>> {
    let json = serde_json::to_string_pretty(state)?;
    std::fs::write("paper_state.json", json)?;
    Ok(())
}

pub fn load_paper_state(initial_balance: f64) -> PaperTradingState {
    match std::fs::read_to_string("paper_state.json") {
        Ok(content) => {
            match serde_json::from_str::<PaperTradingState>(&content) {
                Ok(state) => {
                    println!(
                        "[PAPER] State loaded: {:.4} SOL balance | {} open positions | {} closed trades",
                        state.current_balance_sol,
                        state.positions.len(),
                        state.closed_trades.len()
                    );
                    state
                }
                Err(e) => {
                    println!("[PAPER] Failed to parse saved state: {} — starting fresh", e);
                    PaperTradingState::new(initial_balance)
                }
            }
        }
        Err(_) => {
            println!("[PAPER] No saved state found, starting fresh simulation");
            PaperTradingState::new(initial_balance)
        }
    }
}

// ============================================================
// TELEGRAM NOTIFICATION FORMATTING
// ============================================================

pub fn format_paper_buy_notification(
    symbol: &str,
    name: &str,
    token_address: &str,
    amount_sol: f64,
    quoted_price: f64,
    effective_price: f64,
    slippage: f64,
    price_impact: f64,
    score: f64,
    balance_after: f64,
    open_positions: usize,
) -> String {
    format!(
        "📝 **PAPER BUY** 📝\n\
        ═══════════════════════════════\n\n\
        💎 Token: **{}** `({})`\n\
        📍 `{}`\n\n\
        💰 Capital: **{:.4} SOL**\n\
        💵 Quoted Price: **${:.8}**\n\
        💵 Effective Price: **${:.8}**\n\
        📊 Slippage: **{:.2}%** | Impact: **{:.2}%**\n\
        ⭐ Score: **{:.1}/100**\n\n\
        💼 Balance After: **{:.4} SOL**\n\
        📊 Open Positions: **{}**\n\n\
        ═══════════════════════════════\n\
        🔬 _Paper trading — no real money_",
        name, symbol, token_address,
        amount_sol,
        quoted_price, effective_price,
        slippage, price_impact,
        score,
        balance_after, open_positions,
    )
}

pub fn format_paper_sell_notification(trade: &PaperTrade, balance_after: f64) -> String {
    let emoji = match trade.result {
        TradeResult::Profit   => "✅",
        TradeResult::Loss     => "❌",
        TradeResult::BreakEven => "➖",
    };

    format!(
        "📝 **PAPER SELL** {}\n\
        ═══════════════════════════════\n\n\
        💎 Token: **{}** `({})`\n\n\
        {} P&L: **{}{:.1}%** ({}{:.5} SOL)\n\
        💰 Entry: **${:.8}**\n\
        💰 Exit: **${:.8}**\n\
        ⏰ Duration: **{} minutes**\n\n\
        🔄 Reason: **{}**\n\
        ⭐ Score at Entry: **{:.1}/100**\n\n\
        💼 Balance After: **{:.4} SOL**\n\
        ═══════════════════════════════\n\
        🔬 _Paper trading — no real money_",
        emoji,
        trade.name, trade.symbol,
        emoji,
        if trade.profit_percent >= 0.0 { "+" } else { "" }, trade.profit_percent,
        if trade.profit_sol >= 0.0 { "+" } else { "" }, trade.profit_sol,
        trade.buy_price, trade.sell_price,
        trade.hold_duration_minutes,
        trade.exit_reason,
        trade.score_at_entry,
        balance_after,
    )
}

pub fn format_paper_report(
    state: &PaperTradingState,
    current_prices: &HashMap<String, f64>,
) -> String {
    let equity = state.total_equity(current_prices);
    let roi = state.roi_percent(current_prices);
    let win_rate = state.win_rate();
    let pf = state.profit_factor();
    let runtime_hours = Utc::now()
        .signed_duration_since(state.start_time)
        .num_hours();

    let open_pnl: f64 = state.positions.values()
        .map(|pos| {
            let price = current_prices.get(&pos.token_address).copied().unwrap_or(pos.buy_price_usd);
            pos.profit_sol(price)
        })
        .sum();

    let positions_str = if state.positions.is_empty() {
        "No open positions".to_string()
    } else {
        state.positions.values()
            .map(|pos| {
                let price = current_prices.get(&pos.token_address).copied().unwrap_or(pos.buy_price_usd);
                format!(
                    "  • {} | P&L: {}{:.1}% | {:.4} SOL",
                    pos.symbol,
                    if pos.profit_percent(price) >= 0.0 { "+" } else { "" },
                    pos.profit_percent(price),
                    pos.amount_sol,
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };

    format!(
        "📊 **PAPER TRADING REPORT**\n\
        ═══════════════════════════════\n\n\
        ⏱️ Runtime: **{} hours**\n\n\
        💼 **Portfolio:**\n\
        💰 Initial Balance: **{:.4} SOL**\n\
        💰 Current Balance: **{:.4} SOL**\n\
        📈 Open P&L: **{}{:.5} SOL**\n\
        💎 Total Equity: **{:.4} SOL**\n\
        📊 ROI: **{}{:.2}%**\n\n\
        🏆 **Performance:**\n\
        📈 Win Rate: **{:.1}%**\n\
        💰 Profit Factor: **{:.2}**\n\
        📊 Total Trades: **{}** (W: {} | L: {})\n\
        💚 Total Profit: **+{:.5} SOL**\n\
        ❤️ Total Loss: **-{:.5} SOL**\n\
        🥇 Best Trade: **+{:.1}%** ({})\n\
        💔 Worst Trade: **{:.1}%** ({})\n\n\
        📋 **Open Positions ({}):**\n\
        {}\n\n\
        ═══════════════════════════════\n\
        🔬 _Paper trading — no real money_",
        runtime_hours,
        state.initial_balance_sol,
        state.current_balance_sol,
        if open_pnl >= 0.0 { "+" } else { "" }, open_pnl,
        equity,
        if roi >= 0.0 { "+" } else { "" }, roi,
        win_rate,
        pf,
        state.winning_trades + state.losing_trades,
        state.winning_trades, state.losing_trades,
        state.total_profit_sol,
        state.total_loss_sol,
        state.best_trade_pct, state.best_trade_symbol,
        state.worst_trade_pct, state.worst_trade_symbol,
        state.positions.len(),
        positions_str,
    )
}
