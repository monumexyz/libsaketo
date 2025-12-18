use cuprate_epee_encoding::epee_object;
use cuprate_epee_encoding::macros::bytes::Bytes;
use cuprate_types::BlockCompleteEntry;
use monero_oxide::block::Block;
use monero_simple_request_rpc::SimpleRequestRpc;
use monero_wallet::rpc::{Rpc, RpcError, ScannableBlock};
use reqwest::Client;
use serde_json::Value;

#[derive(Clone)]
pub struct FastscanRpc {
    /// URL of the Fastscan server (with port if available)
    url: String,
    /// Internal RPC
    oxide_rpc: SimpleRequestRpc,
}

struct HeightsRequest {
    heights: Vec<u64>
}
epee_object!(HeightsRequest, heights: Vec<u64>,);

struct BlocksResponse {
    blocks: Vec<BlockCompleteEntry>
}
epee_object!(BlocksResponse, blocks: Vec<BlockCompleteEntry>,);

impl FastscanRpc {
    pub async fn new(url: String, port: u16) -> Self {
        let mut url = url.trim_end_matches("/").to_string();
        url = if url.starts_with("https://") || url.starts_with("http://") {
            format!("{}:{}", url, port)
        } else {
            format!("http://{}:{}", url, port)
        };
        let oxide_rpc = SimpleRequestRpc::new(url.clone()).await;
        let oxide_rpc = match oxide_rpc {
            Err(e) => panic!("Failed to create SimpleRequestRpc: {}", e),
            Ok(rpc) => rpc,
        };
        Self { url, oxide_rpc }
    }

    pub fn get_height(&self) -> impl Send + Future<Output = Result<u64, RpcError>> {
        let url = format!("{}/get_height", self.url);
        async move {
            match Client::new().get(&url).send().await {
                Err(e) => Err(RpcError::ConnectionError(format!("Request error: {}", e))),
                Ok(resp) => {
                    if !resp.status().is_success() {
                        return Err(RpcError::ConnectionError(format!("HTTP error: {}", resp.status())));
                    }
                    let json : Value = resp.json().await.unwrap();
                    match json["height"].as_u64() {
                        Some(height) => Ok(height),
                        None => Err(RpcError::InternalError("Failed to parse height from response".to_string())),
                    }
                }
            }
        }
    }

    pub fn get_scannable_blocks_by_number(&self, numbers: Vec<u64>) -> impl Send + Future<Output = Result<Vec<ScannableBlock>, RpcError>> {
        async move {
            let bytes_req = match cuprate_epee_encoding::to_bytes(HeightsRequest { heights: numbers }) {
                Err(e) => return Err(RpcError::InternalError(format!("Failed to encode request: {}", e))),
                Ok(b) => b,
            };
            let bytes_resp : Bytes = match Client::new().get(format!("{}/get_blocks_by_height.bin", self.url)).header("Content-Type", "application/octet-stream").body(bytes_req.freeze().to_vec()).send().await {
                Err(e) => return Err(RpcError::ConnectionError(format!("Request error: {}", e))),
                Ok(resp) => {
                    if !resp.status().is_success() {
                        return Err(RpcError::ConnectionError(format!("HTTP error: {}", resp.status())));
                    }
                    match resp.bytes().await {
                        Err(e) => return Err(RpcError::InternalError(format!("Failed to read response bytes: {}", e))),
                        Ok(b) => b,
                    }
                }
            };
            let resp: BlocksResponse = match cuprate_epee_encoding::from_bytes(&mut bytes_resp.clone()) {
                Err(e) => return Err(RpcError::InternalError(format!("Failed to decode response: {}", e))),
                Ok(b) => b,
            };
            let mut blocks: Vec<ScannableBlock> = Vec::new();
            for block in &resp.blocks {
                let mut cursor = std::io::Cursor::new(&block.block);
                match Block::read(&mut cursor) {
                    Err(e) => return Err(RpcError::InternalError(format!("Failed to read block: {}", e))),
                    Ok(b) => {
                        match self.oxide_rpc.get_scannable_block(b).await {
                            Err(e) => return Err(RpcError::InternalError(format!("Failed to get scannable block: {}", e))),
                            Ok(sb) => blocks.push(sb),
                        }
                    }
                }
            }
            Ok(blocks)
        }
    }
}

