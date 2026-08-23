use super::objects::SessionObjectWriter;
use crate::runtime::{
    context::{
        CONTEXT_PROFILE_ID, CONTEXT_PROFILE_VERSION, ContextManifestCheckpoint,
        ContextManifestRecord, OPERATOR_MODEL_PROFILE_ID, STUB_MODEL_PROFILE_ID,
        ensure_context_manifest_growth_within_limit,
    },
    fs_guards::{AnchoredDir, AnchoredFile, for_each_segmented_jsonl_line},
    segmented_appender::{EventLogAppender, SessionLogAppender},
    stream_signature::{CONTEXT_PLAN_DOMAIN, RuntimeStreamSignatureBuilder},
    types::{CONTEXT_MANIFEST_STREAM_LIMITS, RuntimeError},
};
use std::{collections::BTreeSet, path::Path};

pub struct ContextManifestWriter {
    pub(crate) appender: SessionLogAppender,
    pub(crate) byte_count: u64,
    pub(crate) last_manifest: Option<String>,
    pub(crate) manifest_count: usize,
    pub(crate) manifest_signature: RuntimeStreamSignatureBuilder,
    pub(crate) object_writer: Option<SessionObjectWriter>,
}

pub(crate) fn context_manifest_inventory(
    path: &AnchoredFile,
) -> Result<(Option<String>, usize, u64, RuntimeStreamSignatureBuilder), RuntimeError> {
    let mut last_manifest = None;
    let mut manifest_count = 0usize;
    let mut manifest_signature = RuntimeStreamSignatureBuilder::new(CONTEXT_PLAN_DOMAIN);
    let byte_count = for_each_segmented_jsonl_line(path, CONTEXT_MANIFEST_STREAM_LIMITS, |line| {
        validate_context_manifest_line(path.diagnostic_path(), line)?;
        last_manifest = Some(line.to_owned());
        manifest_count = manifest_count.saturating_add(1);
        manifest_signature.push(line.as_bytes());
        Ok(())
    })?;
    Ok((
        last_manifest,
        manifest_count,
        byte_count,
        manifest_signature,
    ))
}

pub(crate) fn validate_context_manifest_checkpoint(
    path: &Path,
    manifest_count: usize,
    last_manifest: Option<&str>,
    checkpoint: &ContextManifestCheckpoint,
) -> Result<bool, RuntimeError> {
    let replay = checkpoint.ordinal == manifest_count;
    if replay && last_manifest != Some(&checkpoint.manifest.line) {
        return Err(RuntimeError::Protocol(format!(
            "{} in-flight context manifest does not match deterministic replay",
            path.display()
        )));
    }
    if !replay && checkpoint.ordinal != manifest_count.saturating_add(1) {
        return Err(RuntimeError::Protocol(format!(
            "{} context manifest ordinal {} does not follow persisted ordinal {}",
            path.display(),
            checkpoint.ordinal,
            manifest_count
        )));
    }
    if checkpoint.manifest.line.is_empty() || !checkpoint.manifest.line.ends_with('\n') {
        return Err(RuntimeError::Protocol(
            "context manifest must be one LF-terminated JSONL record".to_owned(),
        ));
    }
    Ok(replay)
}

pub(crate) fn validate_context_manifest_line(
    path: &Path,
    line: &str,
) -> Result<ContextManifestRecord, RuntimeError> {
    let body = line.strip_suffix('\n').ok_or_else(|| {
        RuntimeError::Protocol("context manifest must be one LF-terminated JSONL record".to_owned())
    })?;
    let value: serde_json::Value = serde_json::from_str(body).map_err(|error| {
        RuntimeError::Protocol(format!(
            "{} invalid context manifest JSON: {error}",
            path.display()
        ))
    })?;
    let manifest: ContextManifestRecord =
        serde_json::from_value(value.clone()).map_err(|error| {
            RuntimeError::Protocol(format!(
                "{} invalid context manifest record: {error}",
                path.display()
            ))
        })?;
    if manifest.context_profile_id != CONTEXT_PROFILE_ID
        || manifest.context_profile_version != CONTEXT_PROFILE_VERSION
        || !matches!(
            manifest.model_profile_id.as_str(),
            STUB_MODEL_PROFILE_ID | OPERATOR_MODEL_PROFILE_ID
        )
    {
        return Err(RuntimeError::Protocol(format!(
            "{} context profile does not match the recorded M1 compiler",
            path.display()
        )));
    }
    let mut canonical = proto::canonical_json(&value).map_err(|error| {
        RuntimeError::Protocol(format!(
            "{} context manifest is not canonicalizable: {error}",
            path.display()
        ))
    })?;
    canonical.push('\n');
    if canonical != line {
        return Err(RuntimeError::Protocol(format!(
            "{} context manifest is not canonical JSONL",
            path.display()
        )));
    }
    for source in &manifest.ordered_sources {
        let digest = core_script::parse_session_object_uri(&source.object_uri).map_err(|_| {
            RuntimeError::Protocol(format!(
                "{} context manifest object_uri is invalid",
                path.display()
            ))
        })?;
        if source.projection_hash != digest {
            return Err(RuntimeError::Protocol(
                "context manifest projection_hash does not match object_uri".to_owned(),
            ));
        }
    }
    Ok(manifest)
}

fn context_manifest_object_digests(manifest: &ContextManifestRecord) -> BTreeSet<String> {
    manifest
        .ordered_sources
        .iter()
        .map(|source| source.projection_hash.clone())
        .collect()
}

impl ContextManifestWriter {
    #[cfg(test)]
    pub(crate) fn open(path: &AnchoredFile) -> Result<Self, RuntimeError> {
        Self::open_with_object_writer(path, None)
    }

    pub(crate) fn open_for_session(
        path: &AnchoredFile,
        object_parent: AnchoredDir,
        session_id: &str,
    ) -> Result<Self, RuntimeError> {
        Self::open_with_object_writer(
            path,
            Some(SessionObjectWriter::open(object_parent, session_id)?),
        )
    }

    pub(crate) fn open_with_object_writer(
        path: &AnchoredFile,
        object_writer: Option<SessionObjectWriter>,
    ) -> Result<Self, RuntimeError> {
        let (last_manifest, manifest_count, byte_count, manifest_signature) =
            context_manifest_inventory(path)?;
        Ok(Self {
            appender: SessionLogAppender::open_with_limits(path, CONTEXT_MANIFEST_STREAM_LIMITS)?,
            byte_count,
            last_manifest,
            manifest_count,
            manifest_signature,
            object_writer,
        })
    }

    pub(crate) fn persist(
        &mut self,
        path: &AnchoredFile,
        checkpoint: &ContextManifestCheckpoint,
    ) -> Result<(), RuntimeError> {
        let replay = validate_context_manifest_checkpoint(
            path.diagnostic_path(),
            self.manifest_count,
            self.last_manifest.as_deref(),
            checkpoint,
        )?;
        let manifest =
            validate_context_manifest_line(path.diagnostic_path(), &checkpoint.manifest.line)?;
        let total = if replay {
            self.byte_count
        } else {
            ensure_context_manifest_growth_within_limit(
                path.diagnostic_path(),
                self.byte_count,
                checkpoint.manifest.line.len(),
            )?
        };
        let actual = self.appender.len(path.diagnostic_path())?;
        if actual != self.byte_count {
            return Err(RuntimeError::Protocol(format!(
                "{} changed outside context manifest append semantics",
                path.diagnostic_path().display()
            )));
        }
        let (_, _, observed_bytes, observed_signature) = context_manifest_inventory(path)?;
        if observed_bytes != self.byte_count
            || observed_signature.signature() != self.manifest_signature.signature()
        {
            return Err(RuntimeError::Protocol(format!(
                "{} changed outside context manifest append semantics",
                path.diagnostic_path().display()
            )));
        }
        if let Some(object_writer) = self.object_writer.as_mut() {
            let referenced = context_manifest_object_digests(&manifest);
            object_writer.persist_manifest_objects(&referenced, &checkpoint.objects)?;
        }
        if replay {
            return self.appender.sync(path.diagnostic_path());
        }
        self.appender
            .append(path.diagnostic_path(), checkpoint.manifest.line.as_bytes())?;
        self.appender.sync(path.diagnostic_path())?;
        self.byte_count = total;
        self.last_manifest = Some(checkpoint.manifest.line.clone());
        self.manifest_count = checkpoint.ordinal;
        self.manifest_signature
            .push(checkpoint.manifest.line.as_bytes());
        Ok(())
    }
}
