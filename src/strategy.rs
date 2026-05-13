// ============================================================
// AUTO BUY STRATEGY - Strategi pembelian otomatis
// ============================================================

use crate::positions::Position;

// ============================================================
// CONFIG TRADING (bisa diubah sesuai kebutuhan)
// ============================================================
#[derive(Debug, Clone)]
pub struct TradingConfig {
    pub trading_enabled: bool,
    pub max_position_sol: f64,
    pub take_profit_percent: f64,
    pub stop_loss_percent: f64,
    pub trailing_start_percent: f64,
    pub trailing_distance_percent: f64,
    pub min_score_to_buy: f64,
    pub min_liquidity_usd: f64,
    pub default_slippage: f64,
    pub max_positions: usize,
}

impl TradingConfig {
    pub fn from_env() -> Self {
        let trading_enabled = std::env::var("TRADING_ENABLED")
            .map(|v| v == "true" || v == "1")
            .unwrap_or(false);

        let max_position_sol = std::env::var("MAX_POSITION_SOL")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.5);

        let take_profit_percent = std::env::var("TAKE_PROFIT_PERCENT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(40.0);

        let stop_loss_percent = std::env::var("STOP_LOSS_PERCENT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(15.0);

        let trailing_start_percent = std::env::var("TRAILING_START_PERCENT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(20.0);

        let trailing_distance_percent = std::env::var("TRAILING_DISTANCE_PERCENT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(5.0);

        let min_score_to_buy = std::env::var("MIN_SCORE_TO_BUY")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(85.0);

        let min_liquidity_usd = std::env::var("MIN_LIQUIDITY_USD")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(10_000.0);

        let default_slippage = std::env::var("DEFAULT_SLIPPAGE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1.0);

        let max_positions = std::env::var("MAX_POSITIONS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(5);

        Self {
            trading_enabled,
            max_position_sol,
            take_profit_percent,
            stop_loss_percent,
            trailing_start_percent,
            trailing_distance_percent,
            min_score_to_buy,
            min_liquidity_usd,
            default_slippage,
            max_positions,
        }
    }

    /// Default config (untuk testing tanpa .env)
    pub fn default_safe() -> Self {
        Self {
            trading_enabled: false,       // SAFETY: default false!
            max_position_sol: 0.5,
            take_profit_percent: 40.0,
            stop_loss_percent: 15.0,
            trailing_start_percent: 20.0,
            trailing_distance_percent: 5.0,
            min_score_to_buy: 85.0,
            min_liquidity_usd: 10_000.0,
            default_slippage: 1.0,
            max_positions: 5,
        }
    }
}

// ============================================================
// TOKEN SIGNAL - Input dari analisis ke strategi buy
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
// BUY DECISION - Hasil evaluasi sinyal beli
// ============================================================

#[derive(Debug)]
pub enum BuyDecision {
    Buy { amount_sol: f64, reason: String },
    Skip { reason: String },
}

// ============================================================
// FUNGSI STRATEGI BUY
// ============================================================

/// Evaluasi apakah token layak dibeli
/// Mengembalikan BuyDecision dengan jumlah SOL atau alasan skip
pub fn evaluate_buy_signal(
    signal: &BuySignal,
    config: &TradingConfig,
    existing_positions: &std::collections::HashMap<String, Position>,
) -> BuyDecision {
    // 1. Cek apakah trading enabled
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

    // 3. Cek apakah sudah punya posisi untuk token ini
    if existing_positions.contains_key(&signal.token_address) {
        return BuyDecision::Skip {
            reason: format!("Sudah punya posisi untuk {}", signal.symbol),
        };
    }

    // 4. Cek jumlah posisi aktif (batas maksimal)
    if existing_positions.len() >= config.max_positions {
        return BuyDecision::Skip {
            reason: format!(
                "Sudah mencapai batas maksimal {} posisi aktif",
                config.max_positions
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
            reason: "Mint authority belum direvoke - risiko terlalu tinggi".to_string(),
        };
    }

    // 7. Hitung ukuran posisi berdasarkan skor
    // Formula: (score - 75) / 25 * max_position_sol
    // Skor 85 -> (85-75)/25 * max = 40% dari max
    // Skor 90 -> (90-75)/25 * max = 60% dari max
    // Skor 100 -> (100-75)/25 * max = 100% dari max
    let score_multiplier = ((signal.total_score - 75.0) / 25.0).max(0.0).min(1.0);
    let position_size = score_multiplier * config.max_position_sol;

    // Minimum position size: 0.05 SOL
    let position_size = position_size.max(0.05);

    let reason = format!(
        "Skor {:.1}/100, likuiditas ${:.0}, ukuran posisi {:.3} SOL",
        signal.total_score, signal.liquidity_usd, position_size
    );

    BuyDecision::Buy {
        amount_sol: position_size,
        reason,
    }
}

/// Log detail keputusan buy ke console
pub fn log_buy_decision(signal: &BuySignal, decision: &BuyDecision) {
    match decision {
        BuyDecision::Buy { amount_sol, reason } => {
            println!(
                "[AUTO BUY] ✅ BELI {} ({}) - {:.4} SOL | Alasan: {}",
                signal.symbol, &signal.token_address[..8], amount_sol, reason
            );
        }
        BuyDecision::Skip { reason } => {
            println!(
                "[AUTO BUY] ⏭ SKIP {} ({}) | Alasan: {}",
                signal.symbol, &signal.token_address[..8], reason
            );
        }
    }
}
