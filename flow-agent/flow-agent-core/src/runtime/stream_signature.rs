use sha2::{Digest, Sha256};

pub const EVENT_PLAN_DOMAIN: &[u8] = b"watershed.runtime.event-plan.v1";
pub const CONTEXT_PLAN_DOMAIN: &[u8] = b"watershed.runtime.context-plan.v1";
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeStreamSignature {
    pub(crate) byte_count: usize,
    pub(crate) digest: [u8; 32],
    pub(crate) record_count: usize,
}

#[derive(Clone)]
pub struct RuntimeStreamSignatureBuilder {
    pub(crate) byte_count: usize,
    pub(crate) hasher: Sha256,
    pub(crate) record_count: usize,
}

impl RuntimeStreamSignatureBuilder {
    pub(crate) fn new(domain: &[u8]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(
            u64::try_from(domain.len())
                .unwrap_or(u64::MAX)
                .to_be_bytes(),
        );
        hasher.update(domain);
        Self {
            byte_count: 0,
            hasher,
            record_count: 0,
        }
    }

    pub(crate) fn push(&mut self, record: &[u8]) {
        self.hasher.update(
            u64::try_from(record.len())
                .unwrap_or(u64::MAX)
                .to_be_bytes(),
        );
        self.hasher.update(record);
        self.byte_count = self.byte_count.saturating_add(record.len());
        self.record_count = self.record_count.saturating_add(1);
    }

    pub(crate) fn signature(&self) -> RuntimeStreamSignature {
        RuntimeStreamSignature {
            byte_count: self.byte_count,
            digest: self.hasher.clone().finalize().into(),
            record_count: self.record_count,
        }
    }
}

#[derive(Clone, Debug)]
pub struct FlowInvocation {
    pub(crate) flow_id: String,
    pub(crate) parent_flow_id: Option<String>,
}
