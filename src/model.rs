use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AssetRecord {
    pub asset_id: String,
    pub category: String,
    pub source: String,
    pub collected_at: u64,
    pub payload: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionError {
    pub collector: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub schema_version: u32,
    pub agent_id: String,
    pub collected_at: u64,
    pub duration_ms: u64,
    pub records: Vec<AssetRecord>,
    pub errors: Vec<CollectionError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvelopeKind {
    Inventory,
    Heartbeat,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Added,
    Updated,
    Removed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetChange {
    pub kind: ChangeKind,
    pub asset_id: String,
    pub category: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record: Option<AssetRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope {
    pub schema_version: u32,
    pub event_id: String,
    pub agent_id: String,
    pub created_at: u64,
    pub kind: EnvelopeKind,
    pub snapshot_hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<Snapshot>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changes: Vec<AssetChange>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub collection_errors: Vec<CollectionError>,
}

pub fn unix_time() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
