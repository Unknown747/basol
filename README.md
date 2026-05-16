# Basol — Solana Memecoin Trading Bot

Bot trading otomatis untuk token baru di Solana. Scan DexScreener setiap 30 detik, skor token berdasarkan likuiditas/volume/holder, lalu eksekusi beli dan jual melalui Jupiter DEX — atau simulasikan dulu via paper trading.

---

## Daftar Isi

1. [Prasyarat](#1-prasyarat)
2. [Setup di Replit](#2-setup-di-replit) ← **Mulai di sini jika pakai Replit**
3. [Setup di VPS (Ubuntu)](#3-setup-di-vps-ubuntu)
4. [Tutorial Paper Trading](#4-tutorial-paper-trading)
5. [Tutorial Live Trading](#5-tutorial-live-trading)
6. [Tutorial Backtesting](#6-tutorial-backtesting)
7. [Konfigurasi Lengkap (config.env)](#7-konfigurasi-lengkap-configenv)
8. [Sistem 3-Stage Take Profit](#8-sistem-3-stage-take-profit)
9. [Perintah Telegram](#9-perintah-telegram)
10. [Perintah Run](#10-perintah-run)
11. [File State & Persistence](#11-file-state--persistence)
12. [Arsitektur Singkat](#12-arsitektur-singkat)

---

## 1. Prasyarat

| Kebutuhan | Detail |
|-----------|--------|
| **Helius API Key** | Daftar gratis di [helius.dev](https://helius.dev) — 1 juta kredit/bulan |
| **Telegram Bot** | Buat via [@BotFather](https://t.me/BotFather), dapat token dan chat ID |
| **Wallet Solana** | Hanya untuk live trading — **JANGAN gunakan wallet utama** |
| **Rust 1.75+** | Hanya untuk VPS — di Replit sudah tersedia otomatis |

### Cara dapat Telegram Chat ID

1. Kirim pesan apapun ke bot kamu di Telegram
2. Buka: `https://api.telegram.org/bot<TOKEN_BOT>/getUpdates`
3. Cari nilai `"chat":{"id": ANGKA_INI}` — itu Chat ID kamu

---

## 2. Setup di Replit

### Step 1 — Isi API Keys di Replit Secrets

Buka tab **Secrets** di panel Replit dan tambahkan tiga key berikut:

| Key | Nilai |
|-----|-------|
| `HELIUS_API_KEY` | API key dari helius.dev |
| `TELEGRAM_BOT_TOKEN` | Token dari @BotFather |
| `TELEGRAM_CHAT_ID` | Chat ID Telegram kamu |

Untuk live trading, tambahkan juga:
- `WALLET_PRIVATE_KEY` — private key wallet bot (base58 / hex / JSON array)

### Step 2 — Atur konfigurasi di `config.env`

File `config.env` di root project adalah sumber konfigurasi utama bot.
Edit nilai-nilai di sana sesuai strategi kamu — tidak perlu restart Replit, cukup restart workflow.

```bash
# Contoh: ubah ukuran posisi
MAX_POSITION_SOL=0.05
MIN_SCORE_TO_BUY=87.0
```

> **Catatan:** `config.env` sudah ada di project dengan nilai default scalping (0.03 SOL per posisi).
> Jika file tidak ada, salin dari template: `cp config.env.example config.env`

### Step 3 — Jalankan bot

Di panel Replit, klik **Start** pada workflow **"Basol Scanner"**.

Bot akan compile dan mulai scan otomatis. Notifikasi startup dikirim ke Telegram.

### Update dari GitHub

```bash
bash update.sh
```

Script ini otomatis deteksi environment Replit, pull kode terbaru, rebuild, lalu minta restart workflow manual.

---

## 3. Setup di VPS (Ubuntu)

### Instalasi satu perintah

```bash
curl -fsSL https://raw.githubusercontent.com/Unknown747/Baxsol/main/install.sh | bash
```

Script `install.sh` otomatis:
- Install Rust dan dependensi sistem
- Clone repo ke `~/basol`
- Buat `config.env` dari template
- Build binary release
- Daftarkan sebagai systemd service

### Setelah instalasi

1. **Isi secrets** di `~/basol/.env`:

```bash
cp ~/basol/.env.example ~/basol/.env
nano ~/basol/.env
```

```env
HELIUS_API_KEY=api_key_helius_kamu
TELEGRAM_BOT_TOKEN=token_dari_botfather
TELEGRAM_CHAT_ID=chat_id_kamu
# Untuk live trading:
# WALLET_PRIVATE_KEY=private_key_base58
```

2. **Atur strategi** di `~/basol/config.env`:

```bash
nano ~/basol/config.env
```

3. **Mulai bot**:

```bash
sudo systemctl start basol
sudo systemctl status basol
```

### Update VPS

```bash
bash ~/basol/update.sh
```

Pull kode terbaru dari GitHub, rebuild, dan restart service otomatis.

---

## 4. Tutorial Paper Trading

Paper trading = simulasi 100% realistis tanpa uang nyata. Biaya identik dengan mainnet (slippage, price impact, network fee).

### Setup paper trading

Edit `config.env`:

```env
TRADING_ENABLED=false
PAPER_TRADING_ENABLED=true
PAPER_BALANCE_SOL=0.1        # Set sama dengan modal nyata yang direncanakan
PAPER_REPORT_INTERVAL_SECS=3600

MAX_POSITION_SOL=0.03
MIN_POSITION_SOL=0.03
TAKE_PROFIT_PERCENT=35.0
STOP_LOSS_PERCENT=8.0
TRAILING_START_PERCENT=12.0
TRAILING_DISTANCE_PERCENT=3.0
MIN_SCORE_TO_BUY=89.0
MIN_LIQUIDITY_USD=5000.0
DEFAULT_SLIPPAGE=1.5
MAX_POSITIONS=2
MAX_HOLD_MINUTES=40
TIME_EXIT_THRESHOLD_PCT=3.0
TP1_PERCENT=12.0
TP1_SELL_PERCENT=33.0
TP2_PERCENT=20.0
TP2_SELL_PERCENT=50.0
```

Restart workflow. Konfirmasi di console:

```
[PAPER] Paper trading ACTIVE | Virtual balance: 0.10 SOL
```

### Notifikasi paper trading

| Notifikasi | Kapan |
|------------|-------|
| `[PAPER BUY]` | Bot membeli token virtual |
| `[PAPER SELL] TP1 33%` | Partial sell 33% di profit +12% |
| `[PAPER SELL] TP2 50% sisa` | Partial sell 50% sisa di profit +20% |
| `[PAPER SELL] Take Profit Final` | Jual semua sisa di profit +35% |
| `[PAPER SELL] Stop Loss` | SL kena, jual 100% |
| `[PAPER SELL] Trailing Stop` | Trailing stop kena |
| `[PAPER SELL] Time Exit` | Posisi stuck > 40 menit |
| Laporan ringkasan | Setiap `PAPER_REPORT_INTERVAL_SECS` detik |

### Kapan siap ke live trading?

- Minimal 2–4 minggu paper trading
- Win rate konsisten > 45%
- Total profit positif setelah semua biaya simulasi
- Tidak ada perilaku aneh pada notifikasi

---

## 5. Tutorial Live Trading

> **PERINGATAN:** Live trading menggunakan SOL sungguhan. Bot bisa rugi. Pastikan sudah paper trading minimal 2 minggu.

### Persiapan wallet

1. **Buat wallet baru khusus bot** — JANGAN pakai wallet utama
2. **Transfer modal** ke wallet tersebut
3. **Ekspor private key** dalam format: base58 / hex / JSON array `[12,34,...]`

Estimasi modal minimum (scalping):
```
Modal per posisi : 0.03 SOL
Max posisi       : 2
Buffer biaya     : 0.02 SOL
─────────────────────────────
Total minimum    : 0.08 SOL
```

### Aktifkan live trading

**Di Replit:** Tambahkan `WALLET_PRIVATE_KEY` di tab Secrets.

**Di VPS:** Tambahkan di `~/.env`.

Lalu edit `config.env`:

```env
TRADING_ENABLED=true
PAPER_TRADING_ENABLED=true   # Biarkan aktif untuk perbandingan real-time

# RPC Helius lebih cepat untuk live trading:
# SOLANA_RPC_URL=https://mainnet.helius-rpc.com/?api-key=KEY_DI_SINI
```

Restart workflow. Konfirmasi di console:

```
[TRADING] Wallet loaded: <PUBLIC_KEY_WALLET>
[TRADING] Mode: ACTIVE | Max: 0.03 SOL | TP: 35.0% | SL: 8.0%
```

### Notifikasi live trading

| Notifikasi | Kapan |
|------------|-------|
| `[BUY]` | Token dibeli via Jupiter swap |
| `[SELL] TP1` | Partial sell 33% berhasil on-chain |
| `[SELL] TP2` | Partial sell 50% sisa berhasil |
| `[SELL] Take Profit Final` | Jual semua sisa berhasil |
| `[SELL] Stop Loss` | SL kena, 100% dijual |
| `[SELL] Trailing Stop` | Trailing stop kena |
| `[ERROR] GAGAL sell` | Swap gagal — bot retry berikutnya |

### Cara berhenti dengan aman

Posisi aktif disimpan ke `bot_data.json` setiap 10 menit dan saat ada sell.
Stop bot tidak menutup posisi — posisi dilanjutkan saat restart.

> **Untuk menutup posisi sebelum berhenti:** tunggu semua posisi di-close oleh TP/SL/TimeExit.

---

## 6. Tutorial Backtesting

Backtest mensimulasikan strategi terhadap data historis OHLCV dari DexScreener.

### Backtest strategi saat ini

```bash
cargo run -- --backtest
```

Output: laporan di console + file `backtest_<timestamp>.json` + kirim ke Telegram.

### Compare preset sekaligus

```bash
cargo run -- --compare
```

Bot menguji 4 kombinasi (Conservative, Scalping, Aggressive, Balanced) dan menampilkan tabel perbandingan. Hasil disimpan ke `compare_<timestamp>.json`.

### Konfigurasi backtest di `config.env`

```env
BACKTEST_TOKEN_LIMIT=150     # Token yang diuji (lebih banyak = lebih akurat, lebih lama)
BACKTEST_MIN_AGE_HOURS=6     # Umur minimum token dalam jam
BACKTEST_MAX_AGE_HOURS=72    # Umur maksimum token dalam jam
BACKTEST_MIN_LIQUIDITY=5000  # Likuiditas minimum USD
BACKTEST_MIN_VOLUME=10000    # Volume 24 jam minimum
```

---

## 7. Konfigurasi Lengkap (config.env)

Semua setting strategi dikontrol dari file `config.env`. Edit file ini dan restart workflow untuk apply perubahan.

> **API keys sensitif** (HELIUS_API_KEY, TELEGRAM_BOT_TOKEN, TELEGRAM_CHAT_ID, WALLET_PRIVATE_KEY) **tidak** dimasukkan di `config.env` — simpan di Replit Secrets atau di `.env` (VPS).

### Trading Utama

| Variable | Default | Keterangan |
|----------|---------|------------|
| `TRADING_ENABLED` | `false` | `true` = live trading dengan SOL nyata |
| `PAPER_TRADING_ENABLED` | `true` | `true` = simulasi tanpa uang nyata |
| `SOLANA_RPC_URL` | publik mainnet | Gunakan Helius untuk produksi |

### Strategi (berlaku identik untuk paper & live)

| Variable | Default | Keterangan |
|----------|---------|------------|
| `MAX_POSITION_SOL` | `0.03` | SOL per posisi (maksimal) |
| `MIN_POSITION_SOL` | `0.03` | SOL per posisi (minimal) |
| `MIN_SCORE_TO_BUY` | `89.0` | Skor minimum (0–100). Bot auto-adjust tiap jam |
| `MIN_LIQUIDITY_USD` | `5000.0` | Likuiditas pool minimum USD |
| `DEFAULT_SLIPPAGE` | `1.5` | Slippage % saat swap |
| `MAX_POSITIONS` | `2` | Maksimal posisi aktif bersamaan |

### Take Profit & Stop Loss

| Variable | Default | Keterangan |
|----------|---------|------------|
| `TAKE_PROFIT_PERCENT` | `35.0` | TP final — jual semua sisa posisi |
| `STOP_LOSS_PERCENT` | `8.0` | Jual 100% seketika (tidak pernah partial) |
| `TRAILING_START_PERCENT` | `12.0` | Trailing aktif setelah profit X% |
| `TRAILING_DISTANCE_PERCENT` | `3.0` | Jarak trailing dari harga tertinggi |
| `MAX_HOLD_MINUTES` | `40` | Time exit (0 = nonaktif) |
| `TIME_EXIT_THRESHOLD_PCT` | `3.0` | Keluar jika profit < ini setelah MAX_HOLD |

### 3-Stage Take Profit

| Variable | Default | Keterangan |
|----------|---------|------------|
| `TP1_PERCENT` | `12.0` | Trigger TP1 (0 = nonaktif) |
| `TP1_SELL_PERCENT` | `33.0` | % posisi dijual di TP1 |
| `TP2_PERCENT` | `20.0` | Trigger TP2 (0 = nonaktif) |
| `TP2_SELL_PERCENT` | `50.0` | % SISA posisi dijual di TP2 |

### Smart Protection v3.0

| Variable | Default | Keterangan |
|----------|---------|------------|
| `CIRCUIT_BREAKER_LOSSES` | `3` | Pause beli setelah N loss berturut. Sell tetap jalan |
| `CIRCUIT_BREAKER_PAUSE_HOURS` | `2` | Durasi pause (jam). Resume via `/resume` |
| `PEAK_HOURS_ONLY` | `false` | `true` = hanya beli 13:00–17:00 UTC dan 20:00–00:00 UTC |
| `MOMENTUM_MAX_PCT` | `30.0` | Skip token yang sudah pump > X% dalam 1 jam |

### Helius Key Rotation

| Variable | Keterangan |
|----------|------------|
| `HELIUS_API_KEY` | Key utama (wajib, simpan di Secrets) |
| `HELIUS_API_KEY_2` … `_10` | Key tambahan — bot auto-rotate saat rate limit |
| `HELIUS_API_KEYS` | Alternatif: semua key dalam satu string comma-separated |

Tambah key via Replit Secrets. Cek status: `/helius` di Telegram.

### Paper Trading

| Variable | Default | Keterangan |
|----------|---------|------------|
| `PAPER_BALANCE_SOL` | `0.1` | Saldo virtual awal |
| `PAPER_REPORT_INTERVAL_SECS` | `3600` | Interval laporan otomatis ke Telegram |

---

## 8. Sistem 3-Stage Take Profit

Bot mendukung ambil profit secara bertahap untuk mengurangi risiko.

### Cara kerja (contoh 0.03 SOL)

```
Modal: 0.03 SOL

Harga naik +12% → TP1 fire → jual 33% posisi (0.0099 SOL)
  Dapat balik  : ~0.0108 SOL (net +8.2% setelah biaya)
  Sisa aktif   : 67% posisi (0.0201 SOL)
  Trailing stop: mulai aktif

Harga naik +20% → TP2 fire → jual 50% dari SISA (0.0101 SOL)
  Dapat balik  : ~0.0117 SOL (net +16.2% setelah biaya)
  Sisa aktif   : ~33% posisi awal (0.0100 SOL)

Harga naik +35% → TP Final → jual semua sisa (0.0100 SOL)
  Dapat balik  : ~0.0131 SOL (net +31.2% setelah biaya)
  ATAU trailing stop kena → jual sisa di harga trailing

Stop Loss -8%  → JUAL SEMUA 100% seketika (tidak pernah partial)
  Net loss     : ~-11.1% termasuk biaya
```

### Aktifkan / nonaktifkan

```env
# Aktif (3-stage — default):
TP1_PERCENT=12.0
TP1_SELL_PERCENT=33.0
TP2_PERCENT=20.0
TP2_SELL_PERCENT=50.0
TAKE_PROFIT_PERCENT=35.0

# Nonaktif (single TP biasa):
TP1_PERCENT=0.0
TP2_PERCENT=0.0
TAKE_PROFIT_PERCENT=35.0
```

> **Penting:** `TP1_PERCENT` harus > break-even biaya. Break-even untuk 0.03 SOL di pool $5k dengan slippage 1.5% adalah **~3.81%**.

---

## 9. Perintah Telegram

Bot merespons perintah teks dan klik tombol inline dalam chat atau channel yang di-konfigurasi.

| Perintah | Fungsi |
|----------|--------|
| `/status` | Status bot, SOL price, win rate, P&L, posisi aktif, circuit breaker |
| `/pause` | Pause buy scanning (sell monitoring tetap jalan) |
| `/resume` | Resume scanning + reset circuit breaker |
| `/trades` | 10 paper trade terakhir |
| `/score` | Score threshold saat ini vs base config |
| `/blacklist` | Tampilkan jumlah token di-blacklist |
| `/blacklist <addr>` | Tambahkan token ke blacklist permanen |
| `/helius` | Status dan test semua Helius key |

**Tombol inline** di notifikasi buy juga berfungsi: ⏸ Pause Bot, 📈 Stats.

---

## 10. Perintah Run

```bash
# Bot utama — scan, analisis, paper/live trading
cargo run

# Backtest strategi saat ini vs data historis DexScreener
cargo run -- --backtest

# Compare 4 preset konfigurasi (Conservative, Scalping, Aggressive, Balanced)
cargo run -- --compare

# Tampilkan semua ENV variable yang aktif
cargo run -- --help

# Typecheck cepat tanpa build binary
cargo check

# Build binary produksi (lebih cepat saat dijalankan, untuk VPS)
cargo build --release
./target/release/solana_analyzer

# Update dari GitHub (otomatis detect Replit vs VPS)
bash update.sh
```

---

## 11. File State & Persistence

Bot menyimpan state otomatis dan melanjutkan dari titik terakhir saat restart.

| File | Isi | Kapan disimpan |
|------|-----|----------------|
| `bot_data.json` | Posisi live aktif, token yang sudah dilihat, statistik | Setiap 10 menit + saat ada sell |
| `paper_state.json` | Saldo paper, posisi aktif, riwayat trade | Setiap ada paper buy/sell |
| `config.env` | Konfigurasi strategi (bukan secrets) | Diedit manual |
| `backtest_<ts>.json` | Hasil satu sesi backtest | Setelah `--backtest` selesai |
| `compare_<ts>.json` | Hasil perbandingan preset | Setelah `--compare` selesai |

**Reset paper trading dari awal:**
```bash
rm paper_state.json
# Restart workflow — saldo virtual kembali ke PAPER_BALANCE_SOL
```

**Reset semua data bot:**
```bash
rm bot_data.json paper_state.json
# HATI-HATI: posisi live yang tersimpan akan hilang
```

---

## 12. Arsitektur Singkat

```
src/
  main.rs          — SolanaBot: scan loop (30s), Telegram poll (3s), sell check (60s)
  strategy.rs      — TradingConfig, skor token, FeeAnalysis, BuySignal
  sell_strategy.rs — SellTrigger: SL, TP1/TP2/Final, Trailing, TimeExit
  positions.rs     — Position struct (live): tp1_fired, tp2_fired, amount_in_sol
  paper_trading.rs — PaperTradingState: evaluate_positions, execute_sell (partial)
  wallet.rs        — WalletManager: Jupiter V6 swap, Solana RPC
  backtest.rs      — BacktestEngine: DexScreener OHLCV, 4-preset compare

config.env         — Konfigurasi strategi utama (edit ini untuk ubah setting)
config.env.example — Template konfigurasi (jangan edit, salin ke config.env)
.env.example       — Template secrets (HELIUS_API_KEY, TELEGRAM, WALLET)
```

### Alur sell decision (setiap 60 detik)

```
Untuk setiap posisi aktif:

  1. profit <= -SL%                → jual 100% seketika (stop loss)
  2. profit >= TP1% && !tp1_fired → jual TP1_SELL_PERCENT% (partial, tandai tp1_fired)
  3. profit >= TP2% && tp1_fired  → jual TP2_SELL_PERCENT% dari sisa (partial)
  4. trailing_start tercapai       → aktifkan/perbarui trailing stop
     price <= trailing_stop        → jual 100% sisa
  5. profit >= TP_FINAL && tp2_fired → jual 100% sisa
  6. age >= MAX_HOLD && profit < threshold → jual 100% (time exit)
```

### Biaya yang disimulasikan (paper identik dengan live)

```
Network fee  : 0.000025 SOL per transaksi (25.000 lamport)
Slippage     : DEFAULT_SLIPPAGE% dari nilai swap
Price impact : (nilai_swap_usd / likuiditas_pool) × 100%
Break-even   : ~3.81% untuk 0.03 SOL di pool $5k, slippage 1.5%
R:R ratio    : ~3.8 (SL 8% vs TP 35%) — profitable jika win rate > 20.9%
```

---

## Checklist Sebelum Live Trading

- [ ] Sudah paper trading minimal 2 minggu dengan win rate > 45%
- [ ] Wallet bot adalah wallet BARU, bukan wallet utama/cold storage
- [ ] Saldo wallet ≥ (MAX_POSITION_SOL × MAX_POSITIONS) + 0.02 SOL buffer
- [ ] `TRADING_ENABLED=true` di `config.env`
- [ ] `WALLET_PRIVATE_KEY` sudah diset di Replit Secrets (atau `.env` di VPS)
- [ ] Konfirmasi di log startup: `[TRADING] Wallet loaded` dan `Mode: ACTIVE`
- [ ] `SOLANA_RPC_URL` menggunakan Helius (lebih cepat dari public endpoint)
- [ ] Notifikasi Telegram sudah diterima saat paper trading
- [ ] `TP1_PERCENT` > 3.81% jika 3-stage TP diaktifkan
- [ ] `MAX_POSITIONS` tidak melebihi kemampuan modal di wallet
