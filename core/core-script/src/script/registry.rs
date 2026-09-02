use crate::script::canonical::{
    canonicalize_registry_block, canonicalize_registry_value, parse_error,
};
use crate::script::error::{RegistryError, SemanticValidationError};
use crate::script::model::{
    FlowBlock, InstructionBlock, MAX_FLOW_FANOUT, MAX_FLOW_NESTING_DEPTH, MAX_PHASE_FANOUT,
    MAX_PHASE_NESTING_DEPTH, PhaseBlock, PhaseTransition, RegistryBlock, RegistryBlockKind,
    ResolvedRegistry, ToolBlock,
};
use crate::script::naming::{insert_named_block, normalize_string};
use crate::script::semantics::{validate_registry_block_semantics, validate_registry_block_shape};
use crate::script::values::validate_predicate_against_contract;
use std::collections::{BTreeMap, BTreeSet};

fn canonicalize_reference(
    name_ids: &BTreeMap<RegistryBlockKind, BTreeMap<String, String>>,
    kind: RegistryBlockKind,
    reference: &mut String,
) {
    if let Some(id) = name_ids.get(&kind).and_then(|names| names.get(reference)) {
        reference.clone_from(id);
    }
}

fn canonicalize_transition_references(
    name_ids: &BTreeMap<RegistryBlockKind, BTreeMap<String, String>>,
    transition: &mut PhaseTransition,
) {
    canonicalize_reference(
        name_ids,
        RegistryBlockKind::Phase,
        &mut transition.from_phase_ref,
    );
    canonicalize_reference(
        name_ids,
        RegistryBlockKind::Phase,
        &mut transition.to_phase_ref,
    );
}

impl ResolvedRegistry {
    /// Resolves a registry from already parsed blocks.
    pub fn from_blocks(
        blocks: impl IntoIterator<Item = RegistryBlock>,
    ) -> Result<Self, RegistryError> {
        let mut registry = Self {
            instructions: BTreeMap::new(),
            flows: BTreeMap::new(),
            phases: BTreeMap::new(),
            tools: BTreeMap::new(),
            name_ids: BTreeMap::new(),
        };
        let mut name_ids: BTreeMap<RegistryBlockKind, BTreeMap<String, String>> = BTreeMap::new();

        for block in blocks {
            validate_registry_block_shape(&block).map_err(|message| {
                let (kind, identity) = block.kind_and_identity();
                if !crate::script::paths::is_valid_block_id(&identity.id) {
                    RegistryError::InvalidBlockId(identity.id.clone())
                } else if identity.name.is_empty() {
                    RegistryError::InvalidBlockName {
                        kind: kind.as_str(),
                        id: identity.id.clone(),
                    }
                } else {
                    parse_error("programmatic registry", message)
                }
            })?;
            let block = canonicalize_registry_block(block)?;
            validate_registry_block_shape(&block)
                .map_err(|message| parse_error("programmatic registry", message))?;
            registry.insert(block, &mut name_ids)?;
        }
        registry.name_ids = name_ids;

        registry.validate_references()?;
        if registry.phases.is_empty() && registry.flows.is_empty() {
            return Ok(registry);
        }
        Ok(registry.with_canonical_references())
    }

    /// Serializes the resolved registry as canonical JSON without a trailing newline.
    pub fn canonical_json(&self) -> Result<String, RegistryError> {
        let mut value = serde_json::to_value(self).map_err(RegistryError::Serialize)?;
        canonicalize_registry_value(&mut value);
        proto::canonical_json(&value).map_err(RegistryError::CanonicalJson)
    }

    fn with_canonical_references(mut self) -> Self {
        let name_ids = &self.name_ids;
        for phase in self.phases.values_mut() {
            for reference in &mut phase.instruction_refs {
                canonicalize_reference(name_ids, RegistryBlockKind::Instruction, reference);
            }
            for reference in &mut phase.tool_refs {
                canonicalize_reference(name_ids, RegistryBlockKind::Tool, reference);
            }
            for reference in &mut phase.phase_refs {
                canonicalize_reference(name_ids, RegistryBlockKind::Phase, reference);
            }
            if let Some(reference) = &mut phase.result_from {
                canonicalize_reference(name_ids, RegistryBlockKind::Phase, reference);
            }
            for transition in &mut phase.transitions {
                canonicalize_transition_references(name_ids, transition);
            }
        }
        for flow_block in self.flows.values_mut() {
            for reference in &mut flow_block.phase_refs {
                canonicalize_reference(name_ids, RegistryBlockKind::Phase, reference);
            }
            for reference in &mut flow_block.subflow_refs {
                canonicalize_reference(name_ids, RegistryBlockKind::Flow, reference);
            }
            for transition in &mut flow_block.transitions {
                canonicalize_transition_references(name_ids, transition);
            }
        }
        self
    }

    /// Resolves a flow by id or unambiguous name.
    pub fn flow_block(&self, reference: &str) -> Option<&FlowBlock> {
        self.named_block(RegistryBlockKind::Flow, reference, &self.flows)
    }

    /// Resolves a phase by id or unambiguous name.
    pub fn phase_block(&self, reference: &str) -> Option<&PhaseBlock> {
        self.named_block(RegistryBlockKind::Phase, reference, &self.phases)
    }

    /// Resolves a tool by id or unambiguous name.
    pub fn tool_block(&self, reference: &str) -> Option<&ToolBlock> {
        self.named_block(RegistryBlockKind::Tool, reference, &self.tools)
    }

    /// Returns tool blocks in canonical id order.
    #[cfg(test)]
    pub(crate) fn tool_blocks(&self) -> impl Iterator<Item = &ToolBlock> {
        self.tools.values()
    }

    /// Resolves an instruction by id or unambiguous name.
    pub fn instruction_block(&self, reference: &str) -> Option<&InstructionBlock> {
        self.named_block(
            RegistryBlockKind::Instruction,
            reference,
            &self.instructions,
        )
    }

    fn named_block<'a, T>(
        &'a self,
        kind: RegistryBlockKind,
        reference: &str,
        blocks: &'a BTreeMap<String, T>,
    ) -> Option<&'a T> {
        blocks.get(reference).or_else(|| {
            self.name_ids
                .get(&kind)
                .and_then(|names| names.get(&normalize_string(reference)))
                .and_then(|id| blocks.get(id))
        })
    }

    fn insert(
        &mut self,
        block: RegistryBlock,
        name_ids: &mut BTreeMap<RegistryBlockKind, BTreeMap<String, String>>,
    ) -> Result<(), RegistryError> {
        validate_registry_block_semantics(&block)?;
        match block {
            RegistryBlock::Tool(block) => insert_named_block(
                RegistryBlockKind::Tool,
                block.identity.clone(),
                &mut self.tools,
                name_ids,
                block,
            ),
            RegistryBlock::Instruction(block) => insert_named_block(
                RegistryBlockKind::Instruction,
                block.identity.clone(),
                &mut self.instructions,
                name_ids,
                block,
            ),
            RegistryBlock::Phase(block) => insert_named_block(
                RegistryBlockKind::Phase,
                block.identity.clone(),
                &mut self.phases,
                name_ids,
                block,
            ),
            RegistryBlock::Flow(block) => insert_named_block(
                RegistryBlockKind::Flow,
                block.identity.clone(),
                &mut self.flows,
                name_ids,
                block,
            ),
        }
    }

    fn validate_references(&self) -> Result<(), RegistryError> {
        for phase in self.phases.values() {
            let mut instruction_ids = BTreeSet::new();
            for reference in &phase.instruction_refs {
                let instruction = self.require_instruction(
                    reference,
                    RegistryBlockKind::Phase,
                    &phase.identity.id,
                )?;
                if !instruction_ids.insert(instruction.identity.id.as_str()) {
                    return Err(RegistryError::DuplicateId {
                        kind: "phase instruction reference",
                        id: format!("{}.{}", phase.identity.id, instruction.identity.id),
                    });
                }
            }
            let mut tool_ids = BTreeSet::new();
            for reference in &phase.tool_refs {
                let tool =
                    self.require_tool(reference, RegistryBlockKind::Phase, &phase.identity.id)?;
                if !tool_ids.insert(tool.identity.id.as_str()) {
                    return Err(RegistryError::DuplicateId {
                        kind: "phase tool reference",
                        id: format!("{}.{}", phase.identity.id, tool.identity.id),
                    });
                }
            }
            let child_positions = self.phase_positions(
                &phase.phase_refs,
                RegistryBlockKind::Phase,
                &phase.identity.id,
            )?;
            if let Some(result_from) = &phase.result_from {
                let result_phase =
                    self.require_phase(result_from, RegistryBlockKind::Phase, &phase.identity.id)?;
                if !child_positions.contains_key(&result_phase.identity.id) {
                    return Err(RegistryError::MissingReference {
                        from_kind: RegistryBlockKind::Phase.as_str(),
                        from_id: phase.identity.id.clone(),
                        reference_kind: "direct child Phase result",
                        reference: result_from.clone(),
                    });
                }
                if phase.output != result_phase.output {
                    return Err(SemanticValidationError::InvalidPhaseDefinition {
                        phase_id: phase.identity.id.clone(),
                        message: format!(
                            "output contract must exactly match result_from Phase {}",
                            result_phase.identity.id
                        ),
                    }
                    .into());
                }
            }
            self.validate_transitions(
                &phase.transitions,
                &child_positions,
                RegistryBlockKind::Phase,
                &phase.identity.id,
            )?;
        }

        for flow_block in self.flows.values() {
            let phase_positions = self.phase_positions(
                &flow_block.phase_refs,
                RegistryBlockKind::Flow,
                &flow_block.identity.id,
            )?;
            for reference in &flow_block.subflow_refs {
                self.require_flow(reference, RegistryBlockKind::Flow, &flow_block.identity.id)?;
            }
            self.validate_transitions(
                &flow_block.transitions,
                &phase_positions,
                RegistryBlockKind::Flow,
                &flow_block.identity.id,
            )?;
        }

        self.validate_phase_cycles()?;
        self.validate_flow_cycles()
    }

    fn phase_positions(
        &self,
        references: &[String],
        from_kind: RegistryBlockKind,
        from_id: &str,
    ) -> Result<BTreeMap<String, usize>, RegistryError> {
        let mut positions = BTreeMap::new();
        for (index, reference) in references.iter().enumerate() {
            let phase = self.require_phase(reference, from_kind, from_id)?;
            if positions.insert(phase.identity.id.clone(), index).is_some() {
                return Err(RegistryError::DuplicateId {
                    kind: "child phase reference",
                    id: format!("{from_id}.{}", phase.identity.id),
                });
            }
        }
        Ok(positions)
    }

    fn validate_transitions(
        &self,
        transitions: &[PhaseTransition],
        positions: &BTreeMap<String, usize>,
        from_kind: RegistryBlockKind,
        from_id: &str,
    ) -> Result<(), RegistryError> {
        for transition in transitions {
            let source = self.require_phase(&transition.from_phase_ref, from_kind, from_id)?;
            let target = self.require_phase(&transition.to_phase_ref, from_kind, from_id)?;
            let Some(source_index) = positions.get(&source.identity.id) else {
                return Err(RegistryError::MissingReference {
                    from_kind: from_kind.as_str(),
                    from_id: from_id.to_owned(),
                    reference_kind: "Transition source child Phase",
                    reference: transition.from_phase_ref.clone(),
                });
            };
            let Some(target_index) = positions.get(&target.identity.id) else {
                return Err(RegistryError::MissingReference {
                    from_kind: from_kind.as_str(),
                    from_id: from_id.to_owned(),
                    reference_kind: "Transition target child Phase",
                    reference: transition.to_phase_ref.clone(),
                });
            };
            if target_index <= source_index {
                return Err(RegistryError::InvalidTransition {
                    owner_kind: from_kind.as_str(),
                    owner_id: from_id.to_owned(),
                    from_phase_id: source.identity.id.clone(),
                    to_phase_id: target.identity.id.clone(),
                });
            }
            validate_predicate_against_contract(&transition.when, &source.output).map_err(
                |error| match from_kind {
                    RegistryBlockKind::Phase => RegistryError::Semantic(
                        SemanticValidationError::InvalidPhaseDefinition {
                            phase_id: from_id.to_owned(),
                            message: format!(
                                "Transition predicate for source Phase {} does not match its output contract: {error}",
                                source.identity.id
                            ),
                        },
                    ),
                    _ => RegistryError::Semantic(
                        SemanticValidationError::InvalidFlowDefinition {
                            flow_id: from_id.to_owned(),
                            message: format!(
                                "Transition predicate for source Phase {} does not match its output contract: {error}",
                                source.identity.id
                            ),
                        },
                    ),
                },
            )?;
        }
        Ok(())
    }

    fn require_tool(
        &self,
        reference: &str,
        from_kind: RegistryBlockKind,
        from_id: &str,
    ) -> Result<&ToolBlock, RegistryError> {
        self.tool_block(reference)
            .ok_or_else(|| RegistryError::MissingReference {
                from_kind: from_kind.as_str(),
                from_id: from_id.to_owned(),
                reference_kind: RegistryBlockKind::Tool.as_str(),
                reference: reference.to_owned(),
            })
    }

    fn require_instruction(
        &self,
        reference: &str,
        from_kind: RegistryBlockKind,
        from_id: &str,
    ) -> Result<&InstructionBlock, RegistryError> {
        self.instruction_block(reference)
            .ok_or_else(|| RegistryError::MissingReference {
                from_kind: from_kind.as_str(),
                from_id: from_id.to_owned(),
                reference_kind: RegistryBlockKind::Instruction.as_str(),
                reference: reference.to_owned(),
            })
    }

    fn require_phase(
        &self,
        reference: &str,
        from_kind: RegistryBlockKind,
        from_id: &str,
    ) -> Result<&PhaseBlock, RegistryError> {
        self.phase_block(reference)
            .ok_or_else(|| RegistryError::MissingReference {
                from_kind: from_kind.as_str(),
                from_id: from_id.to_owned(),
                reference_kind: RegistryBlockKind::Phase.as_str(),
                reference: reference.to_owned(),
            })
    }

    fn require_flow(
        &self,
        reference: &str,
        from_kind: RegistryBlockKind,
        from_id: &str,
    ) -> Result<&FlowBlock, RegistryError> {
        self.flow_block(reference)
            .ok_or_else(|| RegistryError::MissingReference {
                from_kind: from_kind.as_str(),
                from_id: from_id.to_owned(),
                reference_kind: RegistryBlockKind::Flow.as_str(),
                reference: reference.to_owned(),
            })
    }

    fn validate_phase_cycles(&self) -> Result<(), RegistryError> {
        let mut visited = BTreeMap::<String, PhaseTailDepth>::new();
        for phase_id in self.phases.keys() {
            let mut visiting = BTreeSet::new();
            self.visit_phase(phase_id, 1, &mut visiting, &mut visited)?;
        }
        Ok(())
    }

    fn visit_phase(
        &self,
        phase_id: &str,
        depth: usize,
        visiting: &mut BTreeSet<String>,
        visited: &mut BTreeMap<String, PhaseTailDepth>,
    ) -> Result<(), RegistryError> {
        if depth > MAX_PHASE_NESTING_DEPTH {
            return Err(RegistryError::PhaseDepthExceeded {
                phase_id: phase_id.to_owned(),
                depth,
                max: MAX_PHASE_NESTING_DEPTH,
            });
        }
        if let Some(tail) = visited.get(phase_id) {
            let resolved_depth = depth + tail.depth - 1;
            if resolved_depth > MAX_PHASE_NESTING_DEPTH {
                return Err(RegistryError::PhaseDepthExceeded {
                    phase_id: tail.deepest_phase_id.clone(),
                    depth: resolved_depth,
                    max: MAX_PHASE_NESTING_DEPTH,
                });
            }
            return Ok(());
        }
        if !visiting.insert(phase_id.to_owned()) {
            return Err(RegistryError::PhaseCycle {
                phase_id: phase_id.to_owned(),
            });
        }

        let phase = self.require_phase(phase_id, RegistryBlockKind::Phase, phase_id)?;
        if phase.phase_refs.len() > MAX_PHASE_FANOUT {
            return Err(RegistryError::PhaseFanoutExceeded {
                phase_id: phase_id.to_owned(),
                count: phase.phase_refs.len(),
                max: MAX_PHASE_FANOUT,
            });
        }
        let mut tail = PhaseTailDepth {
            deepest_phase_id: phase_id.to_owned(),
            depth: 1,
        };
        for child_ref in &phase.phase_refs {
            let child = self.require_phase(child_ref, RegistryBlockKind::Phase, phase_id)?;
            self.visit_phase(&child.identity.id, depth + 1, visiting, visited)?;
            let child_tail = visited
                .get(&child.identity.id)
                .expect("visited child phase has tail depth");
            if child_tail.depth + 1 > tail.depth {
                tail = PhaseTailDepth {
                    deepest_phase_id: child_tail.deepest_phase_id.clone(),
                    depth: child_tail.depth + 1,
                };
            }
        }

        visiting.remove(phase_id);
        visited.insert(phase_id.to_owned(), tail);
        Ok(())
    }

    fn validate_flow_cycles(&self) -> Result<(), RegistryError> {
        // WHY: keep the visited cache for the whole registry validation pass so duplicated
        // subflow tails are validated once without changing duplicate execution semantics.
        let mut visited = BTreeMap::<String, FlowTailDepth>::new();
        for flow_id in self.flows.keys() {
            let mut visiting = BTreeSet::new();
            self.visit_flow(flow_id, 1, &mut visiting, &mut visited)?;
        }
        Ok(())
    }

    fn visit_flow(
        &self,
        flow_id: &str,
        depth: usize,
        visiting: &mut BTreeSet<String>,
        visited: &mut BTreeMap<String, FlowTailDepth>,
    ) -> Result<(), RegistryError> {
        if depth > MAX_FLOW_NESTING_DEPTH {
            return Err(RegistryError::FlowDepthExceeded {
                flow_id: flow_id.to_owned(),
                depth,
                max: MAX_FLOW_NESTING_DEPTH,
            });
        }
        if let Some(tail) = visited.get(flow_id) {
            let resolved_depth = depth + tail.depth - 1;
            if resolved_depth > MAX_FLOW_NESTING_DEPTH {
                return Err(RegistryError::FlowDepthExceeded {
                    flow_id: tail.deepest_flow_id.clone(),
                    depth: resolved_depth,
                    max: MAX_FLOW_NESTING_DEPTH,
                });
            }
            return Ok(());
        }
        if !visiting.insert(flow_id.to_owned()) {
            return Err(RegistryError::FlowCycle {
                flow_id: flow_id.to_owned(),
            });
        }

        let flow_block = self.require_flow(flow_id, RegistryBlockKind::Flow, flow_id)?;
        if flow_block.subflow_refs.len() > MAX_FLOW_FANOUT {
            return Err(RegistryError::FlowFanoutExceeded {
                flow_id: flow_id.to_owned(),
                count: flow_block.subflow_refs.len(),
                max: MAX_FLOW_FANOUT,
            });
        }
        let mut tail = FlowTailDepth {
            deepest_flow_id: flow_id.to_owned(),
            depth: 1,
        };
        for subflow_ref in &flow_block.subflow_refs {
            let subflow = self.require_flow(subflow_ref, RegistryBlockKind::Flow, flow_id)?;
            self.visit_flow(&subflow.identity.id, depth + 1, visiting, visited)?;
            let child_tail = visited
                .get(&subflow.identity.id)
                .expect("visited child flow has tail depth");
            if child_tail.depth + 1 > tail.depth {
                tail = FlowTailDepth {
                    deepest_flow_id: child_tail.deepest_flow_id.clone(),
                    depth: child_tail.depth + 1,
                };
            }
        }

        visiting.remove(flow_id);
        visited.insert(flow_id.to_owned(), tail);
        Ok(())
    }
}

struct FlowTailDepth {
    deepest_flow_id: String,
    depth: usize,
}

struct PhaseTailDepth {
    deepest_phase_id: String,
    depth: usize,
}
