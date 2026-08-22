use super::ProductiveRecoveryWriter;
use crate::runtime::{
    context::ContextHistory,
    conversations::{
        contract::protocol,
        recovery_record::{PRODUCTIVE_RECOVERY_SCHEMA_V0, ProductiveRecoveryRecord},
    },
    run_attempts::{ProductiveRecovery, RunAttemptKind, RunAttemptResult},
    types::RuntimeError,
};

impl ProductiveRecoveryWriter {
    fn next_replay_record(&self) -> Option<ProductiveRecoveryRecord> {
        self.replay_records.get(self.replay_cursor).cloned()
    }

    fn consume_replay_record(&mut self) {
        self.replay_cursor = self.replay_cursor.saturating_add(1);
    }

    fn ensure_live_append(&self) -> Result<(), RuntimeError> {
        if self.replay_cursor != self.replay_records.len() || self.terminal_snapshot_hash.is_some()
        {
            return Err(protocol(
                "productive recovery replay diverged before its recorded boundary",
            ));
        }
        Ok(())
    }

    fn ensure_completed_attempts_consumed(&self) -> Result<(), RuntimeError> {
        if self.consumed_attempts.len() != self.completed_attempts.len()
            || self
                .completed_attempts
                .keys()
                .any(|attempt| !self.consumed_attempts.contains(attempt))
        {
            return Err(protocol(
                "productive recovery terminal boundary left completed attempts unreplayed",
            ));
        }
        Ok(())
    }

    fn verify_value_object(
        &self,
        uri: &str,
        value: &serde_json::Value,
    ) -> Result<(), RuntimeError> {
        let recorded = self.run_objects.read(uri)?;
        let expected = proto::canonical_json(value)
            .map_err(|error| protocol(format!("recovery value is invalid: {error}")))?;
        if recorded != expected.as_bytes() {
            return Err(protocol(
                "productive recovery boundary result diverged from deterministic replay",
            ));
        }
        Ok(())
    }

    fn attempt_record(
        &self,
        request_hash: &str,
        result: &RunAttemptResult,
        tool_id: Option<&str>,
    ) -> Result<ProductiveRecoveryRecord, RuntimeError> {
        let durable_output = result.durable_output.clone().ok_or_else(|| {
            protocol("completed productive attempt has no durable recovery output")
        })?;
        Ok(match (result.attempt_kind, tool_id) {
            (RunAttemptKind::Provider, None) => ProductiveRecoveryRecord::Provider {
                schema: PRODUCTIVE_RECOVERY_SCHEMA_V0.to_owned(),
                attempt_id: result.attempt_id.clone(),
                request_hash: request_hash.to_owned(),
                outcome: result.outcome,
                classification: result.classification.clone(),
                exit_code: result.exit_code,
                timestamp: result.timestamp.clone(),
                durable_output,
            },
            (RunAttemptKind::Tool, Some(tool_id)) => ProductiveRecoveryRecord::Tool {
                schema: PRODUCTIVE_RECOVERY_SCHEMA_V0.to_owned(),
                attempt_id: result.attempt_id.clone(),
                request_hash: request_hash.to_owned(),
                tool_id: tool_id.to_owned(),
                outcome: result.outcome,
                classification: result.classification.clone(),
                exit_code: result.exit_code,
                timestamp: result.timestamp.clone(),
                durable_output,
            },
            (RunAttemptKind::Provider, Some(_)) => {
                return Err(protocol(
                    "completed recovery Provider attempt has a Tool id",
                ));
            }
            (RunAttemptKind::Tool, None) => {
                return Err(protocol("completed recovery Tool attempt has no Tool id"));
            }
        })
    }
}

impl ProductiveRecovery for ProductiveRecoveryWriter {
    fn recover_attempt(
        &mut self,
        kind: RunAttemptKind,
        attempt_id: &str,
        request_hash: &str,
        tool_id: Option<&str>,
    ) -> Result<Option<RunAttemptResult>, RuntimeError> {
        self.ensure_usable()?;
        if self.consumed_attempts.contains(attempt_id) {
            return Err(protocol(
                "productive recovery completed attempt was already consumed",
            ));
        }
        if let Some(record) = self.next_replay_record() {
            let (record_kind, record_attempt_id, record_request_hash, record_tool_id, result) =
                match record {
                    ProductiveRecoveryRecord::Provider {
                        attempt_id,
                        request_hash,
                        outcome,
                        classification,
                        exit_code,
                        timestamp,
                        durable_output,
                        ..
                    } => (
                        RunAttemptKind::Provider,
                        attempt_id.clone(),
                        request_hash,
                        None,
                        RunAttemptResult {
                            attempt_id,
                            attempt_kind: RunAttemptKind::Provider,
                            outcome,
                            classification,
                            exit_code,
                            timestamp,
                            durable_output: Some(durable_output),
                        },
                    ),
                    ProductiveRecoveryRecord::Tool {
                        attempt_id,
                        request_hash,
                        tool_id,
                        outcome,
                        classification,
                        exit_code,
                        timestamp,
                        durable_output,
                        ..
                    } => (
                        RunAttemptKind::Tool,
                        attempt_id.clone(),
                        request_hash,
                        Some(tool_id.clone()),
                        RunAttemptResult {
                            attempt_id,
                            attempt_kind: RunAttemptKind::Tool,
                            outcome,
                            classification,
                            exit_code,
                            timestamp,
                            durable_output: Some(durable_output),
                        },
                    ),
                    _ => {
                        return Err(protocol(
                            "productive recovery replay reached a different boundary than the next attempt",
                        ));
                    }
                };
            if record_kind != kind
                || record_attempt_id != attempt_id
                || record_request_hash != request_hash
                || record_tool_id.as_deref() != tool_id
            {
                return Err(protocol(
                    "productive recovery attempt diverged from its recorded request",
                ));
            }
            let completed = self.completed_attempts.get(attempt_id).ok_or_else(|| {
                protocol("productive recovery attempt has no completed run-log result")
            })?;
            if completed.request_hash != request_hash
                || completed.tool_id.as_deref() != tool_id
                || completed.result != result
            {
                return Err(protocol(
                    "productive recovery attempt conflicts with its completed run-log result",
                ));
            }
            self.consumed_attempts.insert(attempt_id.to_owned());
            self.consume_replay_record();
            return Ok(Some(result));
        }

        if let Some(completed) = self.completed_attempts.get(attempt_id).cloned() {
            if self.terminal_snapshot_hash.is_some() {
                return Err(protocol(
                    "productive recovery terminal snapshot has an unrecorded completed attempt",
                ));
            }
            if completed.result.attempt_kind != kind
                || completed.request_hash != request_hash
                || completed.tool_id.as_deref() != tool_id
            {
                return Err(protocol(
                    "completed productive attempt diverged from deterministic replay",
                ));
            }
            let record = self.attempt_record(
                request_hash,
                &completed.result,
                completed.tool_id.as_deref(),
            )?;
            self.append_record(&record)?;
            self.consumed_attempts.insert(attempt_id.to_owned());
            return Ok(Some(completed.result));
        }

        self.ensure_live_append()?;
        self.ensure_completed_attempts_consumed()?;
        Ok(None)
    }

    fn record_attempt(
        &mut self,
        tool_id: Option<&str>,
        request_hash: &str,
        result: &RunAttemptResult,
    ) -> Result<(), RuntimeError> {
        self.ensure_usable()?;
        self.ensure_live_append()?;
        let record = self.attempt_record(request_hash, result, tool_id)?;
        self.append_record(&record)
    }

    fn phase_boundary(
        &mut self,
        flow_execution_id: &str,
        phase_execution_id: &str,
        phase_id: &str,
        iteration: u8,
        result: &core_script::FlowValue,
        will_repeat: bool,
    ) -> Result<(), RuntimeError> {
        self.ensure_usable()?;
        let result_value = serde_json::to_value(result).map_err(RuntimeError::Json)?;
        if let Some(record) = self.next_replay_record() {
            let ProductiveRecoveryRecord::Phase {
                flow_execution_id: recorded_flow,
                phase_execution_id: recorded_execution,
                phase_id: recorded_phase,
                iteration: recorded_iteration,
                result_object,
                will_repeat: recorded_repeat,
                ..
            } = record
            else {
                return Err(protocol(
                    "productive recovery replay reached a different boundary than Phase completion",
                ));
            };
            if recorded_flow != flow_execution_id
                || recorded_execution != phase_execution_id
                || recorded_phase != phase_id
                || recorded_iteration != u32::from(iteration)
                || recorded_repeat != will_repeat
            {
                return Err(protocol(
                    "productive recovery Phase boundary diverged from deterministic replay",
                ));
            }
            self.verify_value_object(&result_object, &result_value)?;
            self.consume_replay_record();
            return Ok(());
        }
        self.ensure_live_append()?;
        let result_object = self.persist_value(&result_value)?;
        self.append_record(&ProductiveRecoveryRecord::Phase {
            schema: PRODUCTIVE_RECOVERY_SCHEMA_V0.to_owned(),
            flow_execution_id: flow_execution_id.to_owned(),
            phase_execution_id: phase_execution_id.to_owned(),
            phase_id: phase_id.to_owned(),
            iteration: u32::from(iteration),
            result_object,
            will_repeat,
        })
    }

    fn transition_boundary(
        &mut self,
        flow_execution_id: &str,
        from_phase_id: &str,
        to_phase_id: Option<&str>,
    ) -> Result<(), RuntimeError> {
        self.ensure_usable()?;
        if let Some(record) = self.next_replay_record() {
            let ProductiveRecoveryRecord::Transition {
                flow_execution_id: recorded_flow,
                from_phase_id: recorded_from,
                to_phase_id: recorded_to,
                ..
            } = record
            else {
                return Err(protocol(
                    "productive recovery replay reached a different boundary than Transition selection",
                ));
            };
            if recorded_flow != flow_execution_id
                || recorded_from != from_phase_id
                || recorded_to.as_deref() != to_phase_id
            {
                return Err(protocol(
                    "productive recovery Transition boundary diverged from deterministic replay",
                ));
            }
            self.consume_replay_record();
            return Ok(());
        }
        self.ensure_live_append()?;
        self.append_record(&ProductiveRecoveryRecord::Transition {
            schema: PRODUCTIVE_RECOVERY_SCHEMA_V0.to_owned(),
            flow_execution_id: flow_execution_id.to_owned(),
            from_phase_id: from_phase_id.to_owned(),
            to_phase_id: to_phase_id.map(str::to_owned),
        })
    }

    fn flow_boundary(
        &mut self,
        flow_execution_id: &str,
        result: Option<&core_script::FlowValue>,
    ) -> Result<(), RuntimeError> {
        self.ensure_usable()?;
        let result_value = serde_json::to_value(result).map_err(RuntimeError::Json)?;
        if let Some(record) = self.next_replay_record() {
            let ProductiveRecoveryRecord::Flow {
                flow_execution_id: recorded_flow,
                result_object,
                ..
            } = record
            else {
                return Err(protocol(
                    "productive recovery replay reached a different boundary than Flow completion",
                ));
            };
            if recorded_flow != flow_execution_id {
                return Err(protocol(
                    "productive recovery Flow boundary diverged from deterministic replay",
                ));
            }
            self.verify_value_object(&result_object, &result_value)?;
            self.consume_replay_record();
            return Ok(());
        }
        self.ensure_live_append()?;
        let result_object = self.persist_value(&result_value)?;
        self.append_record(&ProductiveRecoveryRecord::Flow {
            schema: PRODUCTIVE_RECOVERY_SCHEMA_V0.to_owned(),
            flow_execution_id: flow_execution_id.to_owned(),
            result_object,
        })
    }

    fn terminal_boundary(
        &mut self,
        history: &ContextHistory,
        failed: bool,
        run_event_count: u64,
    ) -> Result<(), RuntimeError> {
        self.ensure_usable()?;
        let cumulative = self
            .prior_event_count
            .checked_add(run_event_count)
            .ok_or_else(|| protocol("conversation event count overflow"))?;
        let cumulative = usize::try_from(cumulative)
            .map_err(|_| protocol("conversation event count exceeds this platform"))?;
        if let Some(record) = self.next_replay_record() {
            let ProductiveRecoveryRecord::Terminal {
                failed: recorded_failed,
                history_object,
                cumulative_event_count,
                ..
            } = record
            else {
                return Err(protocol(
                    "productive recovery replay reached a different boundary than terminal completion",
                ));
            };
            if recorded_failed != failed
                || cumulative_event_count
                    != u64::try_from(cumulative)
                        .map_err(|_| protocol("conversation event count exceeds u64"))?
            {
                return Err(protocol(
                    "productive recovery terminal boundary diverged from deterministic replay",
                ));
            }
            let expected = history.recovery_object()?;
            if self.run_objects.read(&history_object)? != expected.bytes {
                return Err(protocol(
                    "productive recovery terminal history diverged from deterministic replay",
                ));
            }
            self.consume_replay_record();
            if self.replay_cursor != self.replay_records.len() {
                return Err(protocol(
                    "productive recovery terminal boundary left recorded work unreplayed",
                ));
            }
            self.ensure_completed_attempts_consumed()?;
            return Ok(());
        }
        self.ensure_live_append()?;
        self.ensure_completed_attempts_consumed()?;
        self.append_terminal(history, failed, cumulative)
            .map(|_| ())
    }

    fn read_object(&self, uri: &str) -> Result<Vec<u8>, RuntimeError> {
        self.run_objects.read(uri)
    }

    fn terminal_snapshot_hash(&self) -> Option<&str> {
        self.terminal_snapshot_hash.as_deref()
    }
}
