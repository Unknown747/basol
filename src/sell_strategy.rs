// ============================================================
// AUTO SELL STRATEGY - Take profit, stop loss, trailing stop
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
    ManualSell,
}

impl SellTrigger {
    pub fn description(&self) -> String {
        match self {
            SellTrigger::TakeProfit { profit_percent } => {
                format!("TAKE PROFIT +{:.1}%", profit_percent)
            }
            SellTrigger::StopLoss { loss_percent } => {
                format!("STOP LOSS -{:.1}%", loss_percent)
            }
            SellTrigger::TrailingStop { profit_percent } => {
                format!("TRAILING STOP (profit: +{:.1}%)", profit_percent)
            }
            SellTrigger::ManualSell => "MANUAL SELL".to_string(),
        }
    }

    pub fn emoji(&self) -> &str {
        match self {
            SellTrigger::TakeProfit { .. } => "💰",
            SellTrigger::StopLoss { .. } => "🛑",
            SellTrigger::TrailingStop { .. } => "📉",
            SellTrigger::ManualSell => "👤",
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

/// Evaluasi satu posisi apakah perlu dijual
/// Mengembalikan SellDecision dengan persentase jual dan trigger
pub fn evaluate_position(
    position: &mut Position,
    current_price: f64,
    config: &TradingConfig,
) -> SellDecision {
    if current_price <= 0.0 {
        return SellDecision::Hold {
            reason: "Tidak bisa ambil harga saat ini".to_string(),
        };
    }

    let profit_pct = position.profit_percent(current_price);

    // Update highest price yang pernah dicapai
    position.update_highest(current_price);

    // -------------------------------------------------------
    // 1. TAKE PROFIT - Jual semua jika profit >= target
    // -------------------------------------------------------
    if profit_pct >= config.take_profit_percent {
        println!(
            "[SELL EVAL] {} - Take profit triggered: +{:.1}% (target: +{:.1}%)",
            position.symbol, profit_pct, config.take_profit_percent
        );
        return SellDecision::Sell {
            percentage: 100.0,
            trigger: SellTrigger::TakeProfit { profit_percent: profit_pct },
        };
    }

    // -------------------------------------------------------
    // 2. STOP LOSS - Jual semua jika loss >= threshold
    // -------------------------------------------------------
    if profit_pct <= -config.stop_loss_percent {
        println!(
            "[SELL EVAL] {} - Stop loss triggered: {:.1}% (batas: -{:.1}%)",
            position.symbol, profit_pct, config.stop_loss_percent
        );
        return SellDecision::Sell {
            percentage: 100.0,
            trigger: SellTrigger::StopLoss { loss_percent: profit_pct.abs() },
        };
    }

    // -------------------------------------------------------
    // 3. TRAILING STOP - Aktif setelah profit >= trailing_start
    // -------------------------------------------------------
    if profit_pct >= config.trailing_start_percent {
        // Aktifkan trailing stop jika belum aktif
        if !position.trailing_stop_active {
            position.activate_trailing_stop(config.trailing_distance_percent);
        } else {
            // Update trailing stop ke atas jika harga naik
            position.update_trailing_stop(current_price, config.trailing_distance_percent);
        }

        // Cek apakah trailing stop kena
        if position.is_trailing_stop_hit(current_price) {
            println!(
                "[SELL EVAL] {} - Trailing stop hit! Profit saat ini: +{:.1}%, stop: ${:.8}",
                position.symbol, profit_pct, position.trailing_stop_price
            );
            return SellDecision::Sell {
                percentage: 100.0,
                trigger: SellTrigger::TrailingStop { profit_percent: profit_pct },
            };
        }
    }

    // -------------------------------------------------------
    // 4. HOLD - Tidak ada trigger yang aktif
    // -------------------------------------------------------
    SellDecision::Hold {
        reason: format!(
            "P&L: {:.1}% | TP: +{:.1}% | SL: -{:.1}% | Trailing: {}",
            profit_pct,
            config.take_profit_percent,
            config.stop_loss_percent,
            if position.trailing_stop_active {
                format!("aktif @ ${:.8}", position.trailing_stop_price)
            } else {
                "belum aktif".to_string()
            }
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
                "[SELL EVAL] ⚠️ {} - Tidak ada harga, skip evaluasi",
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

    format!(
        "{} **AUTO SELL - {}** {}\n\
        ═══════════════════════════════\n\n\
        💎 Token: **{}** `({})`\n\
        📍 `{}`\n\n\
        {} P&L: **{}{:.1}%** ({}{:.4} SOL)\n\
        💰 Masuk: **${:.8}**\n\
        💰 Keluar: **${:.8}**\n\
        📊 Modal: **{:.4} SOL**\n\
        ⏰ Durasi Posisi: **{} menit**\n\n\
        🔄 Trigger: **{}**\n\
        🔗 TX: `{}`\n\n\
        ═══════════════════════════════\n\
        ⚠️ Trading otomatis - kelola risiko dengan baik",
        trigger.emoji(),
        status_text,
        trigger.emoji(),
        position.name,
        position.symbol,
        position.token_address,
        status_emoji,
        if profit_pct >= 0.0 { "+" } else { "" },
        profit_pct,
        if profit_sol >= 0.0 { "+" } else { "" },
        profit_sol,
        position.buy_price_usd,
        current_price,
        position.amount_in_sol,
        age,
        trigger.description(),
        &tx_signature[..std::cmp::min(tx_signature.len(), 20)]
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
        ⚠️ Trading otomatis - kelola risiko dengan baik",
        name,
        symbol,
        token_address,
        amount_sol,
        price_usd,
        score,
        &tx_signature[..std::cmp::min(tx_signature.len(), 20)]
    )
}
