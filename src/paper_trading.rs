// ============================================================
// PAPER TRADING - Simulasi trading tanpa uang nyata
// Gunakan untuk menguji strategi sebelum live trading
// ============================================================

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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
            max_positions: std::env::var("MAX_POSITIONS")
                .ok().and_then(|v| v.parse().ok()).unwrap_or(5),
            report_interval_secs: std::env::var("PAPER_REPORT_INTERVAL_SECS")
                .ok().and_then(|v| v.parse().ok()).unwrap_or(3600),
        }
    }
}

// ============================================================
// PAPER POSITION - Posisi simulasi
// ============================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaperPosition {
    pub token_address: String,
    pub symbol: String,
    pub name: String,
    pub buy_price_usd: f64,
    pub amount_sol: f64,
    pub token_amount: f64,
    pub highest_price: f64,
    pub trailing_stop_active: bool,
    pub trailing_stop_price: f64,
    pub entry_time: DateTime<Utc>,
    pub score_at_entry: f64,
    pub liquidity_at_entry: f64,
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
// PAPER TRADE HISTORY - Riwayat trade
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

    /// Hitung total equity (balance + nilai posisi terbuka)
    pub fn total_equity(&self, current_prices: &HashMap<String, f64>) -> f64 {
        let open_pnl: f64 = self.positions.values()
            .map(|pos| {
                let price = current_prices.get(&pos.token_address).copied().unwrap_or(pos.buy_price_usd);
                pos.amount_sol + pos.profit_sol(price)
            })
            .sum();
        self.current_balance_sol + open_pnl
    }

    /// Return on Investment keseluruhan
    pub fn roi_percent(&self, current_prices: &HashMap<String, f64>) -> f64 {
        if self.initial_balance_sol == 0.0 { return 0.0; }
        (self.total_equity(current_prices) - self.initial_balance_sol)
            / self.initial_balance_sol * 100.0
    }

    /// Win rate berdasarkan closed trades
    pub fn win_rate(&self) -> f64 {
        let total = self.winning_trades + self.losing_trades;
        if total == 0 { return 0.0; }
        self.winning_trades as f64 / total as f64 * 100.0
    }

    /// Profit factor (gross profit / gross loss)
    pub fn profit_factor(&self) -> f64 {
        if self.total_loss_sol == 0.0 {
            return if self.total_profit_sol > 0.0 { f64::INFINITY } else { 0.0 };
        }
        self.total_profit_sol / self.total_loss_sol
    }

    /// Eksekusi paper buy
    pub fn execute_buy(
        &mut self,
        token_address: String,
        symbol: String,
        name: String,
        buy_price_usd: f64,
        amount_sol: f64,
        token_amount: f64,
        score: f64,
        liquidity_usd: f64,
    ) -> Result<String, String> {
        if self.current_balance_sol < amount_sol {
            return Err(format!(
                "Saldo virtual tidak cukup: {:.4} SOL (butuh {:.4} SOL)",
                self.current_balance_sol, amount_sol
            ));
        }

        if self.positions.contains_key(&token_address) {
            return Err(format!("Sudah punya posisi untuk {}", symbol));
        }

        self.current_balance_sol -= amount_sol;
        self.total_buys += 1;

        let position = PaperPosition {
            token_address: token_address.clone(),
            symbol: symbol.clone(),
            name: name.clone(),
            buy_price_usd,
            amount_sol,
            token_amount,
            highest_price: buy_price_usd,
            trailing_stop_active: false,
            trailing_stop_price: 0.0,
            entry_time: Utc::now(),
            score_at_entry: score,
            liquidity_at_entry: liquidity_usd,
        };

        self.positions.insert(token_address.clone(), position);

        let fake_sig = format!("PAPER_{}", &token_address[..8]);
        println!(
            "[PAPER BUY] ✅ {} ({}) | {:.4} SOL @ ${:.8} | Saldo: {:.4} SOL",
            name, symbol, amount_sol, buy_price_usd, self.current_balance_sol
        );

        Ok(fake_sig)
    }

    /// Eksekusi paper sell
    pub fn execute_sell(
        &mut self,
        token_address: &str,
        sell_price: f64,
        percentage: f64,
        exit_reason: String,
    ) -> Result<PaperTrade, String> {
        let pos = self.positions.remove(token_address)
            .ok_or_else(|| format!("Posisi tidak ditemukan: {}", token_address))?;

        let profit_pct = pos.profit_percent(sell_price);
        let profit_sol = pos.profit_sol(sell_price);
        let proceeds = pos.amount_sol + profit_sol;

        self.current_balance_sol += proceeds * (percentage / 100.0);
        self.total_sells += 1;

        let result = if profit_pct > 0.5 {
            self.winning_trades += 1;
            self.total_profit_sol += profit_sol.max(0.0);
            TradeResult::Profit
        } else if profit_pct < -0.5 {
            self.losing_trades += 1;
            self.total_loss_sol += profit_sol.abs();
            TradeResult::Loss
        } else {
            TradeResult::BreakEven
        };

        if profit_pct > self.best_trade_pct {
            self.best_trade_pct = profit_pct;
            self.best_trade_symbol = pos.symbol.clone();
        }
        if profit_pct < self.worst_trade_pct {
            self.worst_trade_pct = profit_pct;
            self.worst_trade_symbol = pos.symbol.clone();
        }

        let trade = PaperTrade {
            token_address: pos.token_address.clone(),
            symbol: pos.symbol.clone(),
            name: pos.name.clone(),
            buy_price: pos.buy_price_usd,
            sell_price,
            amount_sol: pos.amount_sol,
            profit_percent: profit_pct,
            profit_sol,
            buy_time: pos.entry_time,
            sell_time: Utc::now(),
            hold_duration_minutes: pos.age_minutes(),
            exit_reason: exit_reason.clone(),
            score_at_entry: pos.score_at_entry,
            result,
        };

        println!(
            "[PAPER SELL] {} {} ({}) | P&L: {}{:.1}% ({}{:.4} SOL) | {} | Saldo: {:.4} SOL",
            if profit_pct >= 0.0 { "✅" } else { "❌" },
            pos.name, pos.symbol,
            if profit_pct >= 0.0 { "+" } else { "" }, profit_pct,
            if profit_sol >= 0.0 { "+" } else { "" }, profit_sol,
            exit_reason,
            self.current_balance_sol,
        );

        self.closed_trades.push(trade.clone());
        Ok(trade)
    }

    /// Update posisi: trailing stop dan harga tertinggi
    pub fn update_positions(
        &mut self,
        prices: &HashMap<String, f64>,
        trailing_start: f64,
        trailing_distance: f64,
    ) -> Vec<(String, String, f64)> {
        let mut to_sell = Vec::new();

        for (addr, pos) in self.positions.iter_mut() {
            let current_price = match prices.get(addr) {
                Some(&p) => p,
                None => continue,
            };

            if current_price > pos.highest_price {
                pos.highest_price = current_price;
            }

            let profit_pct = pos.profit_percent(current_price);

            // Aktifkan trailing stop
            if profit_pct >= trailing_start && !pos.trailing_stop_active {
                pos.trailing_stop_active = true;
                pos.trailing_stop_price = current_price * (1.0 - trailing_distance / 100.0);
                println!(
                    "[PAPER TRAILING] {} - Trailing aktif di ${:.8} (profit: +{:.1}%)",
                    pos.symbol, pos.trailing_stop_price, profit_pct
                );
            }

            // Update trailing stop ke atas
            if pos.trailing_stop_active {
                let new_stop = current_price * (1.0 - trailing_distance / 100.0);
                if new_stop > pos.trailing_stop_price {
                    pos.trailing_stop_price = new_stop;
                }

                // Cek apakah trailing stop kena
                if current_price <= pos.trailing_stop_price {
                    to_sell.push((addr.clone(), "Trailing Stop".to_string(), current_price));
                }
            }
        }

        to_sell
    }

    /// Evaluasi semua posisi untuk TP/SL/Trailing
    pub fn evaluate_positions(
        &mut self,
        prices: &HashMap<String, f64>,
        take_profit: f64,
        stop_loss: f64,
        trailing_start: f64,
        trailing_distance: f64,
    ) -> Vec<(String, String, f64)> {
        let mut to_sell: Vec<(String, String, f64)> = Vec::new();

        for (addr, pos) in self.positions.iter_mut() {
            let current_price = match prices.get(addr) {
                Some(&p) => p,
                None => continue,
            };

            if current_price > pos.highest_price {
                pos.highest_price = current_price;
            }

            let profit_pct = pos.profit_percent(current_price);

            // Take profit
            if profit_pct >= take_profit {
                to_sell.push((addr.clone(), format!("Take Profit +{:.1}%", profit_pct), current_price));
                continue;
            }

            // Stop loss
            if profit_pct <= -stop_loss {
                to_sell.push((addr.clone(), format!("Stop Loss {:.1}%", profit_pct), current_price));
                continue;
            }

            // Trailing stop
            if profit_pct >= trailing_start {
                if !pos.trailing_stop_active {
                    pos.trailing_stop_active = true;
                    pos.trailing_stop_price = current_price * (1.0 - trailing_distance / 100.0);
                    println!(
                        "[PAPER TRAILING] {} - Aktif di ${:.8} | Profit: +{:.1}%",
                        pos.symbol, pos.trailing_stop_price, profit_pct
                    );
                } else {
                    let new_stop = current_price * (1.0 - trailing_distance / 100.0);
                    if new_stop > pos.trailing_stop_price {
                        pos.trailing_stop_price = new_stop;
                    }
                    if current_price <= pos.trailing_stop_price {
                        to_sell.push((addr.clone(), format!("Trailing Stop (profit: +{:.1}%)", profit_pct), current_price));
                    }
                }
            }
        }

        to_sell
    }
}

// ============================================================
// FORMATTING & REPORTING
// ============================================================

pub fn format_paper_buy_notification(
    symbol: &str,
    name: &str,
    token_address: &str,
    amount_sol: f64,
    price_usd: f64,
    score: f64,
    balance_after: f64,
    total_positions: usize,
) -> String {
    format!(
        "📋 **PAPER TRADING - SIMULASI BUY**\n\
        ═══════════════════════════════\n\n\
        💎 Token: **{}** `({})`\n\
        📍 `{}`\n\n\
        💰 Modal Simulasi: **{:.4} SOL** (virtual)\n\
        💵 Harga Masuk: **${:.8}**\n\
        ⭐ Skor Analisis: **{:.1}/100**\n\
        📊 Posisi Aktif: **{}/max**\n\
        💼 Saldo Virtual Tersisa: **{:.4} SOL**\n\n\
        ⚠️ _Ini adalah simulasi - tidak ada uang nyata yang digunakan_",
        name, symbol, token_address,
        amount_sol, price_usd, score,
        total_positions, balance_after
    )
}

pub fn format_paper_sell_notification(trade: &PaperTrade, balance_after: f64) -> String {
    let (emoji, result_text) = if trade.profit_percent >= 0.0 {
        ("✅", "PROFIT")
    } else {
        ("❌", "LOSS")
    };

    format!(
        "📋 **PAPER TRADING - SIMULASI SELL**\n\
        ═══════════════════════════════\n\n\
        {} P&L: **{}**\n\n\
        💎 Token: **{}** `({})`\n\
        📍 `{}`\n\n\
        {} P&L: **{}{:.1}%** ({}{:.4} SOL)\n\
        💰 Masuk: **${:.8}**\n\
        💰 Keluar: **${:.8}**\n\
        📊 Modal: **{:.4} SOL**\n\
        ⏰ Durasi: **{} menit**\n\
        🔄 Alasan: **{}**\n\
        💼 Saldo Virtual: **{:.4} SOL**\n\n\
        ⚠️ _Ini adalah simulasi - tidak ada uang nyata yang digunakan_",
        emoji, result_text,
        trade.name, trade.symbol,
        trade.token_address,
        emoji,
        if trade.profit_percent >= 0.0 { "+" } else { "" }, trade.profit_percent,
        if trade.profit_sol >= 0.0 { "+" } else { "" }, trade.profit_sol,
        trade.buy_price, trade.sell_price,
        trade.amount_sol,
        trade.hold_duration_minutes,
        trade.exit_reason,
        balance_after
    )
}

pub fn format_paper_report(state: &PaperTradingState, current_prices: &HashMap<String, f64>) -> String {
    let equity = state.total_equity(current_prices);
    let roi = state.roi_percent(current_prices);
    let win_rate = state.win_rate();
    let profit_factor = state.profit_factor();
    let running_hours = Utc::now()
        .signed_duration_since(state.start_time)
        .num_hours();

    // Rangking 5 trade terbaik
    let mut sorted_trades = state.closed_trades.clone();
    sorted_trades.sort_by(|a, b| b.profit_percent.partial_cmp(&a.profit_percent).unwrap());

    let top_trades: String = sorted_trades.iter().take(5)
        .enumerate()
        .map(|(i, t)| format!(
            "{}. {} ({}) → {}{:.1}% | {} | {}",
            i + 1,
            t.symbol, t.name,
            if t.profit_percent >= 0.0 { "+" } else { "" },
            t.profit_percent,
            if t.profit_percent >= 0.0 { "✅" } else { "❌" },
            t.exit_reason
        ))
        .collect::<Vec<_>>()
        .join("\n");

    // Status posisi terbuka
    let open_positions: String = if state.positions.is_empty() {
        "Tidak ada posisi terbuka".to_string()
    } else {
        state.positions.values()
            .map(|pos| {
                let curr = current_prices.get(&pos.token_address).copied().unwrap_or(pos.buy_price_usd);
                let pct = pos.profit_percent(curr);
                format!(
                    "• {} | {}{:.1}% | {:.4} SOL | {} menit",
                    pos.symbol,
                    if pct >= 0.0 { "+" } else { "" }, pct,
                    pos.amount_sol,
                    pos.age_minutes()
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };

    format!(
        "📊 **PAPER TRADING REPORT**\n\
        ═══════════════════════════════\n\
        ⏰ Durasi Running: **{} jam**\n\n\
        💰 **Performa Keseluruhan:**\n\
        🏦 Modal Awal: **{:.4} SOL**\n\
        💼 Equity Saat Ini: **{:.4} SOL**\n\
        {} ROI: **{}{:.1}%**\n\n\
        📈 **Statistik Trading:**\n\
        🔢 Total Trade: **{}** (Buy: {} | Sell: {})\n\
        ✅ Winning: **{}** | ❌ Losing: **{}**\n\
        🎯 Win Rate: **{:.1}%**\n\
        ⚖️ Profit Factor: **{:.2}**\n\
        💚 Total Profit: **+{:.4} SOL**\n\
        ❤️ Total Loss: **-{:.4} SOL**\n\
        🏆 Trade Terbaik: **{}** (+{:.1}%)\n\
        💀 Trade Terburuk: **{}** ({:.1}%)\n\n\
        📋 **Posisi Terbuka ({}):**\n\
        {}\n\n\
        🏅 **Top 5 Trade:**\n\
        {}\n\n\
        ═══════════════════════════════\n\
        ⚠️ _Simulasi Paper Trading - Bukan uang nyata_",
        running_hours,
        state.initial_balance_sol,
        equity,
        if roi >= 0.0 { "📈" } else { "📉" },
        if roi >= 0.0 { "+" } else { "" }, roi,
        state.total_buys.max(state.total_sells), state.total_buys, state.total_sells,
        state.winning_trades, state.losing_trades,
        win_rate,
        profit_factor,
        state.total_profit_sol,
        state.total_loss_sol,
        if state.best_trade_symbol.is_empty() { "-".to_string() } else { state.best_trade_symbol.clone() },
        state.best_trade_pct,
        if state.worst_trade_symbol.is_empty() { "-".to_string() } else { state.worst_trade_symbol.clone() },
        state.worst_trade_pct,
        state.positions.len(),
        open_positions,
        if top_trades.is_empty() { "Belum ada trade selesai".to_string() } else { top_trades },
    )
}

pub fn save_paper_state(state: &PaperTradingState) -> Result<(), String> {
    let json = serde_json::to_string_pretty(state)
        .map_err(|e| format!("Gagal serialize paper state: {}", e))?;
    std::fs::write("paper_trading.json", json)
        .map_err(|e| format!("Gagal save paper state: {}", e))?;
    Ok(())
}

pub fn load_paper_state(initial_balance: f64) -> PaperTradingState {
    match std::fs::read_to_string("paper_trading.json") {
        Ok(content) => {
            match serde_json::from_str::<PaperTradingState>(&content) {
                Ok(state) => {
                    println!(
                        "[PAPER] State dimuat - Equity: {:.4} SOL | {} trade closed | {} posisi terbuka",
                        state.current_balance_sol,
                        state.closed_trades.len(),
                        state.positions.len()
                    );
                    state
                }
                Err(e) => {
                    println!("[PAPER] Gagal load state ({}), mulai baru", e);
                    PaperTradingState::new(initial_balance)
                }
            }
        }
        Err(_) => {
            println!("[PAPER] Tidak ada state tersimpan, mulai simulasi baru");
            PaperTradingState::new(initial_balance)
        }
    }
}
