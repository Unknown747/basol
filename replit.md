# Basol

Bot trading Solana memecoin otomatis (Rust v3.0) — scan token baru, analisis skor, auto buy/sell di DEX (Jupiter), paper trading, backtesting, notifikasi Telegram.

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

### Hierarki konfigurasi (penting!)

Ada **dua tempat** konfigurasi, masing-masing untuk tujuan berbeda:

| File / Tempat | Isi | Edit di mana |
|---|---|---|
| **Replit Secrets** | API keys sensitif (Helius, Telegram, Wallet) | Tab Secrets di Replit |
| **`config.env`** | Semua setting strategi (TP, SL, skor, posisi, dll) | Edit file ini langsung |

`config.env` selalu menang (override) — **satu-satunya tempat untuk edit strategi.**

### Secrets (API keys sensitif)

Di **Replit**: simpan di tab Secrets (lebih aman)
Di **VPS**: salin ke `.env` (dibuat otomatis oleh `install.sh`, atau buat manual dari `.env.example`)

- `HELIUS_API_KEY` — dari helius.dev (wajib)
- `TELEGRAM_BOT_TOKEN` — dari @BotFather (wajib)
- `TELEGRAM_CHAT_ID` — ID chat tujuan notifikasi (wajib)
- `WALLET_PRIVATE_KEY` — hanya untuk live trading (opsional)
- `HELIUS_API_KEY_2`, `HELIUS_API_KEY_3` — key rotation opsional

### Setting strategi — edit `config.env` di root project:
- Semua nilai TP, SL, posisi, circuit breaker, dll ada di sini
- Edit lalu restart workflow untuk apply
- Template tersedia di `config.env.example`
- File ini di-gitignore sehingga aman untuk kustomisasi lokal

## Stack

- Rust (Cargo) v3.0.0, async via Tokio
- Jupiter DEX aggregator v6 (swap on-chain)
- Solana RPC (Helius, 5 key auto-rotation)
- DexScreener API (harga & token baru)
- Telegram Bot API (notifikasi + perintah interaktif)
- No database — state disimpan di JSON (`bot_data.json`, `paper_state.json`)

## Where things live

```
src/
  main.rs          — entrypoint, SolanaBot struct, semua fitur v3.0
  strategy.rs      — TradingConfig, FeeAnalysis, scoring, buy signal, position sizing
  sell_strategy.rs — SellTrigger enum (3-stage TP, trailing, time exit, break-even)
  positions.rs     — Position struct (live trading)
  paper_trading.rs — PaperTradingState, PaperPosition, evaluate_positions
  backtest.rs      — backtesting via DexScreener OHLCV (mirrors live exit logic)
  wallet.rs        — Solana wallet & balance utils (Jupiter v6)

config.env         — Konfigurasi strategi utama (edit ini untuk ubah setting)
config.env.example — Template konfigurasi (salin ke config.env jika belum ada)
.env               — Secrets lokal (auto-generated, di-gitignore)
.env.example       — Template secrets (HELIUS, TELEGRAM, WALLET)
update.sh          — git pull + rebuild + restart (Replit & VPS)
install.sh         — one-click install untuk Ubuntu VPS (systemd)
```

## Architecture decisions

- **Contract-first sell**: semua exit logic di `SellTrigger` enum — mudah di-backtest & di-paper test tanpa duplikasi
- **Fee-aware**: `FeeAnalysis` hitung break-even bersih termasuk network fee + slippage (~3.5% untuk 0.03 SOL)
- **Identical config**: paper dan live baca ENV vars yang sama — tidak ada duplikasi parameter
- **State persistent**: posisi live & paper + daily loss counter disimpan ke JSON agar survive restart
- **Partial sell**: `execute_sell(tp_stage)` mengurangi amount posisi untuk TP1/TP2
- **No hot reload**: config dibaca dari ENV saat startup — restart workflow untuk apply perubahan
- **Circuit breaker**: pause buy-side saja, sell monitoring tetap jalan
- **Dynamic score**: `dynamic_min_score` di-adjust setiap jam dari rolling win rate, capped +10 di atas base
- **Fast Telegram poll**: Telegram di-poll setiap 3 detik (bukan per siklus scan 30 detik)
- **config.env priority**: `config.env` override Replit platform env vars via `dotenv_override()`
- **Code defaults = config.env**: semua default di `from_env()` cocok dengan `config.env` — jika file hilang, bot tetap jalan dengan strategi yang benar
- **Score-based position sizing**: formula `(score - min_score) / (100 - min_score)` — skalabel dengan threshold berapapun
- **Daily loss persisted**: `daily_loss_sol` dan `daily_loss_date` disimpan di `bot_data.json` — restart tidak bisa bypass daily limit

## Product — Fitur Aktif

- **Auto-scan**: polling DexScreener setiap 30 detik, filter token baru berdasarkan skor
- **Auto-buy**: Jupiter swap jika skor ≥ MIN_SCORE_TO_BUY (scalping default: 45)
- **3-Stage TP**: TP1 (+8% → jual 33%) → TP2 (+15% → jual 50% sisa) → Trailing +16% → TP Final (+25%)
- **Stop Loss**: −5% — selalu jual 100% sekaligus
- **Break-Even Stop**: setelah TP1 fired, SL sisa posisi pindah ke 0% (entry price) — trade tidak bisa net loss
- **Time Exit**: keluar jika posisi stuck > 25 menit dan profit < 1.5%
- **Paper trading**: simulasi fulltime, tanpa uang nyata
- **Backtest**: mode `--backtest` dengan data DexScreener historis (termasuk break-even stop logic)

### v3.0 — Smart Protection Features

- **Circuit Breaker**: pause beli otomatis setelah 3 loss berturut (jeda 1 jam)
- **Daily Max Loss**: hentikan beli jika loss harian ≥ DAILY_MAX_LOSS_PCT (default 8%) — reset tiap 00:00 UTC, **persisted across restarts**
- **Break-Even Stop**: `BREAKEVEN_AFTER_TP1=true` — setelah TP1, SL sisa posisi = 0%
- **Peak Hours Filter**: hanya beli 13:00–17:00 UTC dan 20:00–00:00 UTC
- **Momentum Filter**: skip token yang sudah pump >20% dalam 1 jam
- **Token Blacklist**: `/blacklist <addr>` via Telegram
- **Auto-Adjust Score**: evaluasi 20 trade terakhir tiap jam, adjust threshold (max drift +10 di atas base)
- **Scan Health Alert**: alert Telegram jika tidak ada trade selama 6 jam
- **Helius Key Rotation**: auto-rotate saat 429 rate limit (support 5 key)
- **Telegram Commands**: `/status`, `/pause`, `/resume`, `/trades`, `/score`, `/blacklist`, `/helius`
- **Inline Button**: tombol ⏸ Pause dan 📈 Stats di notifikasi buy berfungsi
- **Daily/Weekly Report**: laporan otomatis ke Telegram

## User preferences

- Comment & log language: English
- Scalping dengan modal kecil: MAX_POSITION_SOL=0.03, MAX_POSITIONS=2
- Selalu fee-aware — tidak ada target profit yang mengabaikan biaya transaksi
- `MIN_SCORE_TO_BUY=45` — threshold realistis untuk token Solana nyata (scoring max ~55-65 tanpa exceptional conditions)

## Gotchas

- **Wajib isi secrets**: HELIUS_API_KEY, TELEGRAM_BOT_TOKEN, TELEGRAM_CHAT_ID — bot crash tanpa ketiga ini
- **`config.env` = sumber konfigurasi utama**: edit file ini untuk ubah strategi, lalu restart workflow
- **`TP1_PERCENT` harus > 3.5%** (break-even bersih untuk 0.03 SOL) — di bawah itu = TP tidak pernah profitable
- **`TRAILING_START_PERCENT` harus > `TP2_PERCENT`** — saat ini 16% > 15%, jangan dibalik
- **`BREAKEVEN_AFTER_TP1=true`** (default) — setelah TP1, SL posisi sisa = 0%. Tidak bisa net loss jika TP1 sudah tercapai
- **Circuit breaker hanya pause beli** — sell positions tetap dievaluasi. Ini by design.
- **`daily_limit_paused`** disimpan di `bot_data.json` — restart bot di tengah hari yang bad tidak me-reset proteksi
- **`blacklisted_tokens`** disimpan di `bot_data.json` — survives restart
- **`dynamic_min_score`** reset ke base config saat bot restart, tapi max drift tetap +10 (bug floor-creep sudah diperbaiki)
- **BACKTEST_MIN_SCORE**: backtest scorer max ~90 (tanpa Helius) — gunakan nilai lebih rendah dari live MIN_SCORE_TO_BUY
- **`.env` di-gitignore** — tidak ikut commit, aman menyimpan API key di sana
- **Position sizing**: formula `(score - min_score) / (100 - min_score)` — dengan MIN=MAX=0.03, semua posisi tetap 0.03 SOL. Untuk sizing dinamis, set `MIN_POSITION_SOL` lebih rendah dari `MAX_POSITION_SOL`

## Pointers

- Helius key rotation: tambah `HELIUS_API_KEY_2`, `HELIUS_API_KEY_3` di Replit Secrets / `.env`
- Telegram polling: setiap 3 detik (bukan 30 detik) — respon command cepat
- Callback query inline button: di-handle di `poll_telegram_commands()` di `main.rs`
- Fee math detail: `src/strategy.rs` → `compute_fee_analysis()`
- Position sizing detail: `src/strategy.rs` → `evaluate_buy_signal()` step 7
- Sell logic detail: `src/sell_strategy.rs` → `evaluate_position()`
- Break-even stop: `src/sell_strategy.rs` step 1, `src/paper_trading.rs` → `evaluate_positions()`
- Circuit breaker + daily limit logic: `src/main.rs` → `handle_trade_result()`
- Daily limit persistence: `src/main.rs` → `save()` / `load()` + `BotPersistentData`
- Score auto-adjust: `src/main.rs` → `adjust_dynamic_score()` (uses `base_min_score` as floor)
- Telegram commands: `src/main.rs` → `poll_telegram_commands()`
- Trailing stop detail: `src/positions.rs` → `activate_trailing_stop()` / `update_trailing_stop()`
- Scan health warning: `src/main.rs` → `send_health_warning_if_needed()`
- Backtest exit simulation: `src/backtest.rs` → `simulate_exit()` (mirrors live logic including break-even stop)
