// ============================================================
// POSITIONS - Manajemen posisi trading aktif
// ============================================================

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    pub token_address: String,
    pub symbol: String,
    pub name: String,
    pub buy_price_usd: f64,
    /// SOL yang diinvestasikan (dikurangi saat partial sell)
    pub amount_in_sol: f64,
    /// Jumlah token yang dipegang (dikurangi saat partial sell)
    pub token_amount: f64,
    pub highest_price: f64,
    pub trailing_stop_active: bool,
    pub trailing_stop_price: f64,
    pub entry_time: DateTime<Utc>,
    pub score_at_entry: f64,

    // === 3-STAGE TAKE PROFIT ===
    /// Sudah jual sebagian di TP1?
    pub tp1_fired: bool,
    /// Sudah jual sebagian di TP2?
    pub tp2_fired: bool,
}

impl Position {
    pub fn new(
        token_address: String,
        symbol: String,
        name: String,
        buy_price_usd: f64,
        amount_in_sol: f64,
        token_amount: f64,
        score_at_entry: f64,
    ) -> Self {
        Self {
            token_address,
            symbol,
            name,
            buy_price_usd,
            amount_in_sol,
            token_amount,
            highest_price: buy_price_usd,
            trailing_stop_active: false,
            trailing_stop_price: 0.0,
            entry_time: Utc::now(),
            score_at_entry,
            tp1_fired: false,
            tp2_fired: false,
        }
    }

    pub fn profit_percent(&self, current_price: f64) -> f64 {
        if self.buy_price_usd == 0.0 { return 0.0; }
        (current_price - self.buy_price_usd) / self.buy_price_usd * 100.0
    }

    pub fn update_highest(&mut self, current_price: f64) {
        if current_price > self.highest_price {
            self.highest_price = current_price;
        }
    }

    pub fn activate_trailing_stop(&mut self, trailing_percent: f64) {
        self.trailing_stop_active = true;
        self.trailing_stop_price = self.highest_price * (1.0 - trailing_percent / 100.0);
        println!(
            "[TRAILING] {} - Trailing stop aktif di ${:.8} ({:.1}% dari puncak)",
            self.symbol, self.trailing_stop_price, trailing_percent
        );
    }

    pub fn update_trailing_stop(&mut self, current_price: f64, trailing_percent: f64) {
        if !self.trailing_stop_active { return; }
        let new_stop = current_price * (1.0 - trailing_percent / 100.0);
        if new_stop > self.trailing_stop_price {
            self.trailing_stop_price = new_stop;
            println!(
                "[TRAILING] {} - Stop naik ke ${:.8}",
                self.symbol, self.trailing_stop_price
            );
        }
    }

    pub fn is_trailing_stop_hit(&self, current_price: f64) -> bool {
        self.trailing_stop_active && current_price <= self.trailing_stop_price
    }

    pub fn age_minutes(&self) -> i64 {
        Utc::now()
            .signed_duration_since(self.entry_time)
            .num_minutes()
    }
}
