use anyhow::Result;
use nexus_transfer::{network::Network, platform, transfer::{FileTransfer, Message}};
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::Arc;
use uuid::Uuid;

#[tokio::main]
async fn main() -> Result<()> {
    println!("NexusTransfer - {} - LAN File Transfer & Chat", platform::get_platform_name());

    print!("Enter your name: ");
    io::stdout().flush()?;
    let mut name = String::new();
    io::stdin().read_line(&mut name)?;
    let name = name.trim().to_string();

    let network = Arc::new(Network::new(name.clone(), 9876)?);
    let file_transfer = Arc::new(FileTransfer::new());

    // Start discovery
    network.start_discovery().await?;
    println!("[*] Starting peer discovery...");

    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

    // Start listener
    let net_clone = network.clone();
    let ft_clone = file_transfer.clone();
    network.start_listener(move |msg| {
        let net = net_clone.clone();
        let ft = ft_clone.clone();
        tokio::spawn(async move {
            handle_message(msg, net, ft).await;
        });
    }).await?;

    println!("[*] Listening on port 9876");
    println!("\nCommands:");
    println!("  /peers              - List discovered peers");
    println!("  /send <id> <text>   - Send text message");
    println!("  /file <id> <path>   - Send file");
    println!("  /quit               - Exit");
    println!();

    // Command loop
    let stdin = io::stdin();
    loop {
        print!("> ");
        io::stdout().flush()?;

        let mut input = String::new();
        stdin.read_line(&mut input)?;
        let input = input.trim();

        if input.is_empty() {
            continue;
        }

        if input == "/quit" {
            break;
        }

        if input == "/peers" {
            let peers = network.list_peers().await;
            if peers.is_empty() {
                println!("No peers found");
            } else {
                println!("Peers:");
                for peer in peers {
                    println!("  {} - {} ({})", peer.id, peer.name, peer.addr);
                }
            }
            continue;
        }

        if let Some(rest) = input.strip_prefix("/send ") {
            let parts: Vec<&str> = rest.splitn(2, ' ').collect();
            if parts.len() != 2 {
                println!("Usage: /send <peer_id> <message>");
                continue;
            }

            match Uuid::parse_str(parts[0]) {
                Ok(peer_id) => {
                    let msg = Message::Text { content: parts[1].to_string() };
                    if let Err(e) = network.send_message(peer_id, msg).await {
                        println!("[!] Failed to send: {}", e);
                    } else {
                        println!("[✓] Sent");
                    }
                }
                Err(_) => println!("[!] Invalid peer ID"),
            }
            continue;
        }

        if let Some(rest) = input.strip_prefix("/file ") {
            let parts: Vec<&str> = rest.splitn(2, ' ').collect();
            if parts.len() != 2 {
                println!("Usage: /file <peer_id> <path>");
                continue;
            }

            match Uuid::parse_str(parts[0]) {
                Ok(peer_id) => {
                    let path = PathBuf::from(parts[1]);
                    match file_transfer.prepare_send(path).await {
                        Ok((id, name, size)) => {
                            let msg = Message::FileOffer { name, size, id };
                            if let Err(e) = network.send_message(peer_id, msg).await {
                                println!("[!] Failed to send offer: {}", e);
                            } else {
                                println!("[✓] File offer sent, waiting for acceptance...");
                            }
                        }
                        Err(e) => println!("[!] Failed to prepare file: {}", e),
                    }
                }
                Err(_) => println!("[!] Invalid peer ID"),
            }
            continue;
        }

        println!("[!] Unknown command");
    }

    println!("Shutting down...");
    Ok(())
}

async fn handle_message(msg: Message, _network: Arc<Network>, file_transfer: Arc<FileTransfer>) {
    match msg {
        Message::Text { content } => {
            println!("\n[MSG] {}", content);
            print!("> ");
            io::stdout().flush().unwrap();
        }
        Message::FileOffer { name, size, id } => {
            println!("\n[FILE] Offer: {} ({} bytes) [id: {}]", name, size, id);
            println!("[FILE] Auto-accepting to downloads/");

            match file_transfer.prepare_receive(id, name, size).await {
                Ok(path) => {
                    println!("[FILE] Saving to: {}", path.display());
                    // In real impl, send accept and handle chunks
                }
                Err(e) => println!("[!] Failed to prepare receive: {}", e),
            }
            print!("> ");
            io::stdout().flush().unwrap();
        }
        Message::FileChunk { id, offset, data } => {
            match file_transfer.receive_chunk(id, offset, data).await {
                Ok(complete) => {
                    if complete {
                        println!("\n[FILE] Transfer complete!");
                        file_transfer.complete(id).await;
                    }
                }
                Err(e) => println!("\n[!] Chunk error: {}", e),
            }
        }
        _ => {}
    }
}
