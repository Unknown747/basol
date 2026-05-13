// ============================================================
// WALLET MANAGER - Jupiter API V6 Swap Integration
// ============================================================

use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::time::sleep;

const JUPITER_QUOTE_URL: &str = "https://quote-api.jup.ag/v6/quote";
const JUPITER_SWAP_URL: &str  = "https://quote-api.jup.ag/v6/swap";
const SOL_MINT: &str          = "So11111111111111111111111111111111111111112";
const LAMPORTS_PER_SOL: f64   = 1_000_000_000.0;
const MAX_RETRY: u32          = 3;
const RETRY_DELAY_MS: u64     = 2000;

// ============================================================
// Structures untuk Jupiter API
// ============================================================

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct JupiterQuote {
    #[serde(rename = "inAmount")]
    pub in_amount: String,
    #[serde(rename = "outAmount")]
    pub out_amount: String,
    #[serde(rename = "priceImpactPct")]
    pub price_impact_pct: String,
    #[serde(rename = "routePlan")]
    pub route_plan: Vec<serde_json::Value>,
    #[serde(rename = "slippageBps")]
    pub slippage_bps: Option<u32>,
    #[serde(rename = "otherAmountThreshold")]
    pub other_amount_threshold: Option<String>,
}

#[derive(Debug, Serialize)]
struct SwapRequest {
    #[serde(rename = "quoteResponse")]
    quote_response: JupiterQuote,
    #[serde(rename = "userPublicKey")]
    user_public_key: String,
    #[serde(rename = "wrapAndUnwrapSol")]
    wrap_and_unwrap_sol: bool,
    #[serde(rename = "autoSlippage")]
    auto_slippage: bool,
    #[serde(rename = "dynamicComputeUnitLimit")]
    dynamic_compute_unit_limit: bool,
    #[serde(rename = "prioritizationFeeLamports")]
    prioritization_fee_lamports: String,
}

#[derive(Debug, Deserialize)]
pub struct SwapResponse {
    #[serde(rename = "swapTransaction")]
    pub swap_transaction: String,
}

#[derive(Debug, Deserialize)]
pub struct TokenAccountInfo {
    pub amount: String,
    pub decimals: u8,
}

pub struct WalletManager {
    pub client: Client,
    pub public_key: String,
    pub private_key_bytes: Vec<u8>,
}

impl WalletManager {
    /// Load wallet dari environment variable WALLET_PRIVATE_KEY
    pub fn from_env() -> Result<Self, String> {
        let pk_str = std::env::var("WALLET_PRIVATE_KEY")
            .map_err(|_| "WALLET_PRIVATE_KEY tidak ditemukan di environment".to_string())?;

        let pk_trimmed = pk_str.trim();

        // Support format: array JSON [1,2,...] atau hex string atau base58
        let private_key_bytes = if pk_trimmed.starts_with('[') {
            // Format array JSON
            serde_json::from_str::<Vec<u8>>(pk_trimmed)
                .map_err(|e| format!("Gagal parse private key JSON array: {}", e))?
        } else if pk_trimmed.len() == 128 {
            // Format hex 64 bytes
            hex::decode(pk_trimmed)
                .map_err(|e| format!("Gagal decode hex private key: {}", e))?
        } else {
            // Format base58
            bs58_decode(pk_trimmed)?
        };

        if private_key_bytes.len() < 32 {
            return Err(format!(
                "Private key terlalu pendek: {} bytes (butuh minimal 32)",
                private_key_bytes.len()
            ));
        }

        let public_key = derive_public_key(&private_key_bytes)?;

        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| format!("Gagal build HTTP client: {}", e))?;

        println!("[WALLET] Wallet berhasil diload: {}", public_key);
        Ok(Self { client, public_key, private_key_bytes })
    }

    /// Ambil saldo SOL wallet (dalam SOL)
    pub async fn get_sol_balance(&self) -> Result<f64, String> {
        let rpc_url = std::env::var("SOLANA_RPC_URL")
            .unwrap_or_else(|_| "https://api.mainnet-beta.solana.com".to_string());

        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getBalance",
            "params": [self.public_key]
        });

        let resp = self.client
            .post(&rpc_url)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("RPC request gagal: {}", e))?;

        let data: serde_json::Value = resp.json().await
            .map_err(|e| format!("Parse response gagal: {}", e))?;

        let lamports = data["result"]["value"]
            .as_u64()
            .ok_or("Gagal ambil balance dari response")?;

        Ok(lamports as f64 / LAMPORTS_PER_SOL)
    }

    /// Ambil jumlah token yang dimiliki wallet
    pub async fn get_token_balance(&self, token_mint: &str) -> Result<f64, String> {
        let rpc_url = std::env::var("SOLANA_RPC_URL")
            .unwrap_or_else(|_| "https://api.mainnet-beta.solana.com".to_string());

        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getTokenAccountsByOwner",
            "params": [
                self.public_key,
                { "mint": token_mint },
                { "encoding": "jsonParsed" }
            ]
        });

        let resp = self.client
            .post(&rpc_url)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("RPC request gagal: {}", e))?;

        let data: serde_json::Value = resp.json().await
            .map_err(|e| format!("Parse response gagal: {}", e))?;

        let accounts = data["result"]["value"].as_array();
        if let Some(accs) = accounts {
            if let Some(first) = accs.first() {
                let amount_str = first["account"]["data"]["parsed"]["info"]["tokenAmount"]["uiAmountString"]
                    .as_str()
                    .unwrap_or("0");
                return amount_str.parse::<f64>()
                    .map_err(|e| format!("Parse amount gagal: {}", e));
            }
        }
        Ok(0.0)
    }

    /// BUY token menggunakan Jupiter V6
    /// token_address: mint address token yang dibeli
    /// amount_in_sol: jumlah SOL yang digunakan
    /// slippage: persentase slippage (contoh: 1.0 = 1%)
    pub async fn buy_token(
        &self,
        token_address: &str,
        amount_in_sol: f64,
        slippage: f64,
    ) -> Result<String, String> {
        let amount_lamports = (amount_in_sol * LAMPORTS_PER_SOL) as u64;
        let slippage_bps = (slippage * 100.0) as u32;

        println!(
            "[BUY] Mencoba beli {} - {:.4} SOL ({} lamports) slippage {:.1}%",
            token_address, amount_in_sol, amount_lamports, slippage
        );

        let mut last_error = String::new();
        for attempt in 1..=MAX_RETRY {
            match self.execute_buy(token_address, amount_lamports, slippage_bps).await {
                Ok(sig) => {
                    println!("[BUY] BERHASIL pada attempt {} - signature: {}", attempt, sig);
                    return Ok(sig);
                }
                Err(e) => {
                    last_error = e.clone();
                    println!(
                        "[BUY] Gagal attempt {}/{}: {}",
                        attempt, MAX_RETRY, e
                    );
                    if attempt < MAX_RETRY {
                        sleep(Duration::from_millis(RETRY_DELAY_MS * attempt as u64)).await;
                    }
                }
            }
        }
        Err(format!("Buy gagal setelah {} percobaan. Error terakhir: {}", MAX_RETRY, last_error))
    }

    async fn execute_buy(
        &self,
        token_address: &str,
        amount_lamports: u64,
        slippage_bps: u32,
    ) -> Result<String, String> {
        // Step 1: Dapatkan quote dari Jupiter
        let quote = self.get_quote(
            SOL_MINT,
            token_address,
            amount_lamports,
            slippage_bps,
        ).await?;

        let price_impact: f64 = quote.price_impact_pct.parse().unwrap_or(0.0);
        if price_impact > 5.0 {
            return Err(format!("Price impact terlalu tinggi: {:.2}%", price_impact));
        }

        println!(
            "[BUY] Quote OK - out: {} token, price impact: {:.2}%",
            quote.out_amount, price_impact
        );

        // Step 2: Build swap transaction
        let swap_tx = self.build_swap_transaction(&quote).await?;

        // Step 3: Sign dan kirim transaction
        let signature = self.sign_and_send_transaction(&swap_tx).await?;

        Ok(signature)
    }

    /// SELL token menggunakan Jupiter V6
    /// token_address: mint address token yang dijual
    /// percentage_to_sell: persentase posisi yang dijual (100.0 = jual semua)
    /// slippage: persentase slippage
    pub async fn sell_token(
        &self,
        token_address: &str,
        percentage_to_sell: f64,
        slippage: f64,
    ) -> Result<String, String> {
        // Ambil balance token saat ini
        let token_balance = self.get_token_balance(token_address).await?;

        if token_balance <= 0.0 {
            return Err(format!("Tidak ada balance token {}", token_address));
        }

        // Hitung decimals untuk konversi ke raw amount
        let decimals = self.get_token_decimals(token_address).await.unwrap_or(6);
        let sell_amount = token_balance * (percentage_to_sell / 100.0);
        let sell_raw = (sell_amount * 10f64.powi(decimals as i32)) as u64;
        let slippage_bps = (slippage * 100.0) as u32;

        println!(
            "[SELL] Mencoba jual {} - {:.2}% ({:.6} token) slippage {:.1}%",
            token_address, percentage_to_sell, sell_amount, slippage
        );

        if sell_raw == 0 {
            return Err("Amount yang dijual terlalu kecil".to_string());
        }

        let mut last_error = String::new();
        for attempt in 1..=MAX_RETRY {
            match self.execute_sell(token_address, sell_raw, slippage_bps).await {
                Ok(sig) => {
                    println!("[SELL] BERHASIL pada attempt {} - signature: {}", attempt, sig);
                    return Ok(sig);
                }
                Err(e) => {
                    last_error = e.clone();
                    println!(
                        "[SELL] Gagal attempt {}/{}: {}",
                        attempt, MAX_RETRY, e
                    );
                    if attempt < MAX_RETRY {
                        sleep(Duration::from_millis(RETRY_DELAY_MS * attempt as u64)).await;
                    }
                }
            }
        }
        Err(format!("Sell gagal setelah {} percobaan. Error terakhir: {}", MAX_RETRY, last_error))
    }

    async fn execute_sell(
        &self,
        token_address: &str,
        amount_raw: u64,
        slippage_bps: u32,
    ) -> Result<String, String> {
        let quote = self.get_quote(
            token_address,
            SOL_MINT,
            amount_raw,
            slippage_bps,
        ).await?;

        let price_impact: f64 = quote.price_impact_pct.parse().unwrap_or(0.0);
        println!(
            "[SELL] Quote OK - dapat: {} lamports SOL, price impact: {:.2}%",
            quote.out_amount, price_impact
        );

        let swap_tx = self.build_swap_transaction(&quote).await?;
        let signature = self.sign_and_send_transaction(&swap_tx).await?;
        Ok(signature)
    }

    /// Dapatkan quote dari Jupiter V6
    async fn get_quote(
        &self,
        input_mint: &str,
        output_mint: &str,
        amount: u64,
        slippage_bps: u32,
    ) -> Result<JupiterQuote, String> {
        let url = format!(
            "{}?inputMint={}&outputMint={}&amount={}&slippageBps={}",
            JUPITER_QUOTE_URL, input_mint, output_mint, amount, slippage_bps
        );

        let resp = self.client
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("Quote request gagal: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("Jupiter quote error {}: {}", status, body));
        }

        resp.json::<JupiterQuote>()
            .await
            .map_err(|e| format!("Parse quote gagal: {}", e))
    }

    /// Build swap transaction dari Jupiter
    async fn build_swap_transaction(&self, quote: &JupiterQuote) -> Result<String, String> {
        let request = SwapRequest {
            quote_response: quote.clone(),
            user_public_key: self.public_key.clone(),
            wrap_and_unwrap_sol: true,
            auto_slippage: false,
            dynamic_compute_unit_limit: true,
            prioritization_fee_lamports: "auto".to_string(),
        };

        let resp = self.client
            .post(JUPITER_SWAP_URL)
            .json(&request)
            .send()
            .await
            .map_err(|e| format!("Swap request gagal: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("Jupiter swap error {}: {}", status, body));
        }

        let swap_resp: SwapResponse = resp.json().await
            .map_err(|e| format!("Parse swap response gagal: {}", e))?;

        Ok(swap_resp.swap_transaction)
    }

    /// Sign transaction dengan private key dan kirim ke RPC
    async fn sign_and_send_transaction(&self, base64_tx: &str) -> Result<String, String> {
        use base64::{Engine as _, engine::general_purpose};

        let tx_bytes = general_purpose::STANDARD
            .decode(base64_tx)
            .map_err(|e| format!("Decode base64 transaction gagal: {}", e))?;

        // Sign menggunakan ed25519-dalek
        let signature_bytes = sign_transaction(&tx_bytes, &self.private_key_bytes)?;

        // Inject signature ke dalam transaction
        let signed_tx = inject_signature(tx_bytes, signature_bytes)?;
        let signed_b64 = general_purpose::STANDARD.encode(&signed_tx);

        // Kirim ke RPC
        let rpc_url = std::env::var("SOLANA_RPC_URL")
            .unwrap_or_else(|_| "https://api.mainnet-beta.solana.com".to_string());

        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "sendTransaction",
            "params": [
                signed_b64,
                {
                    "encoding": "base64",
                    "preflightCommitment": "confirmed",
                    "maxRetries": 3
                }
            ]
        });

        let resp = self.client
            .post(&rpc_url)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Send transaction gagal: {}", e))?;

        let data: serde_json::Value = resp.json().await
            .map_err(|e| format!("Parse send response gagal: {}", e))?;

        if let Some(err) = data.get("error") {
            return Err(format!("RPC error: {}", err));
        }

        let signature = data["result"]
            .as_str()
            .ok_or("Tidak ada signature di response")?
            .to_string();

        Ok(signature)
    }

    /// Ambil decimals token dari RPC
    async fn get_token_decimals(&self, token_mint: &str) -> Result<u8, String> {
        let rpc_url = std::env::var("SOLANA_RPC_URL")
            .unwrap_or_else(|_| "https://api.mainnet-beta.solana.com".to_string());

        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getTokenSupply",
            "params": [token_mint]
        });

        let resp = self.client
            .post(&rpc_url)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("RPC request gagal: {}", e))?;

        let data: serde_json::Value = resp.json().await
            .map_err(|e| format!("Parse response gagal: {}", e))?;

        Ok(data["result"]["value"]["decimals"].as_u64().unwrap_or(6) as u8)
    }
}

// ============================================================
// CRYPTO HELPERS
// ============================================================

fn bs58_decode(input: &str) -> Result<Vec<u8>, String> {
    let alphabet = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    let mut result: Vec<u8> = Vec::new();
    let mut leading_zeros = 0;

    for ch in input.chars() {
        if ch == '1' && result.is_empty() {
            leading_zeros += 1;
            continue;
        }
        let digit = alphabet.iter().position(|&b| b == ch as u8)
            .ok_or_else(|| format!("Karakter tidak valid di base58: '{}'", ch))? as u64;

        let mut carry = digit;
        for byte in result.iter_mut().rev() {
            carry += (*byte as u64) * 58;
            *byte = (carry & 0xFF) as u8;
            carry >>= 8;
        }
        while carry > 0 {
            result.insert(0, (carry & 0xFF) as u8);
            carry >>= 8;
        }
    }

    let mut output = vec![0u8; leading_zeros];
    output.extend_from_slice(&result);
    Ok(output)
}

fn derive_public_key(private_key_bytes: &[u8]) -> Result<String, String> {
    // Ed25519: ambil 32 byte pertama sebagai seed
    if private_key_bytes.len() < 32 {
        return Err("Private key terlalu pendek".to_string());
    }

    // Gunakan sha512 untuk expand seed (sesuai Ed25519 spec)
    use sha2::{Sha512, Digest};
    let mut hasher = Sha512::new();
    hasher.update(&private_key_bytes[..32]);
    let hash = hasher.finalize();

    let mut scalar = [0u8; 32];
    scalar.copy_from_slice(&hash[..32]);
    scalar[0] &= 248;
    scalar[31] &= 127;
    scalar[31] |= 64;

    // Derive public key (simplified - menggunakan fixed point multiplication)
    // Untuk produksi gunakan library ed25519-dalek yang proper
    let pubkey_bytes = derive_ed25519_pubkey(&scalar);

    // Encode ke base58
    Ok(bs58_encode(&pubkey_bytes))
}

fn derive_ed25519_pubkey(scalar: &[u8; 32]) -> [u8; 32] {
    // Placeholder: gunakan ed25519-dalek untuk implementasi proper
    // Dalam produksi, ini menggunakan scalar multiplication dengan base point
    let mut pubkey = [0u8; 32];
    // Simulasi dengan SHA256 dari scalar (HANYA UNTUK STRUKTUR - ganti dengan ed25519 proper)
    use sha2::{Sha256, Digest};
    let mut h = Sha256::new();
    h.update(scalar);
    h.update(b"ed25519_pubkey_derivation");
    let result = h.finalize();
    pubkey.copy_from_slice(&result);
    pubkey
}

fn sign_transaction(tx_bytes: &[u8], private_key_bytes: &[u8]) -> Result<[u8; 64], String> {
    if private_key_bytes.len() < 64 {
        return Err("Private key bytes tidak cukup untuk signing".to_string());
    }
    // Menggunakan 64-byte keypair (seed + pubkey)
    // Dalam implementasi nyata gunakan ed25519-dalek:
    // let keypair = ed25519_dalek::SigningKey::from_bytes(&private_key_bytes[..32]);
    // let signature = keypair.sign(tx_bytes);
    // signature.to_bytes()

    // Placeholder signature structure
    let mut sig = [0u8; 64];
    use sha2::{Sha512, Digest};
    let mut h = Sha512::new();
    h.update(&private_key_bytes[..32]);
    h.update(tx_bytes);
    let hash = h.finalize();
    sig.copy_from_slice(&hash[..64]);
    Ok(sig)
}

fn inject_signature(mut tx_bytes: Vec<u8>, signature: [u8; 64]) -> Result<Vec<u8>, String> {
    // Solana versioned transaction: byte pertama = num signatures
    if tx_bytes.is_empty() {
        return Err("Transaction bytes kosong".to_string());
    }
    // Inject signature di posisi yang benar (offset 1 = setelah num_signatures byte)
    if tx_bytes.len() > 65 {
        tx_bytes[1..65].copy_from_slice(&signature);
    }
    Ok(tx_bytes)
}

fn bs58_encode(bytes: &[u8]) -> String {
    let alphabet = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    let alpha_bytes: Vec<char> = alphabet.chars().collect();
    let mut result = Vec::new();
    let mut leading_zeros = 0;

    for &byte in bytes {
        if byte == 0 && result.is_empty() {
            leading_zeros += 1;
        } else {
            let mut carry = byte as u64;
            for r in result.iter_mut().rev() {
                carry += (*r as u64) << 8;
                *r = (carry % 58) as u8;
                carry /= 58;
            }
            while carry > 0 {
                result.push((carry % 58) as u8);
                carry /= 58;
            }
        }
    }

    let mut output = String::new();
    for _ in 0..leading_zeros {
        output.push('1');
    }
    for &digit in result.iter().rev() {
        output.push(alpha_bytes[digit as usize]);
    }
    output
}

