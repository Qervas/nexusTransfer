use anyhow::Result;
use mdns_sd::{ServiceDaemon, ServiceInfo};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::transfer::{Message, Peer};

const SERVICE_TYPE: &str = "_nexustransfer._tcp.local.";

pub struct Network {
    pub peer_id: Uuid,
    pub peer_name: String,
    pub port: u16,
    pub peers: Arc<RwLock<HashMap<Uuid, Peer>>>,
    mdns: ServiceDaemon,
}

impl Network {
    pub fn new(name: String, port: u16) -> Result<Self> {
        let mdns = ServiceDaemon::new()?;
        Ok(Self {
            peer_id: Uuid::new_v4(),
            peer_name: name,
            port,
            peers: Arc::new(RwLock::new(HashMap::new())),
            mdns,
        })
    }

    pub async fn start_discovery(&self) -> Result<()> {
        let service_info = ServiceInfo::new(
            SERVICE_TYPE,
            &self.peer_name,
            &format!("{}.local.", self.peer_name),
            "",
            self.port,
            None,
        )?;

        self.mdns.register(service_info)?;

        let receiver = self.mdns.browse(SERVICE_TYPE)?;
        let peers = self.peers.clone();
        let my_id = self.peer_id;

        tokio::spawn(async move {
            while let Ok(event) = receiver.recv_async().await {
                match event {
                    mdns_sd::ServiceEvent::ServiceResolved(info) => {
                        if let Some(addr) = info.get_addresses().iter().next() {
                            let peer = Peer {
                                id: Uuid::new_v4(), // In real impl, should be from TXT record
                                name: info.get_fullname().to_string(),
                                addr: format!("{}:{}", addr, info.get_port()),
                            };

                            if peer.id != my_id {
                                peers.write().await.insert(peer.id, peer);
                            }
                        }
                    }
                    mdns_sd::ServiceEvent::ServiceRemoved(_, fullname) => {
                        let mut peers = peers.write().await;
                        peers.retain(|_, p| p.name != fullname);
                    }
                    _ => {}
                }
            }
        });

        Ok(())
    }

    pub async fn start_listener<F>(&self, on_message: F) -> Result<()>
    where
        F: Fn(Message) + Send + Sync + 'static,
    {
        let listener = TcpListener::bind(format!("0.0.0.0:{}", self.port)).await?;
        let on_message = Arc::new(on_message);

        tokio::spawn(async move {
            loop {
                if let Ok((stream, _)) = listener.accept().await {
                    let callback = on_message.clone();
                    tokio::spawn(async move {
                        if let Err(e) = handle_connection(stream, callback).await {
                            eprintln!("Connection error: {}", e);
                        }
                    });
                }
            }
        });

        Ok(())
    }

    pub async fn send_message(&self, peer_id: Uuid, msg: Message) -> Result<()> {
        let peers = self.peers.read().await;
        let peer = peers.get(&peer_id).ok_or_else(|| anyhow::anyhow!("Peer not found"))?;

        let mut stream = TcpStream::connect(&peer.addr).await?;
        let data = msg.encode()?;
        let len = data.len() as u32;

        stream.write_all(&len.to_be_bytes()).await?;
        stream.write_all(&data).await?;
        stream.flush().await?;

        Ok(())
    }

    pub async fn list_peers(&self) -> Vec<Peer> {
        self.peers.read().await.values().cloned().collect()
    }
}

async fn handle_connection<F>(mut stream: TcpStream, on_message: Arc<F>) -> Result<()>
where
    F: Fn(Message) + Send + Sync,
{
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;

    let mut buffer = vec![0u8; len];
    stream.read_exact(&mut buffer).await?;

    let msg = Message::decode(&buffer)?;
    on_message(msg);

    Ok(())
}
