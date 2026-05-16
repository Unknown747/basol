# Basol

Bot trading Solana memecoin otomatis (Rust) — scan token baru, analisis skor, auto buy/sell di DEX (Jupiter), paper trading, backtesting, notifikasi Telegram.

## Run & Operate

- `cargo check` — typecheck (cepat, tanpa build)
- `cargo build` — build binary
- `cargo run` — jalankan bot
- `cargo run -- --backtest` — backtest strategi dengan data historis
- `cargo run -- --compare` — bandingkan preset strategi sekaligus
- `cargo run -- --help` — lihat semua opsi dan env vars
- `bash update.sh` — git pull + rebuild + restart (auto-detect Replit vs VPS)
- Start workflow "Basol Scanner" dari Replit untuk run otomatis

## Konfigurasi

### Secrets (API keys sensitif) — simpan di Replit Secrets:
- `HELIUS_API_KEY` — dari helius.dev
- `TELEGRAM_BOT_TOKEN` — dari @BotFather
- `TELEGRAM_CHAT_ID` — ID chat tujuan notifikasi
- `WALLET_PRIVATE_KEY` — hanya untuk live trading

### Setting strategi — edit `config.env` di root project:
- Semua nilai TP, SL, posisi, circuit breaker, dll ada di sini
- Edit lalu restart workflow untuk apply
- Template tersedia di `config.env.example`
- File ini di-gitignore sehingga aman untuk kustomisasi lokal

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
  main.rs          — entrypoint, SolanaBot struct, semua fitur v3.0
  strategy.rs      — TradingConfig, FeeAnalysis, scoring, buy signal
  sell_strategy.rs — SellTrigger enum (3-stage TP, trailing, time exit)
  positions.rs     — Position struct (live trading)
  paper_trading.rs — PaperTradingState, PaperPosition, evaluate_positions
  backtest.rs      — backtesting via DexScreener OHLCV
  wallet.rs        — Solana wallet & balance utils (Jupiter v6)

config.env         — Konfigurasi strategi utama (edit ini untuk ubah setting)
config.env.example — Template konfigurasi (salin ke config.env jika belum ada)
.env.example       — Template secrets (HELIUS, TELEGRAM, WALLET)
update.sh          — git pull + rebuild + restart (Replit & VPS)
install.sh         — one-click install untuk Ubuntu VPS (systemd)
```

## Architecture decisions

- **Contract-first sell**: semua exit logic di `SellTrigger` enum — mudah di-backtest & di-paper test tanpa duplikasi
- **Fee-aware**: `FeeAnalysis` hitung break-even bersih termasuk network fee + slippage (break-even ~3.81% untuk 0.03 SOL)
- **State persistent**: posisi live & paper disimpan ke JSON agar survive restart
- **Partial sell**: `execute_sell(tp_stage)` mengurangi amount posisi untuk TP1/TP2
- **No hot reload**: config dibaca dari ENV saat startup — restart workflow untuk apply perubahan
- **Circuit breaker**: pause buy-side saja, sell monitoring tetap jalan
- **Dynamic score**: `dynamic_min_score` di-adjust setiap jam dari rolling win rate
- **Fast Telegram poll**: Telegram di-poll setiap 3 detik (bukan per siklus scan 30 detik)
- **config.env priority**: `config.env` override Replit platform env vars via `dotenv_override()`

## Product — Fitur Aktif

- **Auto-scan**: polling DexScreener setiap 30 detik, filter token baru berdasarkan skor
- **Auto-buy**: Jupiter swap jika skor ≥ MIN_SCORE_TO_BUY (scalping default: 89)
- **3-Stage TP**: TP1 (12% → jual 33%) → TP2 (20% → jual 50% sisa) → Trailing → TP Final (35%)
- **Stop Loss**: 8% — selalu jual 100% sekaligus
- **Time Exit**: keluar jika posisi stuck > 40 menit dan profit < 3%
- **Paper trading**: simulasi fulltime, tanpa uang nyata
- **Backtest**: mode `--backtest` dengan data OHLCV historis

### v3.0 — Smart Protection Features

- **Circuit Breaker**: pause beli otomatis setelah N loss berturut
- **Peak Hours Filter**: hanya beli 13:00-17:00 UTC dan 20:00-00:00 UTC
- **Momentum Filter**: skip token yang sudah pump >30% dalam 1 jam
- **Token Blacklist**: `/blacklist <addr>` via Telegram
- **Auto-Adjust Score**: evaluasi 20 trade terakhir tiap jam, adjust threshold
- **Helius Key Rotation**: auto-rotate saat 429 rate limit
- **Telegram Commands**: `/status`, `/pause`, `/resume`, `/trades`, `/score`, `/blacklist`, `/helius`
- **Inline Button**: tombol ⏸ Pause dan 📈 Stats di notifikasi buy berfungsi
- **Daily/Weekly Report**: laporan otomatis ke Telegram

## User preferences

- Comment & log language: English
- Scalping dengan modal kecil: MAX_POSITION_SOL=0.03, MAX_POSITIONS=2
- Selalu fee-aware — tidak ada target profit yang mengabaikan biaya transaksi

## Gotchas

- **Wajib isi Replit Secrets**: HELIUS_API_KEY, TELEGRAM_BOT_TOKEN, TELEGRAM_CHAT_ID wajib ada
- **`config.env` = sumber konfigurasi utama**: edit file ini untuk ubah strategi, lalu restart workflow
- **`TP1_PERCENT` harus > 3.81%** (break-even bersih) — di bawah itu = TP tidak pernah profitable
- **Circuit breaker hanya pause beli** — sell positions tetap dievaluasi. Ini by design.
- **`blacklisted_tokens`** disimpan di `bot_data.json` — survives restart
- **`dynamic_min_score`** reset ke base config saat bot restart

## Pointers

- Helius key rotation: tambah `HELIUS_API_KEY_2`, `HELIUS_API_KEY_3` di Replit Secrets
- Telegram polling: setiap 3 detik (bukan 30 detik) — respon command cepat
- Callback query inline button: di-handle di `poll_telegram_commands()` di `main.rs`
- Fee math detail: `src/strategy.rs` → `compute_fee_analysis()`
- Sell logic detail: `src/sell_strategy.rs` → `evaluate_sell_trigger()`
- Circuit breaker logic: `src/main.rs` → `handle_trade_result()`
- Score auto-adjust: `src/main.rs` → `adjust_dynamic_score()`
- Telegram commands: `src/main.rs` → `poll_telegram_commands()`
