# Basol — Solana Memecoin Trading Bot

Bot trading otomatis untuk token baru di Solana. Scan DexScreener setiap 30 detik, skor token berdasarkan likuiditas/volume/holder, lalu eksekusi beli dan jual melalui Jupiter DEX — atau simulasikan dulu via paper trading.

---

## Daftar Isi

1. [Prasyarat](#1-prasyarat)
2. [Instalasi & Setup Awal](#2-instalasi--setup-awal)
3. [Tutorial Paper Trading](#3-tutorial-paper-trading) ← **Mulai di sini**
4. [Tutorial Mainnet (Live Trading)](#4-tutorial-mainnet-live-trading)
5. [Tutorial Backtesting](#5-tutorial-backtesting)
6. [Sistem 3-Stage Take Profit](#6-sistem-3-stage-take-profit)
7. [Referensi Lengkap ENV](#7-referensi-lengkap-env)
8. [Perintah Run](#8-perintah-run)
9. [File State & Persistence](#9-file-state--persistence)
10. [Arsitektur Singkat](#10-arsitektur-singkat)

---

## 1. Prasyarat

| Kebutuhan | Detail |
|-----------|--------|
| **Rust** | Minimal 1.75+ — install via [rustup.rs](https://rustup.rs) |
| **Cargo** | Sudah termasuk bersama Rust |
| **Helius API Key** | Daftar gratis di [helius.dev](https://helius.dev) — 1 juta kredit/bulan gratis |
| **Telegram Bot** | Buat via [@BotFather](https://t.me/BotFather), dapat token dan chat ID |
| **Wallet Solana** | Hanya untuk live trading — **JANGAN gunakan wallet utama** |

### Cara dapat Telegram Chat ID
1. Kirim pesan apa saja ke bot kamu di Telegram
2. Buka browser: `https://api.telegram.org/bot<TOKEN_BOT>/getUpdates`
3. Cari nilai `"chat":{"id": ANGKA_INI}` — itu Chat ID kamu

---

## 2. Instalasi & Setup Awal

### Clone & build

```bash
git clone <repo-url>
cd basol
cargo build
```

Build pertama membutuhkan 2–5 menit (download dependensi Rust).

### Buat file `.env`

```bash
cp .env.example .env
```

Buka `.env` dan isi minimal 3 field wajib:

```env
HELIUS_API_KEY=<api_key_dari_helius.dev>
TELEGRAM_BOT_TOKEN=<token_dari_botfather>
TELEGRAM_CHAT_ID=<chat_id_kamu>
```

> **Tanpa ketiga field ini, bot akan crash saat startup dengan pesan error yang jelas.**

---

## 3. Tutorial Paper Trading

Paper trading = simulasi 100% realistis tanpa uang nyata. Menggunakan biaya yang identik dengan mainnet (slippage, price impact, network fee 0.000025 SOL/tx) agar hasil simulasi akurat.

### Step 1 — Setup `.env` untuk paper trading

Salin blok ini ke `.env` kamu (ganti nilai yang di-wrap `<...>`):

```env
# === WAJIB ===
HELIUS_API_KEY=<api_key_helius>
TELEGRAM_BOT_TOKEN=<token_botfather>
TELEGRAM_CHAT_ID=<chat_id>

# === AKTIFKAN PAPER TRADING ===
PAPER_TRADING_ENABLED=true
PAPER_BALANCE_SOL=0.1         # Modal virtual — set sama dengan modal nyata yang direncanakan
PAPER_REPORT_INTERVAL_SECS=3600  # Laporan otomatis ke Telegram setiap 1 jam

# === PASTIKAN LIVE TRADING NONAKTIF ===
TRADING_ENABLED=false

# === STRATEGI (scalping 0.05 SOL — direkomendasikan untuk mulai) ===
MAX_POSITION_SOL=0.05
MIN_POSITION_SOL=0.05
TAKE_PROFIT_PERCENT=35.0
STOP_LOSS_PERCENT=8.0
TRAILING_START_PERCENT=12.0
TRAILING_DISTANCE_PERCENT=3.0
MIN_SCORE_TO_BUY=87.0
MIN_LIQUIDITY_USD=5000.0
DEFAULT_SLIPPAGE=1.5
MAX_POSITIONS=2
MAX_HOLD_MINUTES=40
TIME_EXIT_THRESHOLD_PCT=3.0

# === 3-STAGE TP (direkomendasikan) ===
TP1_PERCENT=12.0
TP1_SELL_PERCENT=33.0
TP2_PERCENT=20.0
TP2_SELL_PERCENT=50.0

# === HARGA SOL AWAL (bot update otomatis tiap 5 menit) ===
SOL_PRICE_USD=170.0
```

### Step 2 — Jalankan bot

**Cara A — Replit workflow** (direkomendasikan, bot jalan 24/7):
- Di panel Replit, klik **Start** pada workflow **"Basol Scanner"**

**Cara B — Terminal**:
```bash
cargo run
```

### Step 3 — Verifikasi paper trading aktif

Saat startup, console akan menampilkan:

```
══════════════════════════════════════════
   Bot Analisis Solana v2.0 + Auto Trade
══════════════════════════════════════════
  TRADING_ENABLED       = false
  PAPER_TRADING_ENABLED = true
  MAX_POSITION_SOL      = 0.05
  ...

[TRADING] Mode: NON-AKTIF | Max: 0.05 SOL | TP: 35.0% | SL: 8.0%
[PAPER] Paper trading AKTIF | Virtual balance: 0.10 SOL
```

Notifikasi Telegram yang akan masuk:

| Notifikasi | Kapan |
|------------|-------|
| `[PAPER BUY]` | Bot membeli token virtual |
| `[PAPER SELL] TP1 33%` | Partial sell 33% di profit +12% |
| `[PAPER SELL] TP2 50%sisa` | Partial sell 50% sisa di profit +20% |
| `[PAPER SELL] Take Profit Final` | Jual semua sisa di profit +35% |
| `[PAPER SELL] Stop Loss` | SL kena, jual 100% |
| `[PAPER SELL] Trailing Stop` | Trailing stop kena |
| `[PAPER SELL] Time Exit` | Posisi stuck > 40 menit |
| Laporan ringkasan | Setiap `PAPER_REPORT_INTERVAL_SECS` detik |

### Step 4 — Monitor & evaluasi

State paper trading otomatis disimpan ke `paper_state.json`. Restart bot tidak mereset saldo — dilanjutkan dari titik terakhir.

**Kapan siap pindah ke live trading?**
- Minimal 2–4 minggu paper trading
- Win rate konsisten > 45%
- Total profit positif setelah semua biaya simulasi
- Tidak ada perilaku aneh pada notifikasi

---

## 4. Tutorial Mainnet (Live Trading)

> **PERINGATAN:** Live trading menggunakan SOL sungguhan. Bot bisa rugi. Pastikan sudah paper trading minimal 2 minggu dan memahami risikonya sepenuhnya.

### Persiapan wallet

1. **Buat wallet baru khusus bot** — jangan pakai wallet utama/cold storage
2. **Transfer modal** ke wallet tersebut

Estimasi modal minimum:
```
Modal per posisi : 0.05 SOL
Max posisi       : 2
Buffer biaya     : 0.02 SOL (network fee + slippage reserve)
─────────────────────────────
Total minimum    : 0.12 SOL
```

3. **Ekspor private key** wallet bot dalam salah satu format berikut:
   - Base58 string (paling umum, output dari Phantom/Solflare "Export Private Key")
   - Hex string
   - JSON array: `[12,34,56,...]` (output dari Solana CLI `solana-keygen`)

### Step 1 — Setup `.env` untuk live trading

```env
# === WAJIB ===
HELIUS_API_KEY=<api_key_helius>
TELEGRAM_BOT_TOKEN=<token_botfather>
TELEGRAM_CHAT_ID=<chat_id>

# === WALLET ===
WALLET_PRIVATE_KEY=<private_key_base58_wallet_bot>

# === RPC — Helius jauh lebih cepat dari public endpoint ===
SOLANA_RPC_URL=https://mainnet.helius-rpc.com/?api-key=<API_KEY_HELIUS>

# === AKTIFKAN LIVE TRADING ===
TRADING_ENABLED=true

# === PAPER TRADING (bisa aktif bersamaan untuk perbandingan, atau nonaktifkan) ===
PAPER_TRADING_ENABLED=false

# === STRATEGI ===
MAX_POSITION_SOL=0.05
MIN_POSITION_SOL=0.05
TAKE_PROFIT_PERCENT=35.0
STOP_LOSS_PERCENT=8.0
TRAILING_START_PERCENT=12.0
TRAILING_DISTANCE_PERCENT=3.0
MIN_SCORE_TO_BUY=87.0
MIN_LIQUIDITY_USD=5000.0
DEFAULT_SLIPPAGE=1.5
MAX_POSITIONS=2
MAX_HOLD_MINUTES=40
TIME_EXIT_THRESHOLD_PCT=3.0

# === 3-STAGE TP ===
TP1_PERCENT=12.0
TP1_SELL_PERCENT=33.0
TP2_PERCENT=20.0
TP2_SELL_PERCENT=50.0

SOL_PRICE_USD=170.0
```

### Step 2 — Verifikasi sebelum start

```bash
cargo check              # Pastikan tidak ada error kompilasi
cargo run -- --help      # Lihat ringkasan konfigurasi yang aktif
```

### Step 3 — Jalankan bot

**Cara A — Replit workflow** (direkomendasikan untuk server 24/7):
- Klik **Start** pada workflow **"Basol Scanner"**
- Bot restart otomatis jika crash

**Cara B — Terminal**:
```bash
cargo run
```

### Step 4 — Verifikasi live trading aktif

Output startup yang benar saat live trading aktif:

```
══════════════════════════════════════════
   Bot Analisis Solana v2.0 + Auto Trade
══════════════════════════════════════════
  TRADING_ENABLED       = true
  WALLET_PRIVATE_KEY    = ✅ SET
  ...

[TRADING] Wallet berhasil diload: <PUBLIC_KEY_WALLET_BOT>
[TRADING] Mode: AKTIF | Max: 0.05 SOL | TP: 35.0% | SL: 8.0%
```

Jika private key gagal dibaca:
```
[TRADING] ⚠️ Gagal load wallet: ... - Trading dinonaktifkan
```
Penyebab umum: format private key salah. Coba format base58 atau JSON array `[...]`.

### Step 5 — Notifikasi Telegram live trading

| Notifikasi | Kapan |
|------------|-------|
| `[BUY]` | Token berhasil dibeli via Jupiter swap |
| `[SELL] TP1` | Partial sell 33% berhasil dieksekusi on-chain |
| `[SELL] TP2` | Partial sell 50% sisa berhasil |
| `[SELL] Take Profit Final` | Jual semua sisa berhasil |
| `[SELL] Stop Loss` | SL kena, 100% dijual |
| `[SELL] Trailing Stop` | Trailing stop kena |
| `[SELL] Time Exit` | Posisi stuck > 40 menit |
| `[ERROR] GAGAL sell` | Swap gagal — bot akan retry berikutnya |

### Cara berhenti dengan aman

Posisi aktif yang terbuka disimpan ke `bot_data.json` setiap 10 menit dan saat ada event sell.
Menghentikan bot (Ctrl+C atau Stop workflow) tidak akan menutup posisi — posisi akan dilanjutkan saat restart.

```bash
# Hentikan via Ctrl+C di terminal
# Atau klik Stop pada workflow di panel Replit
# Posisi aktif aman, tersimpan di bot_data.json
```

> **Untuk menutup posisi sebelum berhenti:** tunggu sampai semua posisi di-close oleh TP/SL/TimeExit, baru stop bot.

---

## 5. Tutorial Backtesting

Backtest mensimulasikan strategi terhadap data historis OHLCV dari DexScreener. Tidak perlu API key Helius atau wallet — hanya `TELEGRAM_BOT_TOKEN` dan `TELEGRAM_CHAT_ID` (opsional untuk kirim hasil).

### Backtest strategi saat ini

```bash
cargo run -- --backtest
```

Output: laporan di console + file `backtest_<timestamp>.json` + laporan ke Telegram (jika dikonfigurasi).

Konfigurasi di `.env`:
```env
BACKTEST_TOKEN_LIMIT=150     # Jumlah token yang diuji (lebih banyak = lebih akurat, lebih lama)
BACKTEST_MIN_AGE_HOURS=6     # Umur minimum token dalam jam
BACKTEST_MAX_AGE_HOURS=72    # Umur maksimum token dalam jam
BACKTEST_MIN_LIQUIDITY=5000  # Likuiditas minimum USD
BACKTEST_MIN_VOLUME=10000    # Volume 24 jam minimum
```

### Compare 8 preset sekaligus

```bash
cargo run -- --compare
```

Bot menguji 8 kombinasi TP/SL/Trailing secara berurutan dan menampilkan tabel perbandingan win rate, profit, drawdown. Hasil disimpan ke `compare_<timestamp>.json`.

---

## 6. Sistem 3-Stage Take Profit

Bot mendukung ambil profit secara bertahap untuk mengurangi risiko.

### Cara kerja (contoh 0.05 SOL, slippage 1.5%, pool $5k)

```
Modal: 0.05 SOL

Harga naik +12% → TP1 fire → jual 33% posisi (0.0167 SOL)
  Dapat balik  : ~0.0180 SOL (net +8.2% setelah biaya)
  Sisa aktif   : 67% posisi (0.0334 SOL)
  Trailing stop: mulai aktif di 12%, stop di 9%

Harga naik +20% → TP2 fire → jual 50% dari SISA (0.0167 SOL)
  Dapat balik  : ~0.0194 SOL (net +16.2% setelah biaya)
  Sisa aktif   : ~33% posisi awal (0.0167 SOL)

Harga naik +35% → TP Final → jual semua sisa (0.0167 SOL)
  Dapat balik  : ~0.0219 SOL (net +31.2% setelah biaya)
  ATAU trailing stop kena → jual sisa di harga trailing

Stop Loss -8%  → JUAL SEMUA 100% seketika (tidak pernah partial)
  Net loss     : ~-11.3% termasuk biaya
```

### Keuntungan utama

Setelah TP1 fire, profit sudah diamankan. Bahkan jika harga berbalik 100%, total kerugian sudah terkompensasi sebagian. **Setelah TP2 fire, hampir tidak mungkin rugi total dari modal awal posisi tersebut.**

### Aktifkan/nonaktifkan di `.env`

```env
# Aktif (3-stage):
TP1_PERCENT=12.0
TP1_SELL_PERCENT=33.0
TP2_PERCENT=20.0
TP2_SELL_PERCENT=50.0
TAKE_PROFIT_PERCENT=35.0   # TP Final

# Nonaktif (single TP biasa):
TP1_PERCENT=0.0
TP2_PERCENT=0.0
TAKE_PROFIT_PERCENT=20.0
```

> **Perhatian:** `TP1_PERCENT` harus lebih besar dari break-even biaya. Break-even untuk 0.05 SOL di pool $5k dengan slippage 1.5% adalah **~3.81%**. Setting TP1 di bawah ini artinya sell di kerugian bersih.

---

## 7. Referensi Lengkap ENV

### Wajib (bot crash jika tidak ada)

| Variable | Contoh | Keterangan |
|----------|--------|------------|
| `HELIUS_API_KEY` | `abc123...` | Dari [helius.dev](https://helius.dev) |
| `TELEGRAM_BOT_TOKEN` | `123456:ABC...` | Dari @BotFather |
| `TELEGRAM_CHAT_ID` | `-100123456789` | ID chat atau channel |

### Wallet (wajib jika `TRADING_ENABLED=true`)

| Variable | Format | Keterangan |
|----------|--------|------------|
| `WALLET_PRIVATE_KEY` | Base58 / Hex / `[1,2,3,...]` | Private key wallet bot (bukan wallet utama!) |
| `SOLANA_RPC_URL` | URL | Default: publik mainnet-beta. Pakai Helius untuk produksi |

### Master switch

| Variable | Default | Keterangan |
|----------|---------|------------|
| `TRADING_ENABLED` | `false` | `true` = live trading dengan SOL nyata |
| `PAPER_TRADING_ENABLED` | `false` | `true` = simulasi tanpa uang nyata |

Keduanya bisa aktif bersamaan — bot eksekusi live DAN paper secara paralel untuk perbandingan real-time.

### Strategi trading

| Variable | Default | Keterangan |
|----------|---------|------------|
| `MAX_POSITION_SOL` | `0.5` | SOL per posisi |
| `MIN_POSITION_SOL` | auto | 10% dari MAX, minimal 0.01 |
| `TAKE_PROFIT_PERCENT` | `40.0` | TP final (jual semua sisa posisi) |
| `STOP_LOSS_PERCENT` | `15.0` | Jual 100% seketika jika rugi melebihi ini |
| `TRAILING_START_PERCENT` | `20.0` | Trailing mulai aktif setelah profit X% |
| `TRAILING_DISTANCE_PERCENT` | `5.0` | Jarak trailing dari harga tertinggi |
| `MIN_SCORE_TO_BUY` | `85.0` | Skor minimum token (skala 0–100) |
| `MIN_LIQUIDITY_USD` | `10000.0` | Likuiditas pool minimum |
| `DEFAULT_SLIPPAGE` | `1.0` | Slippage % saat swap |
| `MAX_POSITIONS` | `5` | Maksimal posisi aktif bersamaan |
| `MAX_HOLD_MINUTES` | `0` | Time exit otomatis (0 = nonaktif) |
| `TIME_EXIT_THRESHOLD_PCT` | `5.0` | Keluar jika profit < ini setelah MAX_HOLD_MINUTES |

### 3-Stage Take Profit

| Variable | Default | Keterangan |
|----------|---------|------------|
| `TP1_PERCENT` | `0.0` | Trigger TP1 dalam % profit (0 = nonaktif) |
| `TP1_SELL_PERCENT` | `33.0` | % posisi yang dijual di TP1 |
| `TP2_PERCENT` | `0.0` | Trigger TP2 dalam % profit (0 = nonaktif) |
| `TP2_SELL_PERCENT` | `50.0` | % SISA posisi yang dijual di TP2 |

### Paper trading

| Variable | Default | Keterangan |
|----------|---------|------------|
| `PAPER_TRADING_ENABLED` | `false` | Aktifkan simulasi |
| `PAPER_BALANCE_SOL` | `10.0` | Saldo virtual awal |
| `PAPER_REPORT_INTERVAL_SECS` | `3600` | Interval laporan otomatis ke Telegram |

### Backtesting

| Variable | Default | Keterangan |
|----------|---------|------------|
| `BACKTEST_TOKEN_LIMIT` | `150` | Jumlah token yang diuji |
| `BACKTEST_MIN_AGE_HOURS` | `6` | Umur minimum token (jam) |
| `BACKTEST_MAX_AGE_HOURS` | `72` | Umur maksimum token (jam) |
| `BACKTEST_MIN_LIQUIDITY` | `5000.0` | Likuiditas minimum USD |
| `BACKTEST_MIN_VOLUME` | `10000.0` | Volume 24 jam minimum |
| `SOL_PRICE_USD` | `170.0` | Harga SOL awal (update otomatis tiap 5 menit) |

---

## 8. Perintah Run

```bash
# Bot utama — scan, analisis, paper/live trading
cargo run

# Backtest strategi saat ini vs data historis DexScreener
cargo run -- --backtest

# Compare 8 preset konfigurasi sekaligus
cargo run -- --compare

# Tampilkan bantuan dan ringkasan ENV aktif
cargo run -- --help

# Typecheck cepat tanpa build binary (untuk verifikasi perubahan kode)
cargo check

# Build binary produksi (lebih cepat saat dijalankan)
cargo build --release
./target/release/solana_analyzer
```

---

## 9. File State & Persistence

Bot menyimpan state otomatis dan melanjutkan dari titik terakhir saat restart.

| File | Isi | Kapan disimpan |
|------|-----|----------------|
| `bot_data.json` | Posisi live aktif, token yang sudah dilihat, statistik harian | Setiap 10 menit |
| `paper_state.json` | Saldo paper, posisi paper aktif, riwayat trade tertutup | Setiap ada paper buy/sell |
| `backtest_<ts>.json` | Hasil lengkap satu sesi backtest | Setelah `--backtest` selesai |
| `compare_<ts>.json` | Hasil perbandingan 8 preset | Setelah `--compare` selesai |

**Reset paper trading dari awal:**
```bash
rm paper_state.json
# Restart bot — saldo virtual kembali ke PAPER_BALANCE_SOL
```

**Reset semua data bot (termasuk posisi live!):**
```bash
rm bot_data.json paper_state.json
# HATI-HATI: posisi live yang tersimpan akan hilang
```

---

## 10. Arsitektur Singkat

```
src/
  main.rs          — SolanaBot: scan loop (30s), sell check (60s), paper report
  strategy.rs      — TradingConfig (from_env), skor token, FeeAnalysis, BuySignal
  sell_strategy.rs — SellTrigger enum: SL, TP1/TP2/Final, Trailing, TimeExit
  positions.rs     — Position struct (live): tp1_fired, tp2_fired, amount_in_sol
  paper_trading.rs — PaperTradingState: evaluate_positions (8 param), execute_sell (partial)
  wallet.rs        — WalletManager: Jupiter V6 swap, Solana RPC
  backtest.rs      — BacktestEngine: DexScreener OHLCV, 8-preset compare
```

### Alur sell decision (setiap 60 detik)

```
Untuk setiap posisi aktif:

  1. profit <= -SL%               → jual 100% seketika (stop loss)
  2. profit >= TP1% && !tp1_fired → jual TP1_SELL_PERCENT% (partial, tandai tp1_fired)
  3. profit >= TP2% && tp1_fired  → jual TP2_SELL_PERCENT% dari sisa (partial, tandai tp2_fired)
  4. profit >= trailing_start     → aktifkan/perbarui trailing stop
     price <= trailing_stop       → jual 100% sisa
  5. profit >= TP_FINAL && tp2    → jual 100% sisa
  6. age >= MAX_HOLD && profit < threshold → jual 100% (time exit)
```

### Biaya yang disimulasikan (paper identik dengan live)

```
Network fee  : 0.000025 SOL per transaksi (25.000 lamport)
Slippage     : DEFAULT_SLIPPAGE% dari nilai swap
Price impact : (nilai_swap_usd / likuiditas_pool) × 50%, max 30%
Break-even   : ~3.81% untuk 0.05 SOL di pool $5k, slippage 1.5%
R:R ratio    : 1.43 (SL 8% vs TP 12%+) — profitable jika win rate > 41.1%
```

---

## Checklist Sebelum Live Trading

- [ ] Sudah paper trading minimal 2 minggu dengan win rate > 45%
- [ ] Wallet bot adalah wallet BARU, bukan wallet utama
- [ ] Saldo wallet = (MAX_POSITION_SOL × MAX_POSITIONS) + 0.02 SOL buffer
- [ ] `TRADING_ENABLED=true` di `.env`
- [ ] `WALLET_PRIVATE_KEY` sudah diset dan berhasil diload (lihat log startup)
- [ ] `SOLANA_RPC_URL` menggunakan Helius (bukan public endpoint yang lambat)
- [ ] Notifikasi Telegram sudah diterima saat paper trading
- [ ] `TP1_PERCENT` > 3.81% jika 3-stage TP diaktifkan
- [ ] `MAX_POSITIONS` tidak melebihi kemampuan modal di wallet
