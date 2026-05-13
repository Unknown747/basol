// ============================================================
// AUTO BUY STRATEGY - Strategi pembelian otomatis
// ============================================================

use crate::positions::Position;

// ============================================================
// KONSTANTA BIAYA TRANSAKSI
// ============================================================

/// Biaya jaringan Solana per transaksi (buy atau sell)
/// Base fee 5000 lamport + priority fee ~20000 lamport = 25000 lamport = 0.000025 SOL
pub const NETWORK_FEE_SOL: f64 = 0.000025;

/// Estimasi price impact default untuk pool kecil ($5k liquidity).
/// Formula AMM: impact = trade_usd / (pool_usd + trade_usd)
/// Untuk trade 0.05 SOL ≈ $8.5 di pool $5k: 8.5/5008.5 ≈ 0.17%
pub const DEFAULT_PRICE_IMPACT_PCT: f64 = 0.17;

// ============================================================
// ANALISIS BIAYA PER TRADE
// ============================================================

/// Hasil kalkulasi biaya lengkap untuk satu round trip trade.
///
/// Berguna untuk menampilkan ke user berapa biaya sesungguhnya,
/// bukan hanya angka TP/SL yang terlihat di layar.
#[derive(Debug, Clone)]
pub struct FeeAnalysis {
    /// Biaya masuk: slippage + price impact + network fee (dalam SOL)
    pub entry_cost_sol: f64,
    /// Biaya masuk dalam persen dari ukuran posisi
    pub entry_cost_pct: f64,
    /// Biaya keluar di harga TP (dalam SOL)
    pub exit_cost_at_tp_sol: f64,
    /// Biaya keluar dalam persen dari nilai jual
    pub exit_cost_at_tp_pct: f64,
    /// Biaya keluar di harga SL (dalam SOL)
    pub exit_cost_at_sl_sol: f64,
    /// Total biaya round trip (masuk + keluar di TP)
    pub total_roundtrip_cost_sol: f64,
    /// Kenaikan harga minimum agar balik modal (cover semua biaya)
    pub breakeven_pct: f64,
    /// Profit bersih (setelah biaya) jika TP tercapai
    pub net_profit_at_tp_sol: f64,
    /// Profit bersih dalam persen dari posisi
    pub net_profit_at_tp_pct: f64,
    /// Loss bersih (setelah biaya) jika SL tercapai
    pub net_loss_at_sl_sol: f64,
    /// Loss bersih dalam persen dari posisi
    pub net_loss_at_sl_pct: f64,
    /// Risk/Reward ratio: net_profit_at_tp / net_loss_at_sl
    pub risk_reward_ratio: f64,
    /// Win rate minimum agar EV positif
    pub min_win_rate_pct: f64,
}

/// Hitung analisis biaya lengkap untuk satu rencana trade.
///
/// **Parameter:**
/// - `amount_sol`: ukuran posisi dalam SOL
/// - `slippage_pct`: slippage yang dikonfigurasi (mis. 1.5)
/// - `take_profit_pct`: target TP (mis. 20.0)
/// - `stop_loss_pct`: batas SL (mis. 8.0)
/// - `liquidity_usd`: likuiditas pool dalam USD
/// - `sol_price_usd`: harga SOL saat ini dalam USD
pub fn compute_fee_analysis(
    amount_sol: f64,
    slippage_pct: f64,
    take_profit_pct: f64,
    stop_loss_pct: f64,
    liquidity_usd: f64,
    sol_price_usd: f64,
) -> FeeAnalysis {
    let position_usd = amount_sol * sol_price_usd;

    // --- Biaya masuk ---
    // Price impact berdasarkan ukuran trade vs likuiditas pool (formula AMM)
    let entry_impact_pct = if liquidity_usd > 0.0 {
        (position_usd / (liquidity_usd + position_usd)) * 100.0
    } else {
        DEFAULT_PRICE_IMPACT_PCT
    };
    let entry_slippage_sol  = amount_sol * slippage_pct / 100.0;
    let entry_impact_sol    = amount_sol * entry_impact_pct / 100.0;
    let entry_cost_sol      = entry_slippage_sol + entry_impact_sol + NETWORK_FEE_SOL;
    let entry_cost_pct      = entry_cost_sol / amount_sol * 100.0;

    // --- Biaya keluar di TP ---
    let tp_value_sol        = amount_sol * (1.0 + take_profit_pct / 100.0);
    let tp_value_usd        = tp_value_sol * sol_price_usd;
    let exit_impact_at_tp   = (tp_value_usd / (liquidity_usd + tp_value_usd)) * 100.0;
    let exit_slip_at_tp_sol = tp_value_sol * slippage_pct / 100.0;
    let exit_imp_at_tp_sol  = tp_value_sol * exit_impact_at_tp / 100.0;
    let exit_cost_at_tp_sol = exit_slip_at_tp_sol + exit_imp_at_tp_sol + NETWORK_FEE_SOL;
    let exit_cost_at_tp_pct = exit_cost_at_tp_sol / tp_value_sol * 100.0;

    // --- Biaya keluar di SL ---
    let sl_value_sol        = amount_sol * (1.0 - stop_loss_pct / 100.0);
    let sl_value_usd        = sl_value_sol * sol_price_usd;
    let exit_impact_at_sl   = (sl_value_usd / (liquidity_usd + sl_value_usd)) * 100.0;
    let exit_slip_at_sl_sol = sl_value_sol * slippage_pct / 100.0;
    let exit_imp_at_sl_sol  = sl_value_sol * exit_impact_at_sl / 100.0;
    let exit_cost_at_sl_sol = exit_slip_at_sl_sol + exit_imp_at_sl_sol + NETWORK_FEE_SOL;

    // --- Round trip total ---
    let total_roundtrip_cost_sol = entry_cost_sol + exit_cost_at_tp_sol;

    // Breakeven = berapa persen token harus naik agar menutup semua biaya
    // (entry cost + exit cost di harga entry) / amount_sol × 100
    // Estimasi konservatif: pakai total_roundtrip / amount_sol
    let breakeven_pct = total_roundtrip_cost_sol / amount_sol * 100.0;

    // --- Net profit di TP ---
    let gross_profit_at_tp  = amount_sol * take_profit_pct / 100.0;
    let net_profit_at_tp_sol = gross_profit_at_tp - total_roundtrip_cost_sol;
    let net_profit_at_tp_pct = net_profit_at_tp_sol / amount_sol * 100.0;

    // --- Net loss di SL ---
    // Total loss = SL% dari posisi + biaya masuk (sudah dibayar) + biaya keluar di SL
    let gross_loss_at_sl    = amount_sol * stop_loss_pct / 100.0;
    let net_loss_at_sl_sol  = gross_loss_at_sl + entry_cost_sol + exit_cost_at_sl_sol;
    let net_loss_at_sl_pct  = net_loss_at_sl_sol / amount_sol * 100.0;

    // --- Risk/Reward ---
    let risk_reward_ratio = if net_loss_at_sl_sol > 0.0 {
        net_profit_at_tp_sol / net_loss_at_sl_sol
    } else {
        0.0
    };

    // --- Win rate minimum untuk EV positif ---
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
// CONFIG TRADING
// ============================================================

#[derive(Debug, Clone)]
pub struct TradingConfig {
    pub trading_enabled: bool,
    pub max_position_sol: f64,
    /// Ukuran posisi minimum (default: 10% dari max, minimal 0.01 SOL)
    pub min_position_sol: f64,
    pub take_profit_percent: f64,
    pub stop_loss_percent: f64,
    pub trailing_start_percent: f64,
    pub trailing_distance_percent: f64,
    pub min_score_to_buy: f64,
    pub min_liquidity_usd: f64,
    pub default_slippage: f64,
    pub max_positions: usize,
    /// Jika > 0: keluar otomatis jika posisi stuck lebih dari X menit.
    /// Ideal untuk scalping agar modal tidak nganggur.
    pub max_hold_minutes: u64,
    /// Threshold P&L untuk time exit.
    /// Contoh 3.0: jika profit < 3% setelah max_hold_minutes, keluar.
    pub time_exit_threshold_pct: f64,
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
        }
    }

    /// Config default aman (untuk testing tanpa .env)
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
        }
    }

    /// Preset scalping untuk modal kecil (0.05–0.2 SOL per trade).
    ///
    /// **Dasar matematis (0.05 SOL, pool $5k, slippage 1.5%):**
    /// - Break-even: +3.81% (biaya round trip)
    /// - Net profit di TP 20%: +16.2% bersih
    /// - Net loss di SL 8%:    -11.3% bersih
    /// - Risk/Reward: 1.43 (positif dengan win rate 42%+)
    /// - Trailing mulai 12%: lindungi profit sebelum TP
    /// - Time exit 40 menit: bebaskan modal yang nganggur
    pub fn scalping_preset() -> Self {
        Self {
            trading_enabled: false,
            max_position_sol: 0.05,
            min_position_sol: 0.02,
            take_profit_percent: 20.0,
            stop_loss_percent: 8.0,
            trailing_start_percent: 12.0,
            trailing_distance_percent: 3.0,
            min_score_to_buy: 87.0,
            min_liquidity_usd: 5_000.0,
            default_slippage: 1.5,
            max_positions: 2,
            max_hold_minutes: 40,
            time_exit_threshold_pct: 3.0,
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
// FUNGSI STRATEGI BUY
// ============================================================

/// Evaluasi apakah token layak dibeli dan berapa ukuran posisinya.
///
/// **Formula position sizing (score-based):**
/// - Skor 87 → 48% dari max_position_sol
/// - Skor 93 → 72% dari max_position_sol
/// - Skor 100 → 100% dari max_position_sol
/// - Hasil di-clamp ke [min_position_sol, max_position_sol]
///
/// **Catatan untuk modal kecil:**
/// Dengan min_position_sol=0.02 dan max=0.05, maka skor 87 → max(0.024, 0.02) = 0.024 SOL.
/// Tapi dianjurkan set MIN_POSITION_SOL=0.05 agar konsisten 0.05 SOL per trade.
pub fn evaluate_buy_signal(
    signal: &BuySignal,
    config: &TradingConfig,
    existing_positions: &std::collections::HashMap<String, Position>,
) -> BuyDecision {
    // 1. Cek trading enabled
    if !config.trading_enabled {
        return BuyDecision::Skip {
            reason: "Trading dinonaktifkan (TRADING_ENABLED=false)".to_string(),
        };
    }

    // 2. Cek skor minimum
    if signal.total_score < config.min_score_to_buy {
        return BuyDecision::Skip {
            reason: format!(
                "Skor {:.1} di bawah minimum {:.1}",
                signal.total_score, config.min_score_to_buy
            ),
        };
    }

    // 3. Cek duplikasi posisi
    if existing_positions.contains_key(&signal.token_address) {
        return BuyDecision::Skip {
            reason: format!("Sudah punya posisi untuk {}", signal.symbol),
        };
    }

    // 4. Cek batas posisi aktif
    if existing_positions.len() >= config.max_positions {
        return BuyDecision::Skip {
            reason: format!(
                "Sudah {} posisi aktif (maks: {})",
                existing_positions.len(), config.max_positions
            ),
        };
    }

    // 5. Cek likuiditas minimum
    if signal.liquidity_usd < config.min_liquidity_usd {
        return BuyDecision::Skip {
            reason: format!(
                "Likuiditas ${:.0} di bawah minimum ${:.0}",
                signal.liquidity_usd, config.min_liquidity_usd
            ),
        };
    }

    // 6. Cek mint authority revoked
    if !signal.mint_authority_revoked {
        return BuyDecision::Skip {
            reason: "Mint authority belum direvoke — risiko rugpull tinggi".to_string(),
        };
    }

    // 7. Hitung ukuran posisi berdasarkan skor
    //
    //   multiplier = (score - 75) / 25   → range [0.0, 1.0]
    //   Skor 87  → 0.48 × max
    //   Skor 93  → 0.72 × max
    //   Skor 100 → 1.00 × max
    //   Di-clamp ke [min_position_sol, max_position_sol]
    let score_multiplier = ((signal.total_score - 75.0) / 25.0).clamp(0.0, 1.0);
    let raw_size = score_multiplier * config.max_position_sol;
    let position_size = raw_size
        .max(config.min_position_sol)
        .min(config.max_position_sol);

    // 8. Hitung analisis biaya untuk posisi ini
    // Gunakan sol_price default 170.0 jika tidak tersedia dari signal
    let sol_price_estimate = 170.0_f64; // fallback; idealnya dari main bot state
    let fee_analysis = compute_fee_analysis(
        position_size,
        config.default_slippage,
        config.take_profit_percent,
        config.stop_loss_percent,
        signal.liquidity_usd,
        sol_price_estimate,
    );

    let reason = format!(
        "Skor {:.1}/100 | Liq ${:.0} | {:.4} SOL | R:R {:.2} | Breakeven +{:.1}%",
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

/// Log detail keputusan buy ke console, termasuk analisis biaya lengkap
pub fn log_buy_decision(signal: &BuySignal, decision: &BuyDecision) {
    let addr_short = &signal.token_address[..signal.token_address.len().min(8)];
    match decision {
        BuyDecision::Buy { amount_sol, reason, fee_analysis: fa } => {
            println!(
                "[BUY EVAL] ✅ BELI {} ({}) | {}",
                signal.symbol, addr_short, reason
            );
            println!(
                "[BUY EVAL]    Biaya masuk  : {:.5} SOL ({:.2}%)",
                fa.entry_cost_sol, fa.entry_cost_pct
            );
            println!(
                "[BUY EVAL]    Biaya keluar : {:.5} SOL ({:.2}% dari nilai jual)",
                fa.exit_cost_at_tp_sol, fa.exit_cost_at_tp_pct
            );
            println!(
                "[BUY EVAL]    Break-even   : harga harus naik minimal +{:.2}%",
                fa.breakeven_pct
            );
            println!(
                "[BUY EVAL]    Net jika TP  : +{:.5} SOL (+{:.2}% bersih)",
                fa.net_profit_at_tp_sol, fa.net_profit_at_tp_pct
            );
            println!(
                "[BUY EVAL]    Net jika SL  : -{:.5} SOL (-{:.2}% bersih)",
                fa.net_loss_at_sl_sol, fa.net_loss_at_sl_pct
            );
            println!(
                "[BUY EVAL]    R:R ratio    : {:.2} | Win rate min: {:.1}%",
                fa.risk_reward_ratio, fa.min_win_rate_pct
            );
            println!(
                "[BUY EVAL]    Ukuran posisi: {:.4} SOL | Total round trip cost: {:.5} SOL",
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
