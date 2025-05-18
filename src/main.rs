use std::io::Write;
use serde_json::json;
use reqwest::blocking::Client;

fn main() {
    // Take block height input
    let mut input = String::new();
    print!("Please enter your private spend key: ");
    std::io::stdout().flush().expect("Failed to flush stdout");
    std::io::stdin().read_line(&mut input).expect("Failed to read line");
    
    // Validate the input
    let private_spend_key = input.trim();
    let private_spend_key = match hex::decode(private_spend_key) {
        Ok(key) => key,
        Err(_) => {
            println!("Invalid hex string");
            return;
        }
    };
    if private_spend_key.len() != 32 {
        println!("Invalid key length");
        return;
    }

    // Take block height input
    let mut input = String::new();
    print!("Please enter block height: ");
    std::io::stdout().flush().expect("Failed to flush stdout");
    std::io::stdin().read_line(&mut input).expect("Failed to read line");

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

    println!("Private spend key: {:?}, block height: {}. Moving forward to syncing process.", hex::encode(private_spend_key), block_height);
    
    // Fetch current block height from node
    let client = Client::new();
    let res = client.post("https://monero.stackwallet.com:18081/json_rpc").json(&json!({
        "jsonrpc": "2.0",
        "id": "0",
        "method": "get_block_count"
    })).send().expect("Failed to send request").json::<serde_json::Value>().expect("Failed to parse response");
    let current_height = res.get("result").and_then(|r| r.get("count")).and_then(|c| c.as_u64()).expect("Failed to get current block height");
    println!("Current block height: {}", current_height);

    // Cycle through blocks
    for i in block_height..current_height {
        println!("Processing block height: {}", i);
    }
}
