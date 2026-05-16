// ============================================================
// AUTO SELL STRATEGY - Staged take profit, stop loss, trailing, time exit
// ============================================================

use crate::positions::Position;
use crate::strategy::TradingConfig;

// ============================================================
// SELL TRIGGER - Types of sell triggers
// ============================================================

#[derive(Debug, Clone)]
pub enum SellTrigger {
    /// Partial take profit — sell a portion of the position at this level
    PartialTakeProfit {
        stage: u8,          // 1 or 2
        target_pct: f64,    // configured TP level
        profit_pct: f64,    // actual profit at trigger
        sell_pct: f64,      // percentage of position to sell
    },
    /// Full take profit — sell all remaining position
    TakeProfit { profit_percent: f64 },
    StopLoss { loss_percent: f64 },
    TrailingStop { profit_percent: f64 },
    /// Position stuck too long — exit to free up capital
    TimeExit { hold_minutes: i64, profit_percent: f64 },
}

impl SellTrigger {
    pub fn description(&self) -> String {
        match self {
            SellTrigger::PartialTakeProfit { stage, target_pct, profit_pct, sell_pct } =>
                format!(
                    "TP{stage} PARTIAL {sell_pct:.0}% — profit +{profit_pct:.1}% (target +{target_pct:.1}%)"
                ),
            SellTrigger::TakeProfit { profit_percent } =>
                format!("TAKE PROFIT FINAL +{profit_percent:.1}%"),
            SellTrigger::StopLoss { loss_percent } =>
                format!("STOP LOSS -{loss_percent:.1}%"),
            SellTrigger::TrailingStop { profit_percent } =>
                format!("TRAILING STOP (profit: +{profit_percent:.1}%)"),
            SellTrigger::TimeExit { hold_minutes, profit_percent } =>
                format!(
                    "TIME EXIT {} min | P&L: {}{:.1}%",
                    hold_minutes,
                    if *profit_percent >= 0.0 { "+" } else { "" },
                    profit_percent
                ),
        }
    }

    pub fn emoji(&self) -> &str {
        match self {
            SellTrigger::PartialTakeProfit { .. } => "🎯",
            SellTrigger::TakeProfit { .. }         => "💰",
            SellTrigger::StopLoss { .. }           => "🛑",
            SellTrigger::TrailingStop { .. }       => "📉",
            SellTrigger::TimeExit { .. }           => "⏰",
        }
    }
}

// ============================================================
// SELL DECISION
// ============================================================

#[derive(Debug)]
pub enum SellDecision {
    Sell {
        /// Percentage of the CURRENT position to sell (100.0 = all)
        percentage: f64,
        trigger: SellTrigger,
    },
    Hold {
        reason: String,
    },
}

// ============================================================
// POSITION EVALUATION FUNCTIONS
// ============================================================

/// Evaluate a single position — whether to sell, how much, and with what trigger.
///
/// **Check order:**
/// 1. Stop Loss        — cut loss now, sell ALL (100%)
/// 2. TP1 Partial      — sell tp1_sell_percent% at tp1_percent level
/// 3. TP2 Partial      — sell tp2_sell_percent% at tp2_percent level (after TP1 fired)
/// 4. Trailing Stop    — protect profit, sell remainder
/// 5. Final TP         — sell all remaining if take_profit_percent reached
/// 6. Time Exit        — idle position, exit to free up capital
/// 7. Hold             — no active trigger
///
/// **Math notes (scalping config, 0.03 SOL, ~3.5% round-trip break-even):**
/// - TP1 at +8%:  net ~+4.5% (above break-even ✅)
/// - TP2 at +15%: net ~+11.5% (good)
/// - Final TP at +25%: net ~+21.5% (strong)
/// - SL at -6%: net ~-9.5% (quick cut, preserves capital)
pub fn evaluate_position(
    position: &mut Position,
    current_price: f64,
    config: &TradingConfig,
) -> SellDecision {
    if current_price <= 0.0 {
        return SellDecision::Hold {
            reason: "Price unavailable".to_string(),
        };
    }

    let profit_pct  = position.profit_percent(current_price);
    let age_minutes = position.age_minutes();

    position.update_highest(current_price);

    // -------------------------------------------------------
    // 1. STOP LOSS — always sell 100%, cut loss immediately
    //    Never partial SL — losses can deepen!
    // -------------------------------------------------------
    if profit_pct <= -config.stop_loss_percent {
        println!(
            "[SELL EVAL] 🛑 {} - Stop loss: {:.1}% (limit: -{:.1}%)",
            position.symbol, profit_pct, config.stop_loss_percent
        );
        return SellDecision::Sell {
            percentage: 100.0,
            trigger: SellTrigger::StopLoss { loss_percent: profit_pct.abs() },
        };
    }

    // -------------------------------------------------------
    // 2. TP1 — sell first partial
    //    Lock in initial profit, ensure fees are covered.
    //    TP1 must be > break-even (~3.81% for 0.05 SOL).
    // -------------------------------------------------------
    if config.tp1_percent > 0.0
        && !position.tp1_fired
        && profit_pct >= config.tp1_percent
    {
        println!(
            "[SELL EVAL] 🎯 {} - TP1: profit +{:.1}% >= +{:.1}% | Sell {:.0}% of position",
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
    // 3. TP2 — sell second partial (only after TP1 fired)
    //    Lock in more profit, let remainder ride.
    // -------------------------------------------------------
    if config.tp2_percent > 0.0
        && position.tp1_fired
        && !position.tp2_fired
        && profit_pct >= config.tp2_percent
    {
        println!(
            "[SELL EVAL] 🎯 {} - TP2: profit +{:.1}% >= +{:.1}% | Sell {:.0}% of remainder",
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
    // 4. TRAILING STOP — active after profit >= trailing_start
    //    After TP1 or TP2 fires, trailing protects remainder.
    //    If price keeps rising → stop ratchets up.
    //    If price reverses → stop hit → sell all remainder.
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
    // 5. FINAL TP — sell all remaining if price keeps pumping.
    //    - TP1 disabled (tp1=0)          → single TP, always eligible
    //    - TP1 enabled, TP2 disabled (tp2=0) → eligible after TP1 fired
    //    - TP1+TP2 enabled               → eligible after TP2 fired (3-stage)
    // -------------------------------------------------------
    let tp3_eligible = if config.tp1_percent > 0.0 {
        if config.tp2_percent > 0.0 {
            position.tp2_fired // full 3-stage: require both TP1 and TP2 first
        } else {
            position.tp1_fired // TP1-only: just need TP1 fired before final TP
        }
    } else {
        true // single TP mode: no partial stages configured
    };

    if tp3_eligible && profit_pct >= config.take_profit_percent {
        println!(
            "[SELL EVAL] 💰 {} - Final TP: +{:.1}% (target: +{:.1}%)",
            position.symbol, profit_pct, config.take_profit_percent
        );
        return SellDecision::Sell {
            percentage: 100.0,
            trigger: SellTrigger::TakeProfit { profit_percent: profit_pct },
        };
    }

    // -------------------------------------------------------
    // 6. TIME EXIT — free up idle capital
    // -------------------------------------------------------
    if config.max_hold_minutes > 0 && age_minutes >= config.max_hold_minutes as i64
        && profit_pct < config.time_exit_threshold_pct {
            println!(
                "[SELL EVAL] ⏰ {} - Time exit: {} min | P&L: {:.1}%",
                position.symbol, age_minutes, profit_pct
            );
            return SellDecision::Sell {
                percentage: 100.0,
                trigger: SellTrigger::TimeExit { hold_minutes: age_minutes, profit_percent: profit_pct },
            };
        }

    // -------------------------------------------------------
    // 7. HOLD
    // -------------------------------------------------------
    let tp_status = if config.tp1_percent > 0.0 {
        match (position.tp1_fired, position.tp2_fired) {
            (false, _)    => format!("TP1 waiting +{:.0}%", config.tp1_percent),
            (true, false) => format!("TP1✅ TP2 waiting +{:.0}%", config.tp2_percent),
            (true, true)  => format!("TP1✅ TP2✅ Final +{:.0}%", config.take_profit_percent),
        }
    } else {
        format!("TP +{:.0}%", config.take_profit_percent)
    };

    let time_info = if config.max_hold_minutes > 0 {
        format!(" | Time: {}/{} min", age_minutes, config.max_hold_minutes)
    } else {
        format!(" | Time: {age_minutes} min")
    };

    SellDecision::Hold {
        reason: format!(
            "P&L: {:.1}% | {} | SL: -{:.1}% | Trailing: {}{}",
            profit_pct,
            tp_status,
            config.stop_loss_percent,
            if position.trailing_stop_active {
                format!("active @ ${:.8}", position.trailing_stop_price)
            } else {
                format!("activates at +{:.0}%", config.trailing_start_percent)
            },
            time_info,
        ),
    }
}

/// Evaluate all active positions — return list of those that need selling
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
                        "[SELL EVAL] {} {} {} ({:.0}%) | Entry: ${:.8} | Now: ${:.8}",
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
            println!("[SELL EVAL] ⚠️  {} - No price available, skipping", position.symbol);
        }
    }

    to_sell
}

// ============================================================
// TELEGRAM NOTIFICATION FORMATTING
// ============================================================

/// Format sell notification — supports partial TP and full close
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
            format!("📊 Sold: **{sell_pct:.0}%** of position (stage {stage}/2)"),
        ),
        _ => {
            let emoji = if profit_pct >= 0.0 { "📈" } else { "📉" };
            (emoji, "📊 Sold: **100%** (full close)".to_string())
        }
    };

    // For partial sells (TP1/TP2), profit_sol reflects only the sold fraction —
    // e.g. TP1 sells 33% so profit = amount * 0.33 * profit_pct/100
    let sold_fraction = match trigger {
        SellTrigger::PartialTakeProfit { sell_pct, .. } => sell_pct / 100.0,
        _ => 1.0,
    };
    let profit_sol = position.amount_in_sol * sold_fraction * profit_pct / 100.0;
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
            "\n⏰ _Exited due to idle position — capital freed for new opportunities_",
        _ => "",
    };

    format!(
        "{} **AUTO SELL** {}\n\
        ═══════════════════════════════\n\n\
        💎 Token: **{}** `({})`\n\
        📍 `{}`\n\n\
        {} P&L: **{}{:.1}%** ({}{:.5} SOL)\n\
        💰 Entry: **${:.8}**\n\
        💰 Now: **${:.8}**\n\
        {}{}\n\
        ⏰ Duration: **{} minutes**\n\n\
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

/// Format buy notification — shows both quoted and effective entry price,
/// slippage, and AMM impact. Mirrors paper trading's buy notification exactly
/// so you can compare paper vs live side-by-side on Telegram.
#[allow(clippy::too_many_arguments)]
pub fn format_buy_notification(
    token_address: &str,
    symbol: &str,
    name: &str,
    amount_sol: f64,
    quoted_price_usd: f64,
    effective_entry_price: f64,
    slippage_pct: f64,
    price_impact_pct: f64,
    score: f64,
    tx_signature: &str,
) -> String {
    format!(
        "🛒 **AUTO BUY** 🛒\n\
        ═══════════════════════════════\n\n\
        💎 Token: **{}** `({})`\n\
        📍 `{}`\n\n\
        💰 Capital: **{:.4} SOL**\n\
        💵 Quoted Price: **${:.8}**\n\
        💵 Effective Entry: **${:.8}**\n\
        📊 Slippage: **{:.2}%** | Impact: **{:.2}%**\n\
        ⭐ Score: **{:.1}/100**\n\n\
        🔗 TX: `{}`\n\n\
        ═══════════════════════════════\n\
        ⚠️ Automated trading — manage your risk",
        name, symbol, token_address,
        amount_sol,
        quoted_price_usd, effective_entry_price,
        slippage_pct, price_impact_pct,
        score,
        &tx_signature[..tx_signature.len().min(20)]
    )
}
