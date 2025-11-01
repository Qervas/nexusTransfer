use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::sync::RwLock;
use uuid::Uuid;

const CHUNK_SIZE: usize = 65536; // 64KB

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Peer {
    pub id: Uuid,
    pub name: String,
    pub addr: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum Message {
    Text { content: String },
    FileOffer { name: String, size: u64, id: Uuid },
    FileAccept { id: Uuid },
    FileReject { id: Uuid },
    FileChunk { id: Uuid, offset: u64, data: Vec<u8> },
    FileComplete { id: Uuid },
}

impl Message {
    pub fn encode(&self) -> anyhow::Result<Vec<u8>> {
        Ok(bincode::serialize(self)?)
    }

    pub fn decode(bytes: &[u8]) -> anyhow::Result<Self> {
        Ok(bincode::deserialize(bytes)?)
    }
}

pub struct FileTransfer {
    active_sends: Arc<RwLock<HashMap<Uuid, PathBuf>>>,
    active_receives: Arc<RwLock<HashMap<Uuid, FileReceive>>>,
}

struct FileReceive {
    #[allow(dead_code)]
    path: PathBuf,
    file: File,
    size: u64,
    received: u64,
}

impl FileTransfer {
    pub fn new() -> Self {
        Self {
            active_sends: Arc::new(RwLock::new(HashMap::new())),
            active_receives: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn prepare_send(&self, path: PathBuf) -> Result<(Uuid, String, u64)> {
        let id = Uuid::new_v4();
        let metadata = tokio::fs::metadata(&path).await?;
        let name = path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();

        self.active_sends.write().await.insert(id, path);

        Ok((id, name, metadata.len()))
    }

    pub async fn send_chunk(&self, id: Uuid, offset: u64) -> Result<Option<Vec<u8>>> {
        let sends = self.active_sends.read().await;
        let path = sends.get(&id).ok_or_else(|| anyhow::anyhow!("File not found"))?;

        let mut file = File::open(path).await?;
        file.seek(std::io::SeekFrom::Start(offset)).await?;

        let mut buffer = vec![0u8; CHUNK_SIZE];
        let n = file.read(&mut buffer).await?;

        if n == 0 {
            return Ok(None);
        }

        buffer.truncate(n);
        Ok(Some(buffer))
    }

    pub async fn prepare_receive(&self, id: Uuid, name: String, size: u64) -> Result<PathBuf> {
        let path = PathBuf::from(format!("downloads/{}", name));
        tokio::fs::create_dir_all("downloads").await?;

        let file = File::create(&path).await?;

        self.active_receives.write().await.insert(
            id,
            FileReceive {
                path: path.clone(),
                file,
                size,
                received: 0,
            },
        );

        Ok(path)
    }

    pub async fn receive_chunk(&self, id: Uuid, _offset: u64, data: Vec<u8>) -> Result<bool> {
        let mut receives = self.active_receives.write().await;
        let receive = receives.get_mut(&id).ok_or_else(|| anyhow::anyhow!("Transfer not found"))?;

        receive.file.write_all(&data).await?;
        receive.received += data.len() as u64;

        Ok(receive.received >= receive.size)
    }

    pub async fn complete(&self, id: Uuid) {
        self.active_sends.write().await.remove(&id);
        self.active_receives.write().await.remove(&id);
    }
}
