impl ResolvedRegistry {
    fn load_for_flow_with_limits(
        workspace: &Path,
        registry_root: &Path,
        flow_reference: &str,
        max_file_bytes: u64,
        max_total_bytes: u64,
        max_active_bytes: u64,
    ) -> Result<Self, RegistryError> {
        Self::load_for_flow_with_all_limits(
            workspace,
            registry_root,
            flow_reference,
            max_active_bytes,
            RegistryTraversalLimits {
                max_file_bytes,
                max_total_bytes,
                max_entries: MAX_REGISTRY_ENTRIES,
                max_depth: MAX_REGISTRY_TRAVERSAL_DEPTH,
            },
        )
    }

    fn load_for_flow_with_all_limits(
        workspace: &Path,
        registry_root: &Path,
        flow_reference: &str,
        max_active_bytes: u64,
        limits: RegistryTraversalLimits,
    ) -> Result<Self, RegistryError> {
        let root = open_registry_root(workspace, registry_root)?;
        let mut paths = Vec::new();
        let mut state = RegistryTraversalState::default();
        collect_registry_files_with_limits(
            &root,
            &root.dir,
            Path::new(""),
            &mut paths,
            limits,
            0,
            &mut state,
        )?;
        paths.sort_by(|left, right| left.path.cmp(&right.path));
        let mut catalog = RegistryCatalog::default();
        let mut total_bytes = 0u64;

        for file in &paths {
            let source = read_registry_file_to_string(&root, file, limits.max_file_bytes)?;
            let bytes = u64::try_from(source.len()).unwrap_or(u64::MAX);
            total_bytes = total_bytes.saturating_add(bytes);
            if total_bytes > limits.max_total_bytes {
                return Err(RegistryError::ReadLimitExceeded {
                    path: root.path.clone(),
                    bytes: total_bytes,
                    max: limits.max_total_bytes,
                });
            }
            let source_name = file.path.to_string_lossy().replace('\\', "/");
            let block = parse_registry_block(&source_name, &source)?;
            catalog.insert(&block, file.clone())?;
        }

        let root_flow = catalog.require("flow", flow_reference, "registry", "root")?;
        let mut pending = vec![(root_flow.kind, root_flow.identity.id.clone())];
        let mut loaded = BTreeSet::new();
        let mut active_bytes = 0u64;
        let mut blocks = Vec::new();

        while let Some((kind, id)) = pending.pop() {
            if !loaded.insert((kind, id.clone())) {
                continue;
            }
            let entry = catalog
                .resolve(kind, &id)
                .expect("queued catalog entries remain available");
            let source = read_registry_file_to_string(&root, &entry.file, limits.max_file_bytes)?;
            active_bytes =
                active_bytes.saturating_add(u64::try_from(source.len()).unwrap_or(u64::MAX));
            if active_bytes > max_active_bytes {
                return Err(RegistryError::ReadLimitExceeded {
                    path: root.path.clone(),
                    bytes: active_bytes,
                    max: max_active_bytes,
                });
            }
            let source_name = entry.file.path.to_string_lossy().replace('\\', "/");
            let block = parse_registry_block(&source_name, &source)?;
            let (actual_kind, actual_identity) = registry_block_identity(&block);
            if actual_kind != entry.kind || actual_identity != &entry.identity {
                return Err(parse_error(
                    &source_name,
                    "registry block identity changed while loading".to_owned(),
                ));
            }
            enqueue_dependencies(&catalog, &block, &mut pending)?;
            blocks.push(block);
        }

        Self::from_blocks(blocks)
    }

    /// Resolves a registry from already parsed blocks.
    pub fn from_blocks(
        blocks: impl IntoIterator<Item = RegistryBlock>,
    ) -> Result<Self, RegistryError> {
        let mut registry = Self {
            connections: BTreeMap::new(),
            instructions: BTreeMap::new(),
            flows: BTreeMap::new(),
            phases: BTreeMap::new(),
            tools: BTreeMap::new(),
            name_ids: BTreeMap::new(),
        };
        let mut name_ids: BTreeMap<&'static str, BTreeMap<String, String>> = BTreeMap::new();

        for block in blocks {
            validate_registry_block_shape(&block).map_err(|message| {
                let (kind, identity) = registry_block_identity(&block);
                if !is_valid_block_id(&identity.id) {
                    RegistryError::InvalidBlockId(identity.id.clone())
                } else if identity.name.is_empty() {
                    RegistryError::InvalidBlockName {
                        kind,
                        id: identity.id.clone(),
                    }
                } else if let RegistryBlock::Phase(phase) = &block
                    && let Some(step) = phase.steps.iter().find(|step| !is_valid_block_id(&step.id))
                {
                    RegistryError::InvalidBlockId(step.id.clone())
                } else {
                    parse_error("programmatic registry", message)
                }
            })?;
            let block = canonicalize_registry_block(block)?;
            registry.insert(block, &mut name_ids)?;
        }
        registry.name_ids = name_ids;

        registry.validate_references()?;
        registry.with_canonical_references()
    }

    /// Serializes the resolved registry as canonical JSON without a trailing newline.
    pub fn canonical_json(&self) -> Result<String, RegistryError> {
        let mut value = serde_json::to_value(self).map_err(RegistryError::Serialize)?;
        canonicalize_registry_value(&mut value);
        proto::canonical_json(&value).map_err(RegistryError::CanonicalJson)
    }

    fn with_canonical_references(&self) -> Result<Self, RegistryError> {
        let mut canonical = self.clone();
        for phase in canonical.phases.values_mut() {
            for reference in &mut phase.instruction_refs {
                *reference = self
                    .require_instruction(reference, "phase", &phase.identity.id)?
                    .identity
                    .id
                    .clone();
            }
            for reference in &mut phase.tool_refs {
                *reference = self
                    .require_tool(reference, "phase", &phase.identity.id)?
                    .identity
                    .id
                    .clone();
            }
            for step in &mut phase.steps {
                for reference in &mut step.connection_refs {
                    *reference = self
                        .require_connection(reference, "step", &step.id)?
                        .identity
                        .id
                        .clone();
                }
            }
        }
        for connection in canonical.connections.values_mut() {
            connection.from_ref =
                self.canonical_endpoint_reference(&connection.from_ref, &connection.identity.id)?;
            connection.to_ref =
                self.canonical_endpoint_reference(&connection.to_ref, &connection.identity.id)?;
        }
        for flow_block in canonical.flows.values_mut() {
            for reference in &mut flow_block.phase_refs {
                *reference = self
                    .require_phase(reference, "flow", &flow_block.identity.id)?
                    .identity
                    .id
                    .clone();
            }
            for reference in &mut flow_block.subflow_refs {
                *reference = self
                    .require_flow(reference, "flow", &flow_block.identity.id)?
                    .identity
                    .id
                    .clone();
            }
            for reference in &mut flow_block.connection_refs {
                *reference = self
                    .require_connection(reference, "flow", &flow_block.identity.id)?
                    .identity
                    .id
                    .clone();
            }
        }
        Ok(canonical)
    }

    fn canonical_endpoint_reference(
        &self,
        reference: &str,
        connection_id: &str,
    ) -> Result<String, RegistryError> {
        self.require_endpoint(reference, connection_id)?;
        let direct_target = self
            .tool_block(reference)
            .map(|block| &block.identity)
            .or_else(|| {
                self.instruction_block(reference)
                    .map(|block| &block.identity)
            })
            .or_else(|| self.phase_block(reference).map(|block| &block.identity))
            .or_else(|| self.flow_block(reference).map(|block| &block.identity));
        if let Some(identity) = direct_target {
            return Ok(if self.direct_endpoint_match_count(&identity.id) == 1 {
                identity.id.clone()
            } else {
                normalize_string(&identity.name)
            });
        }

        let (phase_ref, step_id) = reference
            .split_once('.')
            .expect("validated step endpoint contains a phase and step");
        let phase = self.require_phase(phase_ref, "connection", connection_id)?;
        let by_id = format!("{}.{step_id}", phase.identity.id);
        Ok(if self.direct_endpoint_match_count(&by_id) == 0 {
            by_id
        } else {
            format!("{}.{step_id}", normalize_string(&phase.identity.name))
        })
    }

    /// Resolves a flow by id or unambiguous name.
    pub fn flow_block(&self, reference: &str) -> Option<&FlowBlock> {
        self.named_block("flow", reference, &self.flows)
    }

    /// Resolves a phase by id or unambiguous name.
    pub fn phase_block(&self, reference: &str) -> Option<&PhaseBlock> {
        self.named_block("phase", reference, &self.phases)
    }

    /// Resolves a tool by id or unambiguous name.
    pub fn tool_block(&self, reference: &str) -> Option<&ToolBlock> {
        self.named_block("tool", reference, &self.tools)
    }

    /// Returns tool blocks in canonical id order.
    pub fn tool_blocks(&self) -> impl Iterator<Item = &ToolBlock> {
        self.tools.values()
    }

    /// Resolves an instruction by id or unambiguous name.
    pub fn instruction_block(&self, reference: &str) -> Option<&InstructionBlock> {
        self.named_block("instruction", reference, &self.instructions)
    }

    /// Resolves a connection by id or unambiguous name.
    pub fn connection_block(&self, reference: &str) -> Option<&ConnectionBlock> {
        self.named_block("connection", reference, &self.connections)
    }

    fn named_block<'a, T>(
        &'a self,
        kind: &'static str,
        reference: &str,
        blocks: &'a BTreeMap<String, T>,
    ) -> Option<&'a T> {
        blocks.get(reference).or_else(|| {
            self.name_ids
                .get(kind)
                .and_then(|names| names.get(&normalize_string(reference)))
                .and_then(|id| blocks.get(id))
        })
    }

    fn insert(
        &mut self,
        block: RegistryBlock,
        name_ids: &mut BTreeMap<&'static str, BTreeMap<String, String>>,
    ) -> Result<(), RegistryError> {
        validate_registry_block_semantics(&block)?;
        match block {
            RegistryBlock::Tool(block) => insert_named_block(
                "tool",
                block.identity.clone(),
                &mut self.tools,
                name_ids,
                block,
            ),
            RegistryBlock::Instruction(block) => insert_named_block(
                "instruction",
                block.identity.clone(),
                &mut self.instructions,
                name_ids,
                block,
            ),
            RegistryBlock::Phase(block) => insert_named_block(
                "phase",
                block.identity.clone(),
                &mut self.phases,
                name_ids,
                block,
            ),
            RegistryBlock::Connection(block) => insert_named_block(
                "connection",
                block.identity.clone(),
                &mut self.connections,
                name_ids,
                block,
            ),
            RegistryBlock::Flow(block) => insert_named_block(
                "flow",
                block.identity.clone(),
                &mut self.flows,
                name_ids,
                block,
            ),
        }
    }

    fn validate_references(&self) -> Result<(), RegistryError> {
        for phase in self.phases.values() {
            for reference in &phase.instruction_refs {
                self.require_instruction(reference, "phase", &phase.identity.id)?;
            }
            let mut tool_ids = BTreeSet::new();
            for reference in &phase.tool_refs {
                let tool = self.require_tool(reference, "phase", &phase.identity.id)?;
                if !tool_ids.insert(tool.identity.id.as_str()) {
                    return Err(RegistryError::DuplicateId {
                        kind: "phase tool reference",
                        id: format!("{}.{}", phase.identity.id, tool.identity.id),
                    });
                }
            }
            let mut step_ids = BTreeSet::new();
            for step in &phase.steps {
                if !is_valid_block_id(&step.id) {
                    return Err(RegistryError::InvalidBlockId(step.id.clone()));
                }
                if !step_ids.insert(step.id.as_str()) {
                    return Err(RegistryError::DuplicateId {
                        kind: "step",
                        id: format!("{}.{}", phase.identity.id, step.id),
                    });
                }
            }
        }

        for connection in self.connections.values() {
            self.require_endpoint(&connection.from_ref, &connection.identity.id)?;
            self.require_endpoint(&connection.to_ref, &connection.identity.id)?;
        }

        for flow_block in self.flows.values() {
            for reference in &flow_block.phase_refs {
                self.require_phase(reference, "flow", &flow_block.identity.id)?;
            }
            for reference in &flow_block.subflow_refs {
                self.require_flow(reference, "flow", &flow_block.identity.id)?;
            }
            let flow_connection_ids = flow_block
                .connection_refs
                .iter()
                .map(|reference| {
                    self.require_connection(reference, "flow", &flow_block.identity.id)
                        .map(|connection| connection.identity.id.as_str())
                })
                .collect::<Result<BTreeSet<_>, RegistryError>>()?;

            for phase_ref in &flow_block.phase_refs {
                let phase = self.require_phase(phase_ref, "flow", &flow_block.identity.id)?;
                for step in &phase.steps {
                    for connection_ref in &step.connection_refs {
                        let connection =
                            self.require_connection(connection_ref, "step", &step.id)?;
                        if !flow_connection_ids.contains(connection.identity.id.as_str()) {
                            return Err(RegistryError::MissingReference {
                                from_kind: "flow",
                                from_id: flow_block.identity.id.clone(),
                                reference_kind: "step connection",
                                reference: connection_ref.clone(),
                            });
                        }
                    }
                }
            }
        }

        self.validate_flow_cycles()
    }

    fn require_tool(
        &self,
        reference: &str,
        from_kind: &'static str,
        from_id: &str,
    ) -> Result<&ToolBlock, RegistryError> {
        self.tool_block(reference)
            .ok_or_else(|| RegistryError::MissingReference {
                from_kind,
                from_id: from_id.to_owned(),
                reference_kind: "tool",
                reference: reference.to_owned(),
            })
    }

    fn require_instruction(
        &self,
        reference: &str,
        from_kind: &'static str,
        from_id: &str,
    ) -> Result<&InstructionBlock, RegistryError> {
        self.instruction_block(reference)
            .ok_or_else(|| RegistryError::MissingReference {
                from_kind,
                from_id: from_id.to_owned(),
                reference_kind: "instruction",
                reference: reference.to_owned(),
            })
    }

    fn require_phase(
        &self,
        reference: &str,
        from_kind: &'static str,
        from_id: &str,
    ) -> Result<&PhaseBlock, RegistryError> {
        self.phase_block(reference)
            .ok_or_else(|| RegistryError::MissingReference {
                from_kind,
                from_id: from_id.to_owned(),
                reference_kind: "phase",
                reference: reference.to_owned(),
            })
    }

    fn require_flow(
        &self,
        reference: &str,
        from_kind: &'static str,
        from_id: &str,
    ) -> Result<&FlowBlock, RegistryError> {
        self.flow_block(reference)
            .ok_or_else(|| RegistryError::MissingReference {
                from_kind,
                from_id: from_id.to_owned(),
                reference_kind: "flow",
                reference: reference.to_owned(),
            })
    }

    fn require_connection(
        &self,
        reference: &str,
        from_kind: &'static str,
        from_id: &str,
    ) -> Result<&ConnectionBlock, RegistryError> {
        self.connection_block(reference)
            .ok_or_else(|| RegistryError::MissingReference {
                from_kind,
                from_id: from_id.to_owned(),
                reference_kind: "connection",
                reference: reference.to_owned(),
            })
    }

    fn require_endpoint(&self, reference: &str, connection_id: &str) -> Result<(), RegistryError> {
        let matches = self.direct_endpoint_match_count(reference);
        match matches {
            1 => Ok(()),
            0 => Err(RegistryError::MissingReference {
                from_kind: "connection",
                from_id: connection_id.to_owned(),
                reference_kind: "endpoint",
                reference: reference.to_owned(),
            }),
            _ => Err(RegistryError::AmbiguousReference {
                kind: "endpoint",
                reference: reference.to_owned(),
            }),
        }
        .or_else(|err| {
            if !matches!(err, RegistryError::MissingReference { .. }) {
                return Err(err);
            }
            let Some((phase_ref, step_id)) = reference.split_once('.') else {
                return Err(err);
            };
            let phase = self.require_phase(phase_ref, "connection", connection_id)?;
            if phase.steps.iter().any(|step| step.id == step_id) {
                return Ok(());
            }
            Err(RegistryError::MissingReference {
                from_kind: "connection",
                from_id: connection_id.to_owned(),
                reference_kind: "step",
                reference: reference.to_owned(),
            })
        })
    }

    fn direct_endpoint_match_count(&self, reference: &str) -> usize {
        [
            self.tool_block(reference).is_some(),
            self.instruction_block(reference).is_some(),
            self.phase_block(reference).is_some(),
            self.flow_block(reference).is_some(),
        ]
        .into_iter()
        .filter(|matched| *matched)
        .count()
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

        let flow_block = self.require_flow(flow_id, "flow", flow_id)?;
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
            let subflow = self.require_flow(subflow_ref, "flow", flow_id)?;
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
