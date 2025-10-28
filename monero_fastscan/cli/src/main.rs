use monero_fastscan::service::{FastscanService, ServiceCommand, ServiceResponse, ServiceStatus};
use std::io::{stdin, stdout, Write};
use tokio::sync::{mpsc, oneshot};

#[tokio::main]
async fn main() {
    let introduction = "
------------------------------------------------------------------------------------------------
▗▖  ▗▖ ▗▄▖ ▗▖  ▗▖▗▄▄▄▖▗▄▄▖  ▗▄▖     ▗▄▄▄▖ ▗▄▖  ▗▄▄▖▗▄▄▄▖▗▄▄▖ ▗▄▄▖ ▗▄▖ ▗▖  ▗▖     ▗▄▄▖▗▖   ▗▄▄▄▖
▐▛▚▞▜▌▐▌ ▐▌▐▛▚▖▐▌▐▌   ▐▌ ▐▌▐▌ ▐▌    ▐▌   ▐▌ ▐▌▐▌     █ ▐▌   ▐▌   ▐▌ ▐▌▐▛▚▖▐▌    ▐▌   ▐▌     █
▐▌  ▐▌▐▌ ▐▌▐▌ ▝▜▌▐▛▀▀▘▐▛▀▚▖▐▌ ▐▌    ▐▛▀▀▘▐▛▀▜▌ ▝▀▚▖  █  ▝▀▚▖▐▌   ▐▛▀▜▌▐▌ ▝▜▌    ▐▌   ▐▌     █
▐▌  ▐▌▝▚▄▞▘▐▌  ▐▌▐▙▄▄▖▐▌ ▐▌▝▚▄▞▘    ▐▌   ▐▌ ▐▌▗▄▄▞▘  █ ▗▄▄▞▘▝▚▄▄▖▐▌ ▐▌▐▌  ▐▌    ▝▚▄▄▖▐▙▄▄▖▗▄█▄▖
------------------------------------------------------------------------------------------------
Welcome to Monero Fastscan CLI Tool! This tool allows you to scan Monero blockchain for incoming and spent outputs based on your private spend key and block height. This tool needs your private spend key to derive the private view key and public spend key, which are essential for scanning the blockchain. Please ensure you trust this tool before proceeding, as handling private keys involves significant security risks. By using this tool, you acknowledge that you understand the risks involved in sharing your private keys and take full responsibility for any consequences that may arise.

DO YOU UNDERSTAND AND ACCEPT THESE RISKS? IF SO, ACKNOWLEDGE BY TYPING 'YES, I UNDERSTAND' BELOW.
";
    println!("{}", introduction);

    // Take user acknowledgment
    let mut acknowledgment = String::new();
    print!("Type 'YES, I UNDERSTAND' to proceed: ");
    stdout().flush().expect("Failed to flush stdout");
    stdin().read_line(&mut acknowledgment).expect("Failed to read line");
    if acknowledgment.trim() != "YES, I UNDERSTAND" {
        println!("Acknowledgment not received. Exiting.");
        return;
    } else {
        println!("Acknowledgment received. Proceeding...");
    }

    // Take private spend key input
    let mut input = String::new();
    print!("Please enter your 64-char hexadecimal private spend key: ");
    stdout().flush().expect("Failed to flush stdout");
    stdin().read_line(&mut input).expect("Failed to read line");

    // Validate the input
    let priv_spend = input.trim();
    if priv_spend.len() != 64 {
        println!("Invalid key length");
        return;
    }
    let priv_spend: [u8; 32] = match hex::decode(priv_spend) {
        Ok(key) => key.try_into().unwrap(),
        Err(_) => {
            println!("Invalid hex string");
            return;
        }
    };

    // Take block height input
    let mut input = String::new();
    print!("Please enter block height: ");
    stdout().flush().expect("Failed to flush stdout");
    stdin().read_line(&mut input).expect("Failed to read line");

    // Validate the input to be integer
    let block_height: u64 = match input.trim().parse() {
        Ok(height) => height,
        Err(_) => {
            println!("Invalid block height");
            return;
        }
    };
    if block_height == 0 {
        println!("Block height must be greater than 0");
        return;
    }

    // Take RPC node URL input (optional)
    let mut input = String::new();
    print!("Please enter RPC node URL (default: https://monero.stackwallet.com:18081): ");
    stdout().flush().expect("Failed to flush stdout");
    stdin().read_line(&mut input).expect("Failed to read line");
    let rpc_url = if input.trim().is_empty() {
        "https://monero.stackwallet.com:18081".to_string()
    } else {
        input.trim().to_string()
    };

    // Split rpc_url into base url and port
    let parts = rpc_url.split(':').collect::<Vec<&str>>();
    let mut rpc_url = String::from("https://monero.stackwallet.com".to_string());
    let mut port = 18081;
    if parts.len() >= 2 {
        rpc_url = parts[0..parts.len() - 1].join(":");
        port = parts[parts.len() - 1].parse::<u16>().unwrap_or(18081);
    }

    let (tx, rx) = mpsc::channel(2);
    let service = match FastscanService::new(priv_spend, block_height, rpc_url, port,200).await {
        Ok(s) => s,
        Err(e) => {
            println!("Failed to create service: {}", e);
            return;
        }
    };
    tokio::spawn(service.run(rx));
    let commands = "---\nCommands:\nh, help - Show this help message\ns, status - Show current height\nt, transactions - Show transactions\nstart - Start scanning\nstop - Stop scanning\n---";
    println!("{}", commands);
    println!("Current status: Stopped. Type 'start' to begin scanning.\n---");
    loop {
        print!("> ");
        stdout().flush().unwrap(); // flush prompt
        let mut line = String::new();
        if stdin().read_line(&mut line).is_err() {
            break;
        }
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }
        let (resp_tx, resp_rx) = oneshot::channel();
        if line == "h" || line == "help" {
            println!("{}", commands);
            continue;
        }
        let command = match line.as_str() {
            "s" | "status" => ServiceCommand::Status,
            "t" | "transactions" => ServiceCommand::Transactions,
            "start" => ServiceCommand::Start,
            "stop" => ServiceCommand::Stop,
            _ => {
                println!("Unknown command. Type 'h' or 'help' for a list of commands.");
                continue;
            }
        };
        if tx.send((command, resp_tx)).await.is_err() {
            println!("Service stopped.");
            break;
        }
        if let Ok(response) = resp_rx.await {
            match response {
                ServiceResponse::Status(status, height) => {
                    match status {
                        ServiceStatus::Scanning => println!("Scanning. Current height: {}", height),
                        ServiceStatus::NotScanning => println!("Stopped (not scanning). Last scanned height: {}", height),
                        ServiceStatus::Scanned => println!("Service is synced with the chain. Last scanned height: {}", height),
                        ServiceStatus::Error(err) => println!("Service gave error: {}", err),
                    }
                }
                ServiceResponse::Transactions(inputs, outputs ) => {
                    if inputs.is_empty() && outputs.is_empty() {
                        println!("No transactions found.");
                        continue;
                    } else {
                        if !outputs.is_empty() {
                            println!("--- Incoming Outputs ---");
                            for output in outputs {
                                println!("{}", &format!("Height: {}, Amount: {}, Key Image: {}", output.height, output.amount, hex::encode(output.key_image)));
                            }
                        }
                        if !inputs.is_empty() {
                            println!("--- Spent Outputs ---");
                            for input in inputs {
                                println!("{}", &format!("Height: {}, Amount: {}, Key Image: {}", input.height, input.amount, hex::encode(input.key_image)));
                            }
                        }
                        println!("------------------------");
                    }
                }
                ServiceResponse::Error(err) => {
                    println!("Error: {}", err);
                }
                ServiceResponse::Started => {
                    println!("Service started.");
                }
                ServiceResponse::Stopped => {
                    println!("Service stopped.");
                }
            }
        }
    }
}