# Basol

Bot trading Solana memecoin otomatis (Rust) — scan token baru, analisis skor, auto buy/sell di DEX (Jupiter), paper trading, backtesting, notifikasi Telegram.

## Run & Operate

- `cargo check` — typecheck (cepat, tanpa build)
- `cargo build` — build binary
- `cargo run` — jalankan bot (butuh `.env` lengkap)
- `cargo run -- --backtest` — backtest strategi dengan data historis
- `cargo run -- --compare` — bandingkan 8 preset strategi sekaligus
- `cargo run -- --help` — lihat semua opsi dan env vars
- Start workflow "Basol Scanner" dari Replit untuk run otomatis

## Stack

- Rust (Cargo) v3.0.0, async via Tokio
- Jupiter DEX aggregator v6 (swap on-chain)
- Solana RPC (Helius)
- DexScreener API (harga & token baru)
- Telegram Bot API (notifikasi + perintah interaktif)
- No database — state disimpan di JSON (`bot_data.json`, `paper_state.json`)

## Where things live

```
src/
  main.rs          — entrypoint, SolanaBot struct, semua fitur baru v3.0 (~2977 baris)
  strategy.rs      — TradingConfig, FeeAnalysis, scoring, buy signal
  sell_strategy.rs — SellTrigger enum (3-stage TP, trailing, time exit)
  positions.rs     — Position struct (live trading)
  paper_trading.rs — PaperTradingState, PaperPosition, evaluate_positions
  backtest.rs      — backtesting via DexScreener OHLCV
  wallet.rs        — Solana wallet & balance utils (Jupiter v6)
.env.example       — template config lengkap dengan scalping preset + fitur v3.0
install.sh         — one-click install/update untuk Ubuntu VPS (systemd)
```

## Architecture decisions

- **Contract-first sell**: semua exit logic di `SellTrigger` enum — mudah di-backtest & di-paper test tanpa duplikasi
- **Fee-aware**: `FeeAnalysis` hitung break-even bersih termasuk network fee + slippage (break-even ~3.81% untuk 0.03 SOL)
- **State persistent**: posisi live & paper disimpan ke JSON agar survive restart (`bot_data.json` includes `blacklisted_tokens`)
- **Partial sell**: `execute_sell(tp_stage)` mengurangi amount posisi untuk TP1/TP2, hanya hapus posisi pada full close
- **No hot reload**: config dibaca dari ENV saat startup — restart workflow untuk apply perubahan config
- **Circuit breaker**: pause buy-side saja, sell monitoring tetap jalan — tidak ada posisi yang terlantar
- **Dynamic score**: `dynamic_min_score` di-adjust setiap jam dari rolling win rate, propagasi ke live + paper

## Product — Fitur Aktif

- **Auto-scan**: polling DexScreener setiap 30 detik, filter token baru berdasarkan skor
- **Auto-buy**: Jupiter swap jika skor ≥ MIN_SCORE_TO_BUY (scalping default: 89)
- **3-Stage TP**: TP1 (12% → jual 33%) → TP2 (20% → jual 50% sisa) → Trailing stop → TP Final (35% → jual semua)
- **Stop Loss**: 8% — selalu jual 100% sekaligus
- **Time Exit**: keluar jika posisi stuck > 40 menit dan profit < 3%
- **Paper trading**: simulasi fulltime di mode `PAPER_TRADING_ENABLED=true`, tanpa uang nyata
- **Backtest**: mode `--backtest` dengan data OHLCV historis

### v3.0 — Smart Protection Features

- **Circuit Breaker**: pause beli otomatis setelah N loss berturut (`CIRCUIT_BREAKER_LOSSES=3`, pause `CIRCUIT_BREAKER_PAUSE_HOURS=2`). Sell tetap jalan. Auto-reset setelah timeout atau `/resume`.
- **Peak Hours Filter**: `PEAK_HOURS_ONLY=true` — hanya beli 13:00-17:00 UTC dan 20:00-00:00 UTC. Sell 24 jam.
- **Momentum Filter**: `MOMENTUM_MAX_PCT=30.0` — skip token yang sudah pump >30% dalam 1 jam (terlambat entry).
- **Token Blacklist**: `/blacklist <addr>` via Telegram — skip token permanen, disimpan ke `bot_data.json`.
- **Auto-Adjust Score**: setiap jam, bot evaluasi 20 trade terakhir. Win rate <40% → naikkan threshold +2 (max 95). Win rate >60% → turunkan -1 (min = base config).
- **Helius Key Rotation**: auto-rotate ke key berikutnya saat 429 rate limit. Tambah key via `HELIUS_API_KEY_2`, `HELIUS_API_KEY_3`, dst. Test semua key via `/helius`.
- **Telegram Commands**: `/status`, `/pause`, `/resume`, `/trades`, `/score`, `/blacklist`, `/helius`
- **Daily Report**: kirim ke Telegram setiap tengah malam UTC — balance, ROI, win rate, best/worst trade.
- **Weekly Report**: kirim tiap Senin 06:00 UTC — summary mingguan + semua stats.

## User preferences

- Comment & log language: English
- Scalping dengan modal kecil: MAX_POSITION_SOL=0.03, MAX_POSITIONS=2
- Selalu fee-aware — tidak ada target profit yang mengabaikan biaya transaksi

## Gotchas

- **Wajib buat `.env`** dari `.env.example` sebelum jalankan bot (tidak ada `.env` = crash saat startup)
- **`TP1_PERCENT` harus > 3.81%** (break-even bersih) — set di bawah itu = TP tidak pernah profitable
- **Circuit breaker hanya pause beli** — sell positions tetap dievaluasi. Ini by design.
- **`blacklisted_tokens`** disimpan di `bot_data.json` — survives restart, tapi kalau file dihapus hilang juga
- **`dynamic_min_score`** reset ke base config saat bot restart — start dari nilai `MIN_SCORE_TO_BUY`
- `evaluate_positions()` dan `execute_sell()` di `paper_trading.rs` dipanggil dengan signature 3-stage TP (8 params) — sudah benar di `main.rs`
- Partial sell di live trading update `pos.amount_in_sol` & `pos.token_amount` — jangan hardcode 100%
- `Position` (live) dan `PaperPosition` (paper) keduanya punya field `tp1_fired` & `tp2_fired` — di-reset jika posisi diclose lalu dibuka ulang

## Pointers

- Helius key rotation: tambah `HELIUS_API_KEY_2`, `HELIUS_API_KEY_3`, ..., `HELIUS_API_KEY_10` di Replit Secrets — bot auto-rotate saat 429. Atau gunakan `HELIUS_API_KEYS` berisi comma-separated keys.
- Scalping preset ENV: lihat bagian `⚡ SCALPING PRESET` di `.env.example`
- 3-stage TP ENV: `TP1_PERCENT`, `TP1_SELL_PERCENT`, `TP2_PERCENT`, `TP2_SELL_PERCENT`
- v3.0 protection ENV: `CIRCUIT_BREAKER_LOSSES`, `CIRCUIT_BREAKER_PAUSE_HOURS`, `PEAK_HOURS_ONLY`, `MOMENTUM_MAX_PCT`
- Fee math detail: `src/strategy.rs` → `compute_fee_analysis()`
- Sell logic detail: `src/sell_strategy.rs` → `evaluate_sell_trigger()`
- Circuit breaker logic: `src/main.rs` → `handle_trade_result()`
- Score auto-adjust: `src/main.rs` → `adjust_dynamic_score()`
- Telegram commands: `src/main.rs` → `poll_telegram_commands()`
