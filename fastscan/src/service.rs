use crate::rpc::FastscanRpc;
use curve25519_dalek::constants::ED25519_BASEPOINT_POINT;
use curve25519_dalek::Scalar;
use monero_oxide::transaction::Input;
use monero_wallet::address::SubaddressIndex;
use monero_wallet::generators::biased_hash_to_point;
use monero_wallet::primitives::keccak256;
use monero_wallet::{Scanner, ViewPair};
use monero_wallet::rpc::ScannableBlock;
use serde::Serialize;
use tokio::sync::{mpsc, oneshot};
use zeroize::Zeroizing;

#[derive(Clone, Serialize)]
pub struct ScannedInput {
    pub height: u64,
    pub key_image: [u8; 32],
    pub amount: u64,
}

#[derive(Clone, Serialize)]
pub struct ScannedOutput {
    pub height: u64,
    pub key_image: [u8; 32],
    pub amount: u64,
}

pub enum ServiceCommand {
    Start,
    Stop,
    Status,
    Transactions,
}

#[derive(Serialize)]
pub enum ServiceResponse {
    Started,
    Stopped,
    Status(ServiceStatus, u64),
    Transactions(Vec<ScannedInput>, Vec<ScannedOutput>),
    Error(String),
}

#[derive(Clone, Serialize)]
pub enum ServiceStatus {
    Scanning,
    NotScanning,
    Scanned,
    Error(String),
}

pub struct FastscanService {
    /// Wallet's private spend key
    priv_spend: [u8; 32],
    /// Last scanned height of this wallet
    service_height: u64,
    /// The length of current the longest known chain
    chain_height: u64,
    /// Outputs
    outputs: Vec<ScannedOutput>,
    /// Inputs
    inputs: Vec<ScannedInput>,
    /// The RPC we use to connect to the node
    rpc: FastscanRpc,
    /// The Scanner struct we use to scan the blockchain
    scanner: Scanner,
    /// Status of the service (running, stopped, error, etc.)
    status: ServiceStatus,
}

impl FastscanService {
    pub async fn new(priv_spend: [u8; 32], height: u64, rpc_url: String, rpc_port: u16, max_subadress_index: u32) -> Result<Self, String> {
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
                return Err(format!("Error getting current height: {}", e));
            }
            Ok(h) => {
                h
            }
        };
        Ok(Self {
            priv_spend,
            service_height: height,
            chain_height: ch,
            outputs: Vec::new(),
            inputs: Vec::new(),
            rpc,
            scanner,
            status: ServiceStatus::NotScanning
        })
    }

    pub async fn run(mut self, mut rx: mpsc::Receiver<(ServiceCommand, oneshot::Sender<ServiceResponse>)>) {
        let (block_tx, mut block_rx) = mpsc::channel::<ScannableBlock>(250);
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
                                        Err(_) => continue,
                                        Ok(blocks) => {
                                            for block in blocks {
                                                if block_tx_clone.send(block).await.is_err() {
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
                }
            }
            Some(block) = block_rx.recv(), if running => {
                loop {
                    match self.scanner.scan(block.clone()) {
                        Err(_) => continue,
                        Ok(outs) => {
                            let outputs = outs.not_additionally_locked();
                            for output in outputs {
                                self.outputs.push(ScannedOutput {
                                    height: self.service_height + 1,
                                    key_image: ((Scalar::from_bytes_mod_order(self.priv_spend) + output.key_offset())
                                        * biased_hash_to_point(*output.key().compress().as_bytes())).compress().to_bytes(),
                                    amount: output.commitment().amount,
                                });
                            }
                            block.transactions.iter().for_each(|tx| {
                                tx.prefix().inputs.iter().for_each(|input| {
                                    if let Input::ToKey { key_image, .. } = input {
                                        for output in self.outputs.iter().clone() {
                                            if output.key_image == key_image.to_bytes() {
                                                self.inputs.push(ScannedInput {
                                                    height: self.service_height + 1,
                                                    key_image: key_image.to_bytes(),
                                                    amount: output.amount,
                                                })
                                            }
                                        }
                                    }
                                });
                            });
                            self.service_height += 1;
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

    /*
    pub async fn run(mut self, mut rx: mpsc::Receiver<(ServiceCommand, oneshot::Sender<ServiceResponse>)>) {
        self.status = ServiceStatus::Syncing;
        let (block_tx, mut block_rx) = mpsc::channel::<ScannableBlock>(250);
        let producer = tokio::task::spawn({
            async move {
                let mut queue_height = self.service_height;
                while queue_height < self.chain_height {
                    let end_height = (queue_height + 50).min(self.chain_height);
                    let block_nums: Vec<u64> = (queue_height + 1..=end_height).collect();
                    match self.rpc.get_scannable_blocks_by_number(block_nums).await {
                        Err(_) => continue,
                        Ok(blocks) => {
                            for block in blocks {
                                if block_tx.send(block).await.is_err() {
                                    // Receiver dropped
                                    break;
                                }
                                queue_height += 1;
                            }
                        }
                    }
                }
            }
        });

        loop {
            tokio::select! {
                Some(block) = block_rx.recv() => {
                    loop {
                        match self.scanner.scan(block.clone()) {
                            Err(_) => continue,
                            Ok(outs) => {
                                let outputs = outs.not_additionally_locked();
                                for output in outputs {
                                    self.outputs.push(ScannedOutput {
                                        height: self.service_height + 1,
                                        key_image: ((Scalar::from_bytes_mod_order(self.priv_spend) + output.key_offset()) * biased_hash_to_point(*output.key().compress().as_bytes())).compress().to_bytes(),
                                        amount: output.commitment().amount,
                                    });
                                }
                                block.transactions.iter().for_each(|tx| {
                                    tx.prefix().inputs.iter().for_each(|input| {
                                        if let Input::ToKey { key_image, .. } = input {
                                            for output in self.outputs.iter().clone() {
                                                if output.key_image == key_image.to_bytes() {
                                                    self.inputs.push(ScannedInput {
                                                        height: self.service_height + 1,
                                                        key_image: key_image.to_bytes(),
                                                        amount: output.amount,
                                                    })
                                                }
                                            }
                                        }
                                    });
                                });
                                self.service_height += 1;
                                break;
                            }
                        }
                    }
                }
                Some((command, resp_tx)) = rx.recv() => {
                    let resp = match command {
                        ServiceCommand::Status => ServiceResponse::Status(self.service_height),
                        ServiceCommand::Transactions => ServiceResponse::Transactions(self.inputs.clone(), self.outputs.clone())
                    };
                    let _ = resp_tx.send(resp);
                }
                else => break, // channel closed
            }
        }

        let _ = producer.await;
        self.status = ServiceStatus::Synced;
    }
     */

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
        }
    }
}