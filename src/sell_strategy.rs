// ============================================================
// AUTO SELL STRATEGY - Take profit bertahap, stop loss, trailing, time exit
// ============================================================

use crate::positions::Position;
use crate::strategy::TradingConfig;

// ============================================================
// SELL TRIGGER - Jenis pemicu jual
// ============================================================

#[derive(Debug, Clone)]
pub enum SellTrigger {
    /// Partial take profit — jual sebagian posisi di level ini
    PartialTakeProfit {
        stage: u8,          // 1 atau 2
        target_pct: f64,    // level TP yang dikonfigurasi
        profit_pct: f64,    // profit aktual saat trigger
        sell_pct: f64,      // berapa % posisi yang dijual
    },
    /// Take profit penuh — jual semua sisa posisi
    TakeProfit { profit_percent: f64 },
    StopLoss { loss_percent: f64 },
    TrailingStop { profit_percent: f64 },
    /// Posisi stuck terlalu lama — keluar untuk bebaskan modal
    TimeExit { hold_minutes: i64, profit_percent: f64 },
    ManualSell,
}

impl SellTrigger {
    pub fn description(&self) -> String {
        match self {
            SellTrigger::PartialTakeProfit { stage, target_pct, profit_pct, sell_pct } =>
                format!(
                    "TP{} PARTIAL {:.0}% — profit +{:.1}% (target +{:.1}%)",
                    stage, sell_pct, profit_pct, target_pct
                ),
            SellTrigger::TakeProfit { profit_percent } =>
                format!("TAKE PROFIT FINAL +{:.1}%", profit_percent),
            SellTrigger::StopLoss { loss_percent } =>
                format!("STOP LOSS -{:.1}%", loss_percent),
            SellTrigger::TrailingStop { profit_percent } =>
                format!("TRAILING STOP (profit: +{:.1}%)", profit_percent),
            SellTrigger::TimeExit { hold_minutes, profit_percent } =>
                format!(
                    "TIME EXIT {} menit | P&L: {}{:.1}%",
                    hold_minutes,
                    if *profit_percent >= 0.0 { "+" } else { "" },
                    profit_percent
                ),
            SellTrigger::ManualSell => "MANUAL SELL".to_string(),
        }
    }

    pub fn emoji(&self) -> &str {
        match self {
            SellTrigger::PartialTakeProfit { .. } => "🎯",
            SellTrigger::TakeProfit { .. }         => "💰",
            SellTrigger::StopLoss { .. }           => "🛑",
            SellTrigger::TrailingStop { .. }       => "📉",
            SellTrigger::TimeExit { .. }           => "⏰",
            SellTrigger::ManualSell                => "👤",
        }
    }

    /// Apakah ini partial sell (bukan close penuh)?
    pub fn is_partial(&self) -> bool {
        matches!(self, SellTrigger::PartialTakeProfit { .. })
    }
}

// ============================================================
// SELL DECISION
// ============================================================

#[derive(Debug)]
pub enum SellDecision {
    Sell {
        /// Persen dari posisi SAAT INI yang dijual (100.0 = semua)
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

/// Evaluasi satu posisi — apakah perlu dijual, berapa persen, dengan trigger apa.
///
/// **Urutan pengecekan:**
/// 1. Stop Loss        — potong rugi sekarang, jual SEMUA (100%)
/// 2. TP1 Partial      — jual tp1_sell_percent% di level tp1_percent
/// 3. TP2 Partial      — jual tp2_sell_percent% di level tp2_percent (setelah TP1 fired)
/// 4. Trailing Stop    — lindungi profit, jual sisa
/// 5. TP Final         — jual semua sisa jika mencapai take_profit_percent
/// 6. Time Exit        — posisi nganggur, keluar untuk bebaskan modal
/// 7. Hold             — tidak ada trigger aktif
///
/// **Catatan matematika:**
/// Dengan modal 0.05 SOL dan biaya round-trip 3.81%:
/// - TP1 di +12%: net +8.2% (AMAN, sudah di atas break-even)
/// - TP2 di +20%: net +16.2% (bagus)
/// - TP Final di +35%: net +31.2% (jika market sedang hot)
/// - SL di -8%: net -11.3% (potong cepat, hemat modal)
pub fn evaluate_position(
    position: &mut Position,
    current_price: f64,
    config: &TradingConfig,
) -> SellDecision {
    if current_price <= 0.0 {
        return SellDecision::Hold {
            reason: "Harga tidak tersedia".to_string(),
        };
    }

    let profit_pct  = position.profit_percent(current_price);
    let age_minutes = position.age_minutes();

    position.update_highest(current_price);

    // -------------------------------------------------------
    // 1. STOP LOSS — selalu jual 100%, potong rugi segera
    //    Jangan partial SL — kerugian bisa makin dalam!
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
    // 2. TP1 — jual sebagian pertama
    //    Kunci profit awal, pastikan tidak rugi fee.
    //    TP1 harus > break-even (~3.81% untuk 0.05 SOL).
    // -------------------------------------------------------
    if config.tp1_percent > 0.0
        && !position.tp1_fired
        && profit_pct >= config.tp1_percent
    {
        println!(
            "[SELL EVAL] 🎯 {} - TP1: profit +{:.1}% >= +{:.1}% | Jual {:.0}% posisi",
            position.symbol, profit_pct, config.tp1_percent, config.tp1_sell_percent
        );
        return SellDecision::Sell {
            percentage: config.tp1_sell_percent,
            trigger: SellTrigger::PartialTakeProfit {
                stage: 1,
                target_pct: config.tp1_percent,
                profit_pct,
                sell_pct: config.tp1_sell_percent,
            },
        };
    }

    // -------------------------------------------------------
    // 3. TP2 — jual sebagian kedua (hanya jika TP1 sudah fire)
    //    Lock in profit lebih besar, biarkan sisa riding.
    // -------------------------------------------------------
    if config.tp2_percent > 0.0
        && position.tp1_fired
        && !position.tp2_fired
        && profit_pct >= config.tp2_percent
    {
        println!(
            "[SELL EVAL] 🎯 {} - TP2: profit +{:.1}% >= +{:.1}% | Jual {:.0}% sisa posisi",
            position.symbol, profit_pct, config.tp2_percent, config.tp2_sell_percent
        );
        return SellDecision::Sell {
            percentage: config.tp2_sell_percent,
            trigger: SellTrigger::PartialTakeProfit {
                stage: 2,
                target_pct: config.tp2_percent,
                profit_pct,
                sell_pct: config.tp2_sell_percent,
            },
        };
    }

    // -------------------------------------------------------
    // 4. TRAILING STOP — aktif setelah profit >= trailing_start
    //    Setelah TP1 atau TP2 fire, trailing melindungi sisa posisi.
    //    Jika harga terus naik → stop ikut naik (ratchet up).
    //    Jika harga balik turun → stop kena → jual semua sisa.
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
    // 5. TP FINAL — jual semua sisa jika harga terus naik kencang.
    //    - Jika 3-stage aktif (tp1/tp2 dikonfigurasi): hanya fire setelah TP2
    //    - Jika 3-stage nonaktif (tp1_percent=0): berlaku sebagai single TP
    // -------------------------------------------------------
    let tp3_eligible = if config.tp1_percent > 0.0 {
        position.tp2_fired // 3-stage mode: harus sudah TP2
    } else {
        true // single TP mode
    };

    if tp3_eligible && profit_pct >= config.take_profit_percent {
        println!(
            "[SELL EVAL] 💰 {} - TP Final: +{:.1}% (target: +{:.1}%)",
            position.symbol, profit_pct, config.take_profit_percent
        );
        return SellDecision::Sell {
            percentage: 100.0,
            trigger: SellTrigger::TakeProfit { profit_percent: profit_pct },
        };
    }

    // -------------------------------------------------------
    // 6. TIME EXIT — bebaskan modal yang nganggur
    // -------------------------------------------------------
    if config.max_hold_minutes > 0 && age_minutes >= config.max_hold_minutes as i64 {
        if profit_pct < config.time_exit_threshold_pct {
            println!(
                "[SELL EVAL] ⏰ {} - Time exit: {} menit | P&L: {:.1}%",
                position.symbol, age_minutes, profit_pct
            );
            return SellDecision::Sell {
                percentage: 100.0,
                trigger: SellTrigger::TimeExit { hold_minutes: age_minutes, profit_percent: profit_pct },
            };
        }
    }

    // -------------------------------------------------------
    // 7. HOLD
    // -------------------------------------------------------
    let tp_status = if config.tp1_percent > 0.0 {
        match (position.tp1_fired, position.tp2_fired) {
            (false, _)    => format!("TP1 menunggu +{:.0}%", config.tp1_percent),
            (true, false) => format!("TP1✅ TP2 menunggu +{:.0}%", config.tp2_percent),
            (true, true)  => format!("TP1✅ TP2✅ Final +{:.0}%", config.take_profit_percent),
        }
    } else {
        format!("TP +{:.0}%", config.take_profit_percent)
    };

    let time_info = if config.max_hold_minutes > 0 {
        format!(" | Waktu: {}/{} mnt", age_minutes, config.max_hold_minutes)
    } else {
        format!(" | Waktu: {} mnt", age_minutes)
    };

    SellDecision::Hold {
        reason: format!(
            "P&L: {:.1}% | {} | SL: -{:.1}% | Trailing: {}{}",
            profit_pct,
            tp_status,
            config.stop_loss_percent,
            if position.trailing_stop_active {
                format!("aktif @ ${:.8}", position.trailing_stop_price)
            } else {
                format!("aktif mulai +{:.0}%", config.trailing_start_percent)
            },
            time_info,
        ),
    }
}

/// Evaluasi semua posisi aktif — kembalikan daftar yang perlu dijual
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
                SellDecision::Sell { trigger, percentage } => {
                    println!(
                        "[SELL EVAL] {} {} {} ({:.0}%) | Masuk: ${:.8} | Sekarang: ${:.8}",
                        trigger.emoji(),
                        position.symbol,
                        trigger.description(),
                        percentage,
                        position.buy_price_usd,
                        current_price
                    );
                    to_sell.push((addr.clone(), decision));
                }
                SellDecision::Hold { reason } => {
                    println!("[SELL EVAL] ✋ {} - HOLD | {}", position.symbol, reason);
                }
            }
        } else {
            println!("[SELL EVAL] ⚠️  {} - Tidak ada harga, skip", position.symbol);
        }
    }

    to_sell
}

// ============================================================
// FORMAT NOTIFIKASI TELEGRAM
// ============================================================

/// Format notifikasi sell — mendukung partial TP dan full close
pub fn format_sell_notification(
    position: &Position,
    current_price: f64,
    trigger: &SellTrigger,
    tx_signature: &str,
) -> String {
    let profit_pct = position.profit_percent(current_price);
    let age        = position.age_minutes();

    let (status_emoji, sell_percent_line) = match trigger {
        SellTrigger::PartialTakeProfit { stage, sell_pct, .. } => (
            "🎯",
            format!("📊 Dijual: **{:.0}%** posisi (stage {}/2)", sell_pct, stage),
        ),
        _ => {
            let emoji = if profit_pct >= 0.0 { "📈" } else { "📉" };
            (emoji, format!("📊 Dijual: **100%** (close penuh)"))
        }
    };

    let profit_sol = position.amount_in_sol * profit_pct / 100.0;
    let tp_stages  = if position.tp1_fired || position.tp2_fired {
        format!(
            "\n📍 Stage: {}{}",
            if position.tp1_fired { "TP1✅ " } else { "TP1⬜ " },
            if position.tp2_fired { "TP2✅" } else { "TP2⬜" },
        )
    } else {
        String::new()
    };

    let time_note = match trigger {
        SellTrigger::TimeExit { .. } =>
            "\n⏰ _Keluar karena posisi stuck — modal dibebaskan untuk peluang baru_",
        _ => "",
    };

    format!(
        "{} **AUTO SELL** {}\n\
        ═══════════════════════════════\n\n\
        💎 Token: **{}** `({})`\n\
        📍 `{}`\n\n\
        {} P&L: **{}{:.1}%** ({}{:.5} SOL)\n\
        💰 Masuk: **${:.8}**\n\
        💰 Sekarang: **${:.8}**\n\
        {}{}\n\
        ⏰ Durasi: **{} menit**\n\n\
        🔄 Trigger: **{}**\n\
        🔗 TX: `{}`{}\n\n\
        ═══════════════════════════════",
        trigger.emoji(), trigger.emoji(),
        position.name, position.symbol,
        position.token_address,
        status_emoji,
        if profit_pct >= 0.0 { "+" } else { "" }, profit_pct,
        if profit_sol >= 0.0 { "+" } else { "" }, profit_sol,
        position.buy_price_usd, current_price,
        sell_percent_line,
        tp_stages,
        age,
        trigger.description(),
        &tx_signature[..tx_signature.len().min(20)],
        time_note,
    )
}

/// Format notifikasi buy
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
        💰 Modal: **{:.4} SOL**\n\
        💵 Harga Masuk: **${:.8}**\n\
        ⭐ Skor: **{:.1}/100**\n\n\
        🔗 TX: `{}`\n\n\
        ═══════════════════════════════\n\
        ⚠️ Trading otomatis — kelola risiko dengan baik",
        name, symbol, token_address,
        amount_sol, price_usd, score,
        &tx_signature[..tx_signature.len().min(20)]
    )
}
