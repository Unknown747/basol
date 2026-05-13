# Basol

Bot trading Solana memecoin otomatis (Rust) — scan token baru, analisis skor, auto buy/sell di DEX (Jupiter), paper trading, backtesting, notifikasi Telegram.

## Run & Operate

- `cargo check` — typecheck (cepat, tanpa build)
- `cargo build` — build binary
- `cargo run` — jalankan bot (butuh `.env` lengkap)
- Start workflow "Basol Scanner" dari Replit untuk run otomatis

## Stack

- Rust (Cargo), async via Tokio
- Jupiter DEX aggregator (swap on-chain)
- Solana RPC (Helius)
- DexScreener API (harga & token baru)
- Telegram Bot API (notifikasi)
- No database — state disimpan di JSON (`positions.json`, `paper_trades.json`)

## Where things live

```
src/
  main.rs          — entrypoint, SolanaBot struct, scan + buy/sell loop (~2482 baris)
  strategy.rs      — TradingConfig, FeeAnalysis, scoring, buy signal
  sell_strategy.rs — SellTrigger enum (3-stage TP, trailing, time exit)
  positions.rs     — Position struct (live trading)
  paper_trading.rs — PaperTradingState, PaperPosition, evaluate_positions
  backtest.rs      — backtesting via DexScreener OHLCV
  wallet.rs        — Solana wallet & balance utils
.env.example       — template config lengkap dengan scalping preset
```

## Architecture decisions

- **Contract-first sell**: semua exit logic di `SellTrigger` enum — mudah di-backtest & di-paper test tanpa duplikasi
- **Fee-aware**: `FeeAnalysis` hitung break-even bersih termasuk network fee + slippage (break-even ~3.81% untuk 0.05 SOL)
- **State persistent**: posisi live & paper disimpan ke JSON agar survive restart
- **Partial sell**: `execute_sell(tp_stage)` mengurangi amount posisi untuk TP1/TP2, hanya hapus posisi pada full close atau TP final
- **No hot reload**: config dibaca dari ENV saat startup — restart workflow untuk apply perubahan config

## Product

- **Auto-scan**: polling DexScreener setiap 30 detik, filter token baru berdasarkan skor (liquidity, volume, holder, dll)
- **Auto-buy**: Jupiter swap jika skor ≥ MIN_SCORE_TO_BUY (scalping default: 87)
- **3-Stage TP**: TP1 (12% → jual 33%) → TP2 (20% → jual 50% sisa) → Trailing stop → TP Final (35% → jual semua)
- **Stop Loss**: 8% — selalu jual 100% sekaligus
- **Time Exit**: keluar jika posisi stuck > 40 menit dan profit < 3%
- **Paper trading**: simulasi fulltime di mode `PAPER_TRADING=true`, tanpa uang nyata
- **Backtest**: mode `BACKTEST_ONLY=true` dengan data OHLCV historis

## User preferences

- Comment & log language: English
- Scalping dengan modal kecil: MAX_POSITION_SOL=0.05, MAX_POSITIONS=2
- Selalu fee-aware — tidak ada target profit yang mengabaikan biaya transaksi

## Gotchas

- **Wajib buat `.env`** dari `.env.example` sebelum jalankan bot (tidak ada `.env` = crash saat startup)
- **`TP1_PERCENT` harus > 3.81%** (break-even bersih) — set di bawah itu = TP tidak pernah profitable
- `evaluate_positions()` dan `execute_sell()` di `paper_trading.rs` **harus** dipanggil dengan signature baru (include tp_stage & 8 TP params) — sudah diupdate di `main.rs`
- Partial sell di live trading update `pos.amount_in_sol` & `pos.token_amount` — jangan hardcode 100% di pemanggil baru
- `Position` (live) dan `PaperPosition` (paper) keduanya punya field `tp1_fired` & `tp2_fired` — harus di-reset jika posisi diclose lalu dibuka ulang

## Pointers

- Scalping preset ENV: lihat bagian `⚡ SCALPING PRESET` di `.env.example`
- 3-stage TP ENV: `TP1_PERCENT`, `TP1_SELL_PERCENT`, `TP2_PERCENT`, `TP2_SELL_PERCENT`
- Fee math detail: `src/strategy.rs` → `compute_fee_analysis()`
- Sell logic detail: `src/sell_strategy.rs` → `evaluate_sell_trigger()`
