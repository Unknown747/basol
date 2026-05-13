// ============================================================
// AUTO SELL STRATEGY - Take profit, stop loss, trailing stop, time exit
// ============================================================

use crate::positions::Position;
use crate::strategy::TradingConfig;

// ============================================================
// SELL SIGNAL - Jenis trigger untuk sell
// ============================================================

#[derive(Debug, Clone)]
pub enum SellTrigger {
    TakeProfit { profit_percent: f64 },
    StopLoss { loss_percent: f64 },
    TrailingStop { profit_percent: f64 },
    /// Posisi terlalu lama tidak bergerak — keluar untuk bebaskan modal
    TimeExit { hold_minutes: i64, profit_percent: f64 },
    ManualSell,
}

impl SellTrigger {
    pub fn description(&self) -> String {
        match self {
            SellTrigger::TakeProfit { profit_percent } =>
                format!("TAKE PROFIT +{:.1}%", profit_percent),
            SellTrigger::StopLoss { loss_percent } =>
                format!("STOP LOSS -{:.1}%", loss_percent),
            SellTrigger::TrailingStop { profit_percent } =>
                format!("TRAILING STOP (profit: +{:.1}%)", profit_percent),
            SellTrigger::TimeExit { hold_minutes, profit_percent } =>
                format!(
                    "TIME EXIT setelah {} menit (P&L: {}{:.1}%)",
                    hold_minutes,
                    if *profit_percent >= 0.0 { "+" } else { "" },
                    profit_percent
                ),
            SellTrigger::ManualSell => "MANUAL SELL".to_string(),
        }
    }

    pub fn emoji(&self) -> &str {
        match self {
            SellTrigger::TakeProfit { .. }    => "💰",
            SellTrigger::StopLoss { .. }      => "🛑",
            SellTrigger::TrailingStop { .. }  => "📉",
            SellTrigger::TimeExit { .. }      => "⏰",
            SellTrigger::ManualSell           => "👤",
        }
    }
}

// ============================================================
// SELL DECISION - Hasil evaluasi posisi
// ============================================================

#[derive(Debug)]
pub enum SellDecision {
    Sell {
        percentage: f64,
        trigger: SellTrigger,
    },
    Hold {
        reason: String,
    },
}

// ============================================================
// FUNGSI EVALUASI POSISI
// ============================================================

/// Evaluasi satu posisi apakah perlu dijual.
///
/// **Urutan pengecekan (dari prioritas tertinggi ke terendah):**
/// 1. Take Profit  — profit >= target → jual semua
/// 2. Stop Loss    — loss >= threshold → potong sekarang
/// 3. Trailing Stop — profit pernah tinggi lalu turun → jual
/// 4. Time Exit    — stuck terlalu lama → bebaskan modal (scalping)
/// 5. Hold         — tidak ada trigger aktif
pub fn evaluate_position(
    position: &mut Position,
    current_price: f64,
    config: &TradingConfig,
) -> SellDecision {
    if current_price <= 0.0 {
        return SellDecision::Hold {
            reason: "Harga tidak tersedia saat ini".to_string(),
        };
    }

    let profit_pct = position.profit_percent(current_price);
    let age_minutes = position.age_minutes();

    // Update highest price yang pernah dicapai
    position.update_highest(current_price);

    // -------------------------------------------------------
    // 1. TAKE PROFIT
    // -------------------------------------------------------
    if profit_pct >= config.take_profit_percent {
        println!(
            "[SELL EVAL] 💰 {} - Take profit: +{:.1}% (target: +{:.1}%)",
            position.symbol, profit_pct, config.take_profit_percent
        );
        return SellDecision::Sell {
            percentage: 100.0,
            trigger: SellTrigger::TakeProfit { profit_percent: profit_pct },
        };
    }

    // -------------------------------------------------------
    // 2. STOP LOSS
    // -------------------------------------------------------
    if profit_pct <= -config.stop_loss_percent {
        println!(
            "[SELL EVAL] 🛑 {} - Stop loss: {:.1}% (batas: -{:.1}%)",
            position.symbol, profit_pct, config.stop_loss_percent
        );
        return SellDecision::Sell {
            percentage: 100.0,
            trigger: SellTrigger::StopLoss { loss_percent: profit_pct.abs() },
        };
    }

    // -------------------------------------------------------
    // 3. TRAILING STOP
    // -------------------------------------------------------
    if profit_pct >= config.trailing_start_percent {
        if !position.trailing_stop_active {
            position.activate_trailing_stop(config.trailing_distance_percent);
        } else {
            position.update_trailing_stop(current_price, config.trailing_distance_percent);
        }

        if position.is_trailing_stop_hit(current_price) {
            println!(
                "[SELL EVAL] 📉 {} - Trailing stop hit | Profit: +{:.1}% | Stop: ${:.8}",
                position.symbol, profit_pct, position.trailing_stop_price
            );
            return SellDecision::Sell {
                percentage: 100.0,
                trigger: SellTrigger::TrailingStop { profit_percent: profit_pct },
            };
        }
    }

    // -------------------------------------------------------
    // 4. TIME EXIT (khusus scalping — bebaskan modal yang nganggur)
    //
    // Logika: jika posisi sudah dipegang > MAX_HOLD_MINUTES
    // DAN profit masih di bawah TIME_EXIT_THRESHOLD_PCT,
    // keluar sekarang daripada tunggu yang tidak pasti.
    //
    // Ini penting untuk modal kecil: lebih baik keluar breakeven
    // dan cari peluang baru, daripada modal nganggur berjam-jam.
    // -------------------------------------------------------
    if config.max_hold_minutes > 0 && age_minutes >= config.max_hold_minutes as i64 {
        if profit_pct < config.time_exit_threshold_pct {
            println!(
                "[SELL EVAL] ⏰ {} - Time exit: {} menit | P&L: {:.1}% (threshold: {:.1}%)",
                position.symbol, age_minutes, profit_pct, config.time_exit_threshold_pct
            );
            return SellDecision::Sell {
                percentage: 100.0,
                trigger: SellTrigger::TimeExit {
                    hold_minutes: age_minutes,
                    profit_percent: profit_pct,
                },
            };
        }
        // Jika sudah melebihi waktu TAPI profit > threshold,
        // biarkan trailing stop yang urus (tidak paksa keluar)
        println!(
            "[SELL EVAL] ⏰ {} - Timeout tapi profit {:.1}% > {:.1}%, biarkan trailing",
            position.symbol, profit_pct, config.time_exit_threshold_pct
        );
    }

    // -------------------------------------------------------
    // 5. HOLD
    // -------------------------------------------------------
    let time_info = if config.max_hold_minutes > 0 {
        format!(
            " | Waktu: {}/{} menit",
            age_minutes, config.max_hold_minutes
        )
    } else {
        format!(" | Waktu: {} menit", age_minutes)
    };

    SellDecision::Hold {
        reason: format!(
            "P&L: {:.1}% | TP: +{:.1}% | SL: -{:.1}% | Trailing: {}{}",
            profit_pct,
            config.take_profit_percent,
            config.stop_loss_percent,
            if position.trailing_stop_active {
                format!("aktif @ ${:.8}", position.trailing_stop_price)
            } else {
                format!("aktif mulai +{:.1}%", config.trailing_start_percent)
            },
            time_info,
        ),
    }
}

/// Evaluasi semua posisi aktif dan kembalikan daftar yang perlu dijual
pub fn evaluate_all_positions(
    positions: &mut std::collections::HashMap<String, Position>,
    prices: &std::collections::HashMap<String, f64>,
    config: &TradingConfig,
) -> Vec<(String, SellDecision)> {
    let mut to_sell = Vec::new();

    for (addr, position) in positions.iter_mut() {
        if let Some(&current_price) = prices.get(addr) {
            let decision = evaluate_position(position, current_price, config);
            match &decision {
                SellDecision::Sell { trigger, .. } => {
                    println!(
                        "[SELL EVAL] {} {} {} | Masuk: ${:.8} | Sekarang: ${:.8}",
                        trigger.emoji(),
                        position.symbol,
                        trigger.description(),
                        position.buy_price_usd,
                        current_price
                    );
                    to_sell.push((addr.clone(), decision));
                }
                SellDecision::Hold { reason } => {
                    println!(
                        "[SELL EVAL] ✋ {} - HOLD | {}",
                        position.symbol, reason
                    );
                }
            }
        } else {
            println!(
                "[SELL EVAL] ⚠️  {} - Tidak ada harga, skip evaluasi",
                position.symbol
            );
        }
    }

    to_sell
}

/// Format pesan Telegram untuk notifikasi sell
pub fn format_sell_notification(
    position: &Position,
    current_price: f64,
    trigger: &SellTrigger,
    tx_signature: &str,
) -> String {
    let profit_pct = position.profit_percent(current_price);
    let profit_sol = position.amount_in_sol * profit_pct / 100.0;
    let age = position.age_minutes();

    let (status_emoji, status_text) = if profit_pct >= 0.0 {
        ("📈", "PROFIT")
    } else {
        ("📉", "LOSS")
    };

    let time_exit_note = match trigger {
        SellTrigger::TimeExit { .. } =>
            "\n⏰ _Keluar karena posisi terlalu lama — modal dibebaskan untuk peluang baru_",
        _ => "",
    };

    format!(
        "{} **AUTO SELL — {}** {}\n\
        ═══════════════════════════════\n\n\
        💎 Token: **{}** `({})`\n\
        📍 `{}`\n\n\
        {} P&L: **{}{:.1}%** ({}{:.4} SOL)\n\
        💰 Masuk: **${:.8}**\n\
        💰 Keluar: **${:.8}**\n\
        📊 Modal: **{:.4} SOL**\n\
        ⏰ Durasi: **{} menit**\n\n\
        🔄 Trigger: **{}**\n\
        🔗 TX: `{}`{}\n\n\
        ═══════════════════════════════\n\
        ⚠️ Trading otomatis — kelola risiko dengan baik",
        trigger.emoji(), status_text, trigger.emoji(),
        position.name, position.symbol,
        position.token_address,
        status_emoji,
        if profit_pct >= 0.0 { "+" } else { "" }, profit_pct,
        if profit_sol >= 0.0 { "+" } else { "" }, profit_sol,
        position.buy_price_usd, current_price,
        position.amount_in_sol,
        age,
        trigger.description(),
        &tx_signature[..tx_signature.len().min(20)],
        time_exit_note,
    )
}

/// Format pesan Telegram untuk notifikasi buy
pub fn format_buy_notification(
    token_address: &str,
    symbol: &str,
    name: &str,
    amount_sol: f64,
    price_usd: f64,
    score: f64,
    tx_signature: &str,
) -> String {
    format!(
        "🛒 **AUTO BUY** 🛒\n\
        ═══════════════════════════════\n\n\
        💎 Token: **{}** `({})`\n\
        📍 `{}`\n\n\
        💰 Jumlah: **{:.4} SOL**\n\
        💵 Harga Masuk: **${:.8}**\n\
        ⭐ Skor Analisis: **{:.1}/100**\n\n\
        🔗 TX: `{}`\n\n\
        ═══════════════════════════════\n\
        ⚠️ Trading otomatis — kelola risiko dengan baik",
        name, symbol, token_address,
        amount_sol, price_usd, score,
        &tx_signature[..tx_signature.len().min(20)]
    )
}
