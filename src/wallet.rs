// ============================================================
// WALLET MANAGER - Jupiter API V6 Swap Integration
// ============================================================

use ed25519_dalek::{SigningKey, Signer};
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
// Jupiter API Structures
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
    /// Force Jupiter to return a legacy (non-versioned) transaction.
    /// Jupiter V6 defaults to versioned transactions (v0), whose wire format
    /// differs from legacy — the sign_and_send_transaction code parses legacy
    /// format, so this flag is required for correct on-chain signing.
    #[serde(rename = "asLegacyTransaction")]
    as_legacy_transaction: bool,
}

#[derive(Debug, Deserialize)]
pub struct SwapResponse {
    #[serde(rename = "swapTransaction")]
    pub swap_transaction: String,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct TokenAccountInfo {
    pub amount: String,
    pub decimals: u8,
}

pub struct WalletManager {
    pub client: Client,
    pub public_key: String,
    private_key_bytes: Vec<u8>,
}

impl WalletManager {
    /// Load wallet from WALLET_PRIVATE_KEY environment variable
    pub fn from_env() -> Result<Self, String> {
        let pk_str = std::env::var("WALLET_PRIVATE_KEY")
            .map_err(|_| "WALLET_PRIVATE_KEY not found in environment".to_string())?;

        let pk_trimmed = pk_str.trim();

        // Supports: JSON array [1,2,...], hex string, or base58
        let private_key_bytes = if pk_trimmed.starts_with('[') {
            // JSON array format
            serde_json::from_str::<Vec<u8>>(pk_trimmed)
                .map_err(|e| format!("Failed to parse private key JSON array: {e}"))?
        } else if pk_trimmed.len() == 128 {
            // 64-byte hex format
            hex::decode(pk_trimmed)
                .map_err(|e| format!("Failed to decode hex private key: {e}"))?
        } else {
            // base58 format
            bs58_decode(pk_trimmed)?
        };

        if private_key_bytes.len() < 32 {
            return Err(format!(
                "Private key too short: {} bytes (need at least 32)",
                private_key_bytes.len()
            ));
        }

        let public_key = derive_public_key(&private_key_bytes)?;

        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| format!("Failed to build HTTP client: {e}"))?;

        println!("[WALLET] Wallet loaded successfully: {public_key}");
        Ok(Self { client, public_key, private_key_bytes })
    }

    /// Get wallet SOL balance (in SOL)
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
            .map_err(|e| format!("RPC request failed: {e}"))?;

        let data: serde_json::Value = resp.json().await
            .map_err(|e| format!("Failed to parse response: {e}"))?;

        let lamports = data["result"]["value"]
            .as_u64()
            .ok_or("Failed to get balance from response")?;

        Ok(lamports as f64 / LAMPORTS_PER_SOL)
    }

    /// Get token balance held by wallet
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
            .map_err(|e| format!("RPC request failed: {e}"))?;

        let data: serde_json::Value = resp.json().await
            .map_err(|e| format!("Failed to parse response: {e}"))?;

        let accounts = data["result"]["value"].as_array();
        if let Some(accs) = accounts {
            if let Some(first) = accs.first() {
                let amount_str = first["account"]["data"]["parsed"]["info"]["tokenAmount"]["uiAmountString"]
                    .as_str()
                    .unwrap_or("0");
                return amount_str.parse::<f64>()
                    .map_err(|e| format!("Failed to parse amount: {e}"));
            }
        }
        Ok(0.0)
    }

    /// BUY token using Jupiter V6
    pub async fn buy_token(
        &self,
        token_address: &str,
        amount_in_sol: f64,
        slippage: f64,
    ) -> Result<String, String> {
        let amount_lamports = (amount_in_sol * LAMPORTS_PER_SOL) as u64;
        let slippage_bps = (slippage * 100.0) as u32;

        println!(
            "[BUY] Attempting to buy {token_address} - {amount_in_sol:.4} SOL ({amount_lamports} lamports) slippage {slippage:.1}%"
        );

        let mut last_error = String::new();
        for attempt in 1..=MAX_RETRY {
            match self.execute_buy(token_address, amount_lamports, slippage_bps).await {
                Ok(sig) => {
                    println!("[BUY] SUCCESS on attempt {attempt} - signature: {sig}");
                    return Ok(sig);
                }
                Err(e) => {
                    last_error = e.clone();
                    println!("[BUY] Failed attempt {attempt}/{MAX_RETRY}: {e}");
                    if attempt < MAX_RETRY {
                        sleep(Duration::from_millis(RETRY_DELAY_MS * attempt as u64)).await;
                    }
                }
            }
        }
        Err(format!("Buy failed after {MAX_RETRY} attempts. Last error: {last_error}"))
    }

    async fn execute_buy(
        &self,
        token_address: &str,
        amount_lamports: u64,
        slippage_bps: u32,
    ) -> Result<String, String> {
        let quote = self.get_quote(
            SOL_MINT,
            token_address,
            amount_lamports,
            slippage_bps,
        ).await?;

        let price_impact: f64 = quote.price_impact_pct.parse().unwrap_or(0.0);
        if price_impact > 5.0 {
            return Err(format!("Price impact too high: {price_impact:.2}%"));
        }

        println!(
            "[BUY] Quote OK - out: {} tokens, price impact: {:.2}%",
            quote.out_amount, price_impact
        );

        let swap_tx = self.build_swap_transaction(&quote).await?;
        let signature = self.sign_and_send_transaction(&swap_tx).await?;
        Ok(signature)
    }

    /// SELL token using Jupiter V6
    pub async fn sell_token(
        &self,
        token_address: &str,
        percentage_to_sell: f64,
        slippage: f64,
    ) -> Result<String, String> {
        let token_balance = self.get_token_balance(token_address).await?;

        if token_balance <= 0.0 {
            return Err(format!("No token balance for {token_address}"));
        }

        let decimals = match self.get_token_decimals(token_address).await {
            Ok(d) => d,
            Err(e) => {
                // Most pump.fun / Raydium meme tokens use 6 decimals. However some SPL
                // tokens use 9 — if decimals are fetched wrong, sell_raw is 1000× too small
                // and the sell silently under-fills. Log a clear warning so it's visible.
                println!("[SELL] ⚠️  Could not fetch decimals for {token_address}: {e} — defaulting to 6");
                6
            }
        };
        let sell_amount = token_balance * (percentage_to_sell / 100.0);
        let sell_raw = (sell_amount * 10f64.powi(decimals as i32)) as u64;
        let slippage_bps = (slippage * 100.0) as u32;

        println!(
            "[SELL] Attempting to sell {token_address} - {percentage_to_sell:.2}% ({sell_amount:.6} tokens) slippage {slippage:.1}%"
        );

        if sell_raw == 0 {
            return Err("Sell amount too small".to_string());
        }

        let mut last_error = String::new();
        for attempt in 1..=MAX_RETRY {
            match self.execute_sell(token_address, sell_raw, slippage_bps).await {
                Ok(sig) => {
                    println!("[SELL] SUCCESS on attempt {attempt} - signature: {sig}");
                    return Ok(sig);
                }
                Err(e) => {
                    last_error = e.clone();
                    println!("[SELL] Failed attempt {attempt}/{MAX_RETRY}: {e}");
                    if attempt < MAX_RETRY {
                        sleep(Duration::from_millis(RETRY_DELAY_MS * attempt as u64)).await;
                    }
                }
            }
        }
        Err(format!("Sell failed after {MAX_RETRY} attempts. Last error: {last_error}"))
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
        // Guard against catastrophic sells into thin pools — mirrors buy-side check.
        // Without this, a partial TP sell on a low-liquidity pool could lose >10% to impact.
        if price_impact > 10.0 {
            return Err(format!(
                "Sell price impact too high: {price_impact:.2}% — skipping to protect capital"
            ));
        }
        println!(
            "[SELL] Quote OK - receiving: {} lamports SOL, price impact: {:.2}%",
            quote.out_amount, price_impact
        );

        let swap_tx = self.build_swap_transaction(&quote).await?;
        let signature = self.sign_and_send_transaction(&swap_tx).await?;
        Ok(signature)
    }

    async fn get_quote(
        &self,
        input_mint: &str,
        output_mint: &str,
        amount: u64,
        slippage_bps: u32,
    ) -> Result<JupiterQuote, String> {
        let url = format!(
            "{JUPITER_QUOTE_URL}?inputMint={input_mint}&outputMint={output_mint}&amount={amount}&slippageBps={slippage_bps}"
        );

        let resp = self.client
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("Quote request failed: {e}"))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("Jupiter quote error {status}: {body}"));
        }

        resp.json::<JupiterQuote>()
            .await
            .map_err(|e| format!("Failed to parse quote: {e}"))
    }

    async fn build_swap_transaction(&self, quote: &JupiterQuote) -> Result<String, String> {
        let request = SwapRequest {
            quote_response: quote.clone(),
            user_public_key: self.public_key.clone(),
            wrap_and_unwrap_sol: true,
            auto_slippage: false,
            dynamic_compute_unit_limit: true,
            prioritization_fee_lamports: "auto".to_string(),
            as_legacy_transaction: true,
        };

        let resp = self.client
            .post(JUPITER_SWAP_URL)
            .json(&request)
            .send()
            .await
            .map_err(|e| format!("Swap request failed: {e}"))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("Jupiter swap error {status}: {body}"));
        }

        let swap_resp: SwapResponse = resp.json().await
            .map_err(|e| format!("Failed to parse swap response: {e}"))?;

        Ok(swap_resp.swap_transaction)
    }

    /// Sign transaction with private key and send to RPC
    async fn sign_and_send_transaction(&self, base64_tx: &str) -> Result<String, String> {
        use base64::{Engine as _, engine::general_purpose};

        let tx_bytes = general_purpose::STANDARD
            .decode(base64_tx)
            .map_err(|e| format!("Failed to decode base64 transaction: {e}"))?;

        let signature_bytes = sign_transaction(&tx_bytes, &self.private_key_bytes)?;
        let signed_tx = inject_signature(tx_bytes, signature_bytes)?;
        let signed_b64 = general_purpose::STANDARD.encode(&signed_tx);

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
            .map_err(|e| format!("Failed to send transaction: {e}"))?;

        let data: serde_json::Value = resp.json().await
            .map_err(|e| format!("Failed to parse send response: {e}"))?;

        if let Some(err) = data.get("error") {
            return Err(format!("RPC error: {err}"));
        }

        let signature = data["result"]
            .as_str()
            .ok_or("No signature in response")?
            .to_string();

        Ok(signature)
    }

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
            .map_err(|e| format!("RPC request failed: {e}"))?;

        let data: serde_json::Value = resp.json().await
            .map_err(|e| format!("Failed to parse response: {e}"))?;

        Ok(data["result"]["value"]["decimals"].as_u64().unwrap_or(6) as u8)
    }
}

// ============================================================
// CRYPTO HELPERS
// ============================================================

/// Decode base58 string to Vec<u8>
fn bs58_decode(input: &str) -> Result<Vec<u8>, String> {
    const ALPHABET: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    let mut result: Vec<u8> = Vec::new();
    let mut leading_zeros = 0usize;

    for ch in input.chars() {
        if ch == '1' && result.is_empty() {
            leading_zeros += 1;
            continue;
        }
        let digit = ALPHABET
            .iter()
            .position(|&b| b == ch as u8)
            .ok_or_else(|| format!("Invalid character in base58: '{ch}'"))? as u64;

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

/// Encode bytes to base58 string
fn bs58_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    let mut result: Vec<u8> = Vec::new();
    let mut leading_zeros = 0usize;

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
        output.push(ALPHABET[digit as usize] as char);
    }
    output
}

/// Derive Solana public key from 32-byte seed using Ed25519
fn derive_public_key(private_key_bytes: &[u8]) -> Result<String, String> {
    if private_key_bytes.len() < 32 {
        return Err("Private key too short: need at least 32 bytes".to_string());
    }

    let seed: [u8; 32] = private_key_bytes[..32]
        .try_into()
        .map_err(|_| "Failed to convert seed to 32-byte array".to_string())?;

    let signing_key = SigningKey::from_bytes(&seed);
    let verifying_key = signing_key.verifying_key();

    Ok(bs58_encode(verifying_key.as_bytes()))
}

/// Decode compact-u16 from start of bytes, returns (value, bytes_consumed)
fn decode_compact_u16(bytes: &[u8]) -> Result<(usize, usize), String> {
    if bytes.is_empty() {
        return Err("Empty bytes for compact-u16 decode".to_string());
    }

    let b0 = bytes[0] as usize;
    if b0 <= 0x7f {
        return Ok((b0, 1));
    }

    if bytes.len() < 2 {
        return Err("Compact-u16 truncated (need 2 bytes)".to_string());
    }
    let b1 = bytes[1] as usize;
    if b1 <= 0x7f {
        return Ok(((b0 & 0x7f) | (b1 << 7), 2));
    }

    if bytes.len() < 3 {
        return Err("Compact-u16 truncated (need 3 bytes)".to_string());
    }
    let b2 = bytes[2] as usize;
    Ok(((b0 & 0x7f) | ((b1 & 0x7f) << 7) | (b2 << 14), 3))
}

/// Sign transaction with Ed25519 using private key.
/// Solana transaction format: [compact-u16 num_sigs][sig slots][message]
/// Only the message bytes are signed (after the signature slots).
fn sign_transaction(tx_bytes: &[u8], private_key_bytes: &[u8]) -> Result<[u8; 64], String> {
    if private_key_bytes.len() < 32 {
        return Err("Private key too short for signing".to_string());
    }

    // Parse signature count from compact-u16 at the start
    let (num_sigs, prefix_len) = decode_compact_u16(tx_bytes)?;

    // Message starts after: compact-u16 prefix + (num_sigs * 64 byte signature slots)
    let message_start = prefix_len + num_sigs * 64;

    if tx_bytes.len() <= message_start {
        return Err(format!(
            "Transaction too short: {} bytes, message starts at offset {}",
            tx_bytes.len(), message_start
        ));
    }

    let message_bytes = &tx_bytes[message_start..];

    let seed: [u8; 32] = private_key_bytes[..32]
        .try_into()
        .map_err(|_| "Failed to convert seed to 32-byte array".to_string())?;

    let signing_key = SigningKey::from_bytes(&seed);
    let signature = signing_key.sign(message_bytes);

    Ok(signature.to_bytes())
}

/// Inject signature into first slot of a Solana transaction.
/// Format: [compact-u16 num_sigs][64-byte sig slot 0][...rest...]
fn inject_signature(mut tx_bytes: Vec<u8>, signature: [u8; 64]) -> Result<Vec<u8>, String> {
    if tx_bytes.is_empty() {
        return Err("Transaction bytes are empty".to_string());
    }

    let (_, prefix_len) = decode_compact_u16(&tx_bytes)?;

    // First signature starts immediately after the compact-u16 prefix
    let sig_start = prefix_len;
    let sig_end = sig_start + 64;

    if tx_bytes.len() < sig_end {
        return Err(format!(
            "Transaction too short to inject signature: {} bytes (need at least {})",
            tx_bytes.len(), sig_end
        ));
    }

    tx_bytes[sig_start..sig_end].copy_from_slice(&signature);
    Ok(tx_bytes)
}
