#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Context {
    pub tokens: Option<u64>,
    pub window: u64,
    pub compacts_at: Option<u64>,
}
