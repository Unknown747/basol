// ============================================================
// PAPER TRADING - Simulasi trading tanpa uang nyata
// Gunakan untuk menguji strategi sebelum live trading
// ============================================================

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ============================================================
// KONSTANTA MAINNET - sama persis dengan biaya transaksi nyata
// ============================================================

/// Biaya jaringan Solana per transaksi (base fee 5000 lamport + priority fee ~20000 lamport)
/// Total ~25000 lamport = 0.000025 SOL per tx — angka konservatif/realistis
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
            // Slippage default sama dengan konfigurasi live trading
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
// PAPER POSITION - Posisi simulasi
// ============================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaperPosition {
    pub token_address: String,
    pub symbol: String,
    pub name: String,
    pub buy_price_usd: f64,
    /// SOL yang masih dipegang (dikurangi saat partial sell)
    pub amount_sol: f64,
    /// Token yang masih dipegang (dikurangi saat partial sell)
    pub token_amount: f64,
    pub highest_price: f64,
    pub trailing_stop_active: bool,
    pub trailing_stop_price: f64,
    pub entry_time: DateTime<Utc>,
    pub score_at_entry: f64,
    pub liquidity_at_entry: f64,
    /// Sudah eksekusi TP1 partial?
    pub tp1_fired: bool,
    /// Sudah eksekusi TP2 partial?
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

    /// Hitung price impact menggunakan formula constant product AMM (xy=k)
    /// Sama persis dengan model yang dipakai Jupiter untuk pool Solana
    /// amount_sol: jumlah SOL yang diinvestasikan
    /// liquidity_usd: total likuiditas pool dalam USD
    /// sol_price_usd: harga SOL saat ini
    pub fn calc_price_impact_pct(amount_sol: f64, liquidity_usd: f64, sol_price_usd: f64) -> f64 {
        if liquidity_usd <= 0.0 || sol_price_usd <= 0.0 {
            return 0.0;
        }
        // SOL reserve di sisi pool ≈ setengah likuiditas total (asumsi 50/50 pool)
        let sol_reserve = liquidity_usd / 2.0 / sol_price_usd;
        // Formula AMM: impact = amount_in / (reserve_in + amount_in)
        let impact = amount_sol / (sol_reserve + amount_sol);
        // Cap di 50% untuk menghindari angka tidak realistis
        (impact * 100.0).min(50.0)
    }

    /// Eksekusi paper buy — 100% simulasi kondisi mainnet
    /// Termasuk: network fee, slippage, dan price impact dari pool AMM
    pub fn execute_buy(
        &mut self,
        token_address: String,
        symbol: String,
        name: String,
        quoted_price_usd: f64,    // harga yang terlihat di DEX (sebelum slippage/impact)
        amount_sol: f64,
        slippage_percent: f64,    // slippage configured (dari DEFAULT_SLIPPAGE env)
        sol_price_usd: f64,       // harga SOL saat ini
        score: f64,
        liquidity_usd: f64,
    ) -> Result<String, String> {
        // Cek saldo cukup untuk amount + network fee
        let total_needed = amount_sol + NETWORK_FEE_SOL;
        if self.current_balance_sol < total_needed {
            return Err(format!(
                "Saldo virtual tidak cukup: {:.6} SOL (butuh {:.6} SOL termasuk fee)",
                self.current_balance_sol, total_needed
            ));
        }

        if self.positions.contains_key(&token_address) {
            return Err(format!("Sudah punya posisi untuk {}", symbol));
        }

        // === SIMULASI MAINNET: Network fee ===
        self.current_balance_sol -= NETWORK_FEE_SOL;

        // === SIMULASI MAINNET: Price impact (constant product AMM) ===
        let price_impact_pct = Self::calc_price_impact_pct(amount_sol, liquidity_usd, sol_price_usd);

        // === SIMULASI MAINNET: Slippage pada beli (harga naik = lebih buruk untuk buyer) ===
        // Effective price = quoted * (1 + slippage%) * (1 + impact%)
        let total_cost_pct = slippage_percent + price_impact_pct;
        let effective_buy_price = quoted_price_usd * (1.0 + total_cost_pct / 100.0);

        // === Token amount berdasarkan harga efektif (bukan harga quote) ===
        let token_amount = if effective_buy_price > 0.0 {
            (amount_sol * sol_price_usd) / effective_buy_price
        } else {
            0.0
        };

        self.current_balance_sol -= amount_sol;
        self.total_buys += 1;

        println!(
            "[PAPER BUY] ✅ {} ({}) | {:.4} SOL @ quoted=${:.8} → effective=${:.8}\n\
             [PAPER BUY]    Slippage: {:.2}% | Price Impact: {:.2}% | Fee: {:.6} SOL | Token: {:.2}",
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

    /// Eksekusi paper sell — mendukung partial sell (3-stage TP) dan full close.
    ///
    /// - `percentage`: persen posisi yang dijual (1–100). 100 = close penuh.
    /// - `tp_stage`: 0 = full close, 1 = TP1 partial, 2 = TP2 partial.
    ///   Partial sell mengurangi amount_sol/token_amount dan menandai tp1/tp2_fired,
    ///   posisi tetap aktif. Full close menghapus posisi dari daftar aktif.
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

        // Ambil snapshot fields yang dibutuhkan (tanpa memindahkan ownership dulu)
        let (sym, name, buy_price, pos_amount_sol, pos_tokens,
             liquidity, score, entry_time, age_min) = {
            let pos = self.positions.get(token_address)
                .ok_or_else(|| format!("Posisi tidak ditemukan: {}", token_address))?;
            (
                pos.symbol.clone(), pos.name.clone(), pos.buy_price_usd,
                pos.amount_sol, pos.token_amount,
                pos.liquidity_at_entry, pos.score_at_entry,
                pos.entry_time, pos.age_minutes(),
            )
        };

        // Hitung berapa yang benar-benar dijual (persen dari posisi SAAT INI)
        let sold_sol    = pos_amount_sol * percentage / 100.0;
        let sold_tokens = pos_tokens     * percentage / 100.0;

        // Price impact dari porsi yang dijual
        let sell_value_usd = sold_tokens * quoted_sell_price;
        let sell_impact_pct = if liquidity > 0.0 {
            (sell_value_usd / liquidity * 50.0).min(30.0)
        } else { 0.0 };

        let total_cost_pct      = slippage_percent + sell_impact_pct;
        let effective_sell_price = quoted_sell_price * (1.0 - total_cost_pct / 100.0);

        // Profit dihitung terhadap harga beli efektif
        let profit_pct = if buy_price > 0.0 {
            (effective_sell_price - buy_price) / buy_price * 100.0
        } else { 0.0 };
        let profit_sol = sold_sol * profit_pct / 100.0;

        // Proceeds = (modal bagian yg dijual + profit) - network fee
        let gross_proceeds = sold_sol + profit_sol;
        let net_proceeds   = (gross_proceeds - NETWORK_FEE_SOL).max(0.0);

        self.current_balance_sol += net_proceeds;
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
            self.best_trade_symbol = sym.clone();
        }
        if profit_pct < self.worst_trade_pct {
            self.worst_trade_pct = profit_pct;
            self.worst_trade_symbol = sym.clone();
        }

        let stage_label = match tp_stage {
            1 => " [TP1 33%]",
            2 => " [TP2 50%sisa]",
            _ => "",
        };

        println!(
            "[PAPER SELL] {} {} ({}) | {:.0}%{} | ${:.8} → ${:.8}\n\
             [PAPER SELL]    Slip: {:.2}% | Impact: {:.2}% | P&L: {}{:.1}% ({}{:.5} SOL) | Saldo: {:.4} SOL",
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
            // Kurangi posisi — jangan hapus
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
            // Full close — hapus posisi
            self.positions.remove(token_address);
        }

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

    /// Evaluasi semua posisi — mendukung 3-stage TP, SL, trailing stop, time exit.
    ///
    /// Return: `Vec<(addr, reason, price, sell_percent, tp_stage)>`
    /// - `sell_percent`: berapa persen posisi yang harus dijual (1–100)
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

            // 1. STOP LOSS — potong rugi, jual SEMUA
            if profit_pct <= -stop_loss {
                to_sell.push((
                    addr.clone(),
                    format!("Stop Loss {:.1}%", profit_pct),
                    current_price, 100.0, 0,
                ));
                continue;
            }

            // 2. TP1 PARTIAL — jual tp1_sell_pct% jika belum fire
            if tp1_pct > 0.0 && !pos.tp1_fired && profit_pct >= tp1_pct {
                println!(
                    "[PAPER TP1] 🎯 {} - profit +{:.1}% >= +{:.1}% | Jual {:.0}%",
                    pos.symbol, profit_pct, tp1_pct, tp1_sell_pct
                );
                to_sell.push((
                    addr.clone(),
                    format!("TP1 Partial {:.0}% @ +{:.1}%", tp1_sell_pct, profit_pct),
                    current_price, tp1_sell_pct, 1,
                ));
                continue;
            }

            // 3. TP2 PARTIAL — jual tp2_sell_pct% dari sisa jika TP1 sudah fire
            if tp2_pct > 0.0 && pos.tp1_fired && !pos.tp2_fired && profit_pct >= tp2_pct {
                println!(
                    "[PAPER TP2] 🎯 {} - profit +{:.1}% >= +{:.1}% | Jual {:.0}% sisa",
                    pos.symbol, profit_pct, tp2_pct, tp2_sell_pct
                );
                to_sell.push((
                    addr.clone(),
                    format!("TP2 Partial {:.0}% @ +{:.1}%", tp2_sell_pct, profit_pct),
                    current_price, tp2_sell_pct, 2,
                ));
                continue;
            }

            // 4. TRAILING STOP — lindungi sisa posisi
            if profit_pct >= trailing_start {
                if !pos.trailing_stop_active {
                    pos.trailing_stop_active = true;
                    pos.trailing_stop_price  = current_price * (1.0 - trailing_distance / 100.0);
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
                        to_sell.push((
                            addr.clone(),
                            format!("Trailing Stop (profit: +{:.1}%)", profit_pct),
                            current_price, 100.0, 0,
                        ));
                        continue;
                    }
                }
            }

            // 5. TP FINAL — jual semua sisa
            // Syarat: jika 3-stage aktif → harus sudah TP2; jika single TP → langsung
            let tp_final_eligible = if tp1_pct > 0.0 { pos.tp2_fired } else { true };
            if tp_final_eligible && profit_pct >= take_profit {
                to_sell.push((
                    addr.clone(),
                    format!("Take Profit Final +{:.1}%", profit_pct),
                    current_price, 100.0, 0,
                ));
                continue;
            }

            // 6. TIME EXIT — posisi stuck
            if max_hold_minutes > 0
                && age_minutes >= max_hold_minutes as i64
                && profit_pct < time_exit_threshold
            {
                to_sell.push((
                    addr.clone(),
                    format!("Time Exit {} menit | P&L: {:.1}%", age_minutes, profit_pct),
                    current_price, 100.0, 0,
                ));
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
    quoted_price_usd: f64,
    effective_price_usd: f64,
    slippage_pct: f64,
    price_impact_pct: f64,
    score: f64,
    balance_after: f64,
    total_positions: usize,
) -> String {
    let total_cost_pct = slippage_pct + price_impact_pct;
    format!(
        "📋 **PAPER TRADING - SIMULASI BUY**\n\
        ═══════════════════════════════\n\n\
        💎 Token: **{}** `({})`\n\
        📍 `{}`\n\n\
        💰 Modal Simulasi: **{:.4} SOL** (virtual)\n\
        💵 Harga Quote: **${:.8}**\n\
        💵 Harga Efektif: **${:.8}** _(setelah biaya)_\n\
        📉 Biaya Transaksi: **{:.2}%** (slip {:.1}% + impact {:.1}%)\n\
        🔧 Network Fee: **{:.6} SOL**\n\
        ⭐ Skor Analisis: **{:.1}/100**\n\
        📊 Posisi Aktif: **{}/max**\n\
        💼 Saldo Virtual Tersisa: **{:.4} SOL**\n\n\
        🔬 _Simulasi 100% akurat — slippage & price impact mainnet diterapkan_",
        name, symbol, token_address,
        amount_sol,
        quoted_price_usd, effective_price_usd,
        total_cost_pct, slippage_pct, price_impact_pct,
        NETWORK_FEE_SOL,
        score,
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
        {} Hasil: **{}**\n\n\
        💎 Token: **{}** `({})`\n\
        📍 `{}`\n\n\
        {} P&L: **{}{:.1}%** ({}{:.4} SOL)\n\
        💰 Beli Efektif: **${:.8}**\n\
        💰 Jual Efektif: **${:.8}** _(setelah slip + impact)_\n\
        📊 Modal: **{:.4} SOL**\n\
        🔧 Fee Jual: **{:.6} SOL**\n\
        ⏰ Durasi: **{} menit**\n\
        🔄 Alasan Keluar: **{}**\n\
        💼 Saldo Virtual: **{:.4} SOL**\n\n\
        🔬 _Simulasi 100% akurat — slippage, impact & fee mainnet diterapkan_",
        emoji, result_text,
        trade.name, trade.symbol,
        trade.token_address,
        emoji,
        if trade.profit_percent >= 0.0 { "+" } else { "" }, trade.profit_percent,
        if trade.profit_sol >= 0.0 { "+" } else { "" }, trade.profit_sol,
        trade.buy_price,
        trade.sell_price,
        trade.amount_sol,
        NETWORK_FEE_SOL,
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
