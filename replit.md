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

## Audit History — Bug Fixes (Paper/Live Parity)

### Session 2 — Full Codebase Audit (9 bugs fixed, `cargo check` clean)

| # | File | Bug | Impact |
|---|---|---|---|
| 1 | `paper_trading.rs` | `NETWORK_FEE_SOL` duplikat di paper_trading + strategy.rs | Divergence silently jika salah satu diubah |
| 2 | `paper_trading.rs` | `calc_price_impact_pct` formula 2× lebih besar dari live | Paper entry price lebih buruk dari kenyataan |
| 3 | `paper_trading.rs` | `profit_sol` disimpan pre-fee; live post-fee | `handle_trade_result`, daily limit, circuit breaker, stats sedikit meleset |
| 4 | `main.rs` | `check_and_paper_sell` pakai `paper_config.TP/SL/trailing` bukan `trading_config` | Paper sell tidak merespons perubahan runtime ke trading_config |
| 5 | `strategy.rs` | Default `stop_loss_percent` = 6.0, tapi config.env = 5.0 | Bot pakai SL 6% jika config.env hilang |
| 6 | `strategy.rs` | Default `min_score_to_buy` = 85.0, tapi config.env = 45.0 | Bot hampir tidak pernah beli jika config.env hilang |
| 7 | `main.rs` | Default `circuit_breaker_pause_hours` = 2, tapi config.env = 1 | Pause 2 jam bukan 1 jam jika config.env hilang |
| 8 | `main.rs` | Daily loss % menggunakan paper balance saja sebagai denominator | Untuk live-only mode, perlu PAPER_BALANCE_SOL = modal nyata |
| 9 | `main.rs` | `adjust_dynamic_score` hanya baca `paper_state.closed_trades` | Score tidak auto-adjust di live-only mode (paper disabled) |

Juga dihapus: 4 field dead (`take_profit/stop_loss/trailing_*`) dari `PaperConfig` struct — semua TP/SL/trailing kini baca langsung dari `trading_config` satu sumber.

### Session 5 — Paper Trading Report Analysis (1 bug fixed)

| # | File | Bug | Impact |
|---|---|---|---|
| 1 | `paper_trading.rs` + `sell_strategy.rs` | `None => continue` saat price unavailable skip SEMUA check termasuk time exit | Posisi held selamanya jika token hilang dari DexScreener — terbukti: UNKNOWN held 373 menit instead of 25 menit |

**Root cause:** DexScreener berhenti menampilkan token lama setelah beberapa jam. Saat price tidak ada di price map, loop langsung `continue` tanpa cek `age_minutes >= max_hold_minutes`. Fixed: force time exit at entry price (0% P&L) jika price unavailable tapi umur posisi sudah melebihi batas.

**False positive diverifikasi:**
- Worst trade -10.2% di report vs -1.56% di JSON → data dari sesi bot sebelumnya (memory tidak sync dengan JSON setelah restart)
- Balance 0.0950 SOL di report vs 0.0695 SOL di JSON → open position 0.03 SOL yang dibuka setelah report dikirim

### Session 4 — Full Codebase Audit (2 bugs fixed, false positives diverifikasi)

| # | File | Bug | Impact |
|---|---|---|---|
| 1 | `sell_strategy.rs` | `format_sell_notification` tampilkan P&L kotor (tanpa fee) — paper sell sudah net fee | Live sell notification tampilkan profit ~0.000025 SOL lebih tinggi dari kenyataan |
| 2 | `strategy.rs` | `compute_fee_analysis` exit cost: jika `liquidity_usd=0` formula runtuh ke 100% impact | Jika dipanggil dengan liquidity nol, `breakeven_pct` jadi tak wajar — guard DEFAULT_PRICE_IMPACT_PCT ditambahkan |

**False positive diverifikasi (bukan bug):**
- `update_trailing_stop` pakai current_price vs `activate_trailing_stop` pakai highest_price → desain yang benar (ratchet up vs anchor)
- Score = min_score → min_position_sol → intentional via clamp
- Trailing stop "100% full close" notification setelah partial TP → akurat (100% dari sisa posisi)
- Daily loss denominator pakai paper balance → sudah diketahui/didokumentasikan (bug #8 Session 2)
- `sold_sol = amount_sol × percentage / 100` → benar (capital deployed, bukan proceeds; profit dihitung terpisah)

**Bug config sebelumnya di sesi ini:**
- `seen_tokens` retention 30 hari → 8 jam (root cause 0 trade di VPS)
- `MOMENTUM_MAX_PCT` 20% → 40%
- `OFF_PEAK_MOMENTUM_MAX_PCT` 15% → 30%

### Session 6 — Full Codebase Audit + 3 Performance Improvements (1 bug fixed, 3 improvements)

| # | File | Perubahan | Impact |
|---|---|---|---|
| 1 | `main.rs` | Bug: startup banner tampilkan default SL `"6.0"` padahal config.env = `5.0` | Display mismatch di log startup — diperbaiki ke `"5.0"` |
| 2 | `main.rs` | Helius RPC auto-config: jika `SOLANA_RPC_URL` masih public mainnet, otomatis set ke `mainnet.helius-rpc.com` dari `HELIUS_API_KEY` | Live trading & balance check pakai Helius RPC (fast) bukan public node (slow/unreliable) |
| 3 | `config.env` | `PAPER_REPORT_INTERVAL_SECS`: 3600 → **1800** (30 menit) | Laporan 2× lebih sering — pantau performa paper trading lebih cepat |
| 4 | `config.env` | `MIN_POSITION_SOL`: 0.03 → **0.015** | Dynamic position sizing aktif: score 40-70 → 0.015 SOL (konservatif), score 70-100 → 0.015-0.03 SOL (scaling up) |

**False positives (bukan bug):**
- `off_peak_*` defaults di code tidak cocok config.env → tidak masalah karena OFF_PEAK_TRADING_ENABLED=false, plus config.env selalu override
- `momentum_max_pct` code default 30.0 vs config.env 55.0 → tidak masalah karena config.env override; default hanya fallback jika file hilang

**Dynamic sizing math (MIN=0.015, MAX=0.03, threshold=40):**
- Score 40-70 → raw_size < 0.015 → clamp ke 0.015 SOL (konservatif untuk borderline tokens)
- Score 70 → 0.5 × 0.03 = 0.015 SOL (tepat di min)
- Score 80 → 0.667 × 0.03 = 0.020 SOL
- Score 90 → 0.833 × 0.03 = 0.025 SOL
- Score 100 → 1.0 × 0.03 = 0.030 SOL (full size untuk exceptional tokens)

### Session 1 — Paper/Live Parity (4 bug sebelumnya)
- Score multiplier hardcoded 75 → min_score_to_buy
- Backtest missing break-even stop
- daily_loss tidak persisted across restarts
- Health warning silenced permanently
- Paper buy tidak di-gate oleh live_buy_blocked
- Live sell price impact formula salah (sold_sol×sol_price)
- format_alert hardcoded 75

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
