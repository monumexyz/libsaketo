use crate::rpc::FastscanRpc;
use curve25519_dalek::constants::ED25519_BASEPOINT_POINT;
use curve25519_dalek::Scalar;
use monero_oxide::transaction::Input;
use monero_wallet::address::SubaddressIndex;
use monero_wallet::generators::biased_hash_to_point;
use monero_wallet::primitives::keccak256;
use monero_wallet::{Scanner, ViewPair};
use monero_wallet::rpc::ScannableBlock;
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot};
use zeroize::Zeroizing;
use crate::service::QueueMessage::NewBlock;

#[derive(Clone, Serialize, Deserialize)]
pub struct ScannedInput {
    pub height: u64,
    pub key_image: [u8; 32],
    pub amount: u64,
    pub hash: [u8; 32],
    pub timestamp: u64,
    pub previous_output: [u8; 32],
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ScannedOutput {
    pub height: u64,
    pub key_image: [u8; 32],
    pub amount: u64,
    pub hash: [u8; 32],
    pub timestamp: u64,
    pub index: Option<u64>,
}

/// Commands that can be sent to the service
pub enum ServiceCommand {
    Start,
    Stop,
    Status,
    Transactions,
    Data
}

#[derive(Serialize)]
pub enum ServiceResponse {
    Started,
    Stopped,
    Status(ServiceStatus, u64),
    Transactions(Vec<ScannedInput>, Vec<ScannedOutput>),
    Data {
        status: ServiceStatus,
        height: u64,
        inputs: Vec<ScannedInput>,
        outputs: Vec<ScannedOutput>,
    },
    Error(String),
}

#[derive(Clone, Serialize)]
pub enum ServiceStatus {
    Scanning,
    NotScanning,
    Scanned,
    Error(String),
}

/// Type of notification sent by the service, no data is transferred and the main thread should check for the data itself
/// When using FFI (saketo_ffi), the integer codes correspond to the following:
/// 0 - FoundTransaction
pub enum ServiceNotification {
    /// When a transaction is found
    FoundTransaction,
}

pub struct FastscanService {
    /// Wallet's private spend key
    priv_spend: [u8; 32],
    /// Last scanned height of this wallet
    service_height: u64,
    /// The length of current the longest known chain
    chain_height: u64,
    /// Inputs
    inputs: Vec<ScannedInput>,
    /// Outputs
    outputs: Vec<ScannedOutput>,
    /// The RPC we use to connect to the node
    rpc: FastscanRpc,
    /// The Scanner struct we use to scan the blockchain
    scanner: Scanner,
    /// Status of the service (running, stopped, error, etc.)
    status: ServiceStatus,
    /// Optionally, a channel to notify about new scanned data
    notification_channel: Option<mpsc::Sender<ServiceNotification>>,
}

/// Internal message queue for blocks and errors between the network producer and the scanner consumer
#[derive(Clone)]
enum QueueMessage {
    NewBlock(ScannableBlock),
    Error(String),
}

impl FastscanService {
    pub async fn new(priv_spend: [u8; 32], height: u64, inputs: Option<Vec<ScannedInput>>, outputs: Option<Vec<ScannedOutput>>, rpc_url: String, rpc_port: u16, max_subadress_index: u32, notification_channel: Option<mpsc::Sender<ServiceNotification>>) -> Result<Self, String> {
        let view_pair = match ViewPair::new(Scalar::from_bytes_mod_order(priv_spend) * ED25519_BASEPOINT_POINT, Zeroizing::new(Scalar::from_bytes_mod_order(Scalar::from_bytes_mod_order(keccak256(priv_spend)).to_bytes()))) {
            Err(e) => return Err(format!("Error creating ViewPair: {}", e)),
            Ok(vp) => vp,
        };
        let mut scanner = Scanner::new(view_pair);
        for i in 1..=max_subadress_index {
            let subaddr = match SubaddressIndex::new(0u32, i) {
                None => return Err(format!("Error creating SubaddressIndex for index {}", i)),
                Some(idx) => idx,
            };
            scanner.register_subaddress(subaddr);
        }
        let rpc = FastscanRpc::new(rpc_url, rpc_port).await;
        let ch = match rpc.get_height().await {
            Err(e) => {
                return Err(format!("Failed to get chain height from RPC: {}", e));
            }
            Ok(h) => {
                h
            }
        };
        Ok(Self {
            priv_spend,
            service_height: height,
            chain_height: ch,
            inputs: inputs.unwrap_or(Vec::new()),
            outputs: outputs.unwrap_or(Vec::new()),
            rpc,
            scanner,
            status: ServiceStatus::NotScanning,
            notification_channel,
        })
    }

    pub async fn run(mut self, mut rx: mpsc::Receiver<(ServiceCommand, oneshot::Sender<ServiceResponse>)>) {
        let (block_tx, mut block_rx) = mpsc::channel::<QueueMessage>(250);
        let mut running = false;
        let mut producer_handle: Option<tokio::task::JoinHandle<()>> = None;
        loop {
            tokio::select! {
            Some((command, resp_tx)) = rx.recv() => {
                match command {
                    ServiceCommand::Start => {
                        if !running {
                            running = true;
                            self.status = ServiceStatus::Scanning;
                            let service_clone = self.clone();
                            let block_tx_clone = block_tx.clone();
                            producer_handle = Some(tokio::spawn(async move {
                                let mut queue_height = service_clone.service_height;
                                while queue_height < service_clone.chain_height {
                                    let end_height = (queue_height + 50).min(service_clone.chain_height);
                                    let block_nums: Vec<u64> = (queue_height + 1..=end_height).collect();
                                    match service_clone.rpc.get_scannable_blocks_by_number(block_nums).await {
                                        Err(_) => {
                                                continue;
                                            },
                                        Ok(blocks) => {
                                            for block in blocks {
                                                if block_tx_clone.send(NewBlock(block)).await.is_err() {
                                                    break;
                                                }
                                                queue_height += 1;
                                            }
                                        }
                                    }
                                }
                            }));
                        }
                        let _ = resp_tx.send(ServiceResponse::Started);
                    }
                    ServiceCommand::Stop => {
                        if running {
                            running = false;
                            self.status = ServiceStatus::NotScanning;
                            if let Some(handle) = producer_handle.take() {
                                handle.abort();
                            }
                        }
                        let _ = resp_tx.send(ServiceResponse::Stopped);
                    }
                    ServiceCommand::Status => {
                        let _ = resp_tx.send(ServiceResponse::Status(self.status.clone(), self.service_height));
                    }
                    ServiceCommand::Transactions => {
                        let _ = resp_tx.send(ServiceResponse::Transactions(self.inputs.clone(), self.outputs.clone()));
                    }
                    ServiceCommand::Data => {
                        let _ = resp_tx.send(ServiceResponse::Data {
                            status: self.status.clone(),
                            height: self.service_height,
                            inputs: self.inputs.clone(),
                            outputs: self.outputs.clone(),
                        });
                    }
                }
            }
            Some(queue_message) = block_rx.recv(), if running => {
                loop {
                    match queue_message.clone() {
                        NewBlock(block) => {
                            match self.scanner.scan(block.clone()) {
                                Err(_) => continue,
                                Ok(outs) => {
                                    let outputs = outs.not_additionally_locked();
                                    for output in outputs {
                                        let scanned = ScannedOutput {
                                            height: self.service_height + 1,
                                            key_image: ((Scalar::from_bytes_mod_order(self.priv_spend) + output.key_offset())
                                                * biased_hash_to_point(*output.key().compress().as_bytes())).compress().to_bytes(),
                                            amount: output.commitment().amount,
                                            hash: output.transaction(),
                                            timestamp: block.block.header.timestamp,
                                            index: Some(output.index_in_transaction()),
                                        };
                                        self.outputs.push(scanned.clone());
                                        if let Some(ch) = &self.notification_channel { let _ = ch.send(ServiceNotification::FoundTransaction).await; };
                                    }
                                    for (index, tx) in block.transactions.iter().enumerate() {
                                        for input in tx.prefix().inputs.iter() {
                                            if let Input::ToKey { key_image, .. } = input {
                                                for output in &self.outputs {
                                                    if output.key_image == key_image.to_bytes() {
                                                        let scanned = ScannedInput {
                                                            height: self.service_height + 1,
                                                            key_image: key_image.to_bytes(),
                                                            amount: output.amount,
                                                            hash: block.block.transactions[index],
                                                            timestamp: block.block.header.timestamp,
                                                            previous_output: output.hash,
                                                        };
                                                        self.inputs.push(scanned.clone());
                                                        if let Some(ch) = &self.notification_channel { let _ = ch.send(ServiceNotification::FoundTransaction).await; }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    self.service_height += 1;
                                    break;
                                }
                            }
                        }
                        QueueMessage::Error(err) => {
                            self.status = ServiceStatus::Error(err.clone());
                            running = false;
                            if let Some(handle) = producer_handle.take() {
                                handle.abort();
                            }
                            break;
                        }
                    }
                }
            }
            else => break,
        }
        }
        if let Some(handle) = producer_handle {
            let _ = handle.await;
        }
        self.status = ServiceStatus::Scanned;
    }

    fn clone(&self) -> Self {
        Self {
            priv_spend: self.priv_spend,
            service_height: self.service_height,
            chain_height: self.chain_height,
            outputs: self.outputs.clone(),
            inputs: self.inputs.clone(),
            rpc: self.rpc.clone(),
            scanner: self.scanner.clone(),
            status: self.status.clone(),
            notification_channel: self.notification_channel.clone(),
        }
    }
}