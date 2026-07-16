impl ResolvedRegistry {
    fn load_with_limits(
        workspace: &Path,
        registry_root: &Path,
        max_file_bytes: u64,
        max_total_bytes: u64,
    ) -> Result<Self, RegistryError> {
        Self::load_with_all_limits(
            workspace,
            registry_root,
            max_file_bytes,
            max_total_bytes,
            MAX_REGISTRY_ENTRIES,
            MAX_REGISTRY_TRAVERSAL_DEPTH,
        )
    }

    fn load_with_all_limits(
        workspace: &Path,
        registry_root: &Path,
        max_file_bytes: u64,
        max_total_bytes: u64,
        max_entries: usize,
        max_depth: usize,
    ) -> Result<Self, RegistryError> {
        let root = open_registry_root(workspace, registry_root)?;
        let mut paths = Vec::new();
        let limits = RegistryTraversalLimits {
            max_file_bytes,
            max_total_bytes,
            max_entries,
            max_depth,
        };
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
        let mut blocks = Vec::new();
        let mut total_bytes = 0u64;

        for file in paths {
            let source = read_registry_file_to_string(&root, &file, max_file_bytes)?;
            let bytes = u64::try_from(source.len()).unwrap_or(u64::MAX);
            total_bytes = total_bytes.saturating_add(bytes);
            if total_bytes > max_total_bytes {
                return Err(RegistryError::ReadLimitExceeded {
                    path: root.path.clone(),
                    bytes: total_bytes,
                    max: max_total_bytes,
                });
            }
            let source_name = file.path.to_string_lossy().replace('\\', "/");
            let block = parse_registry_block(&source_name, &source)?;
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
            loops: BTreeMap::new(),
            phases: BTreeMap::new(),
            tools: BTreeMap::new(),
            name_ids: BTreeMap::new(),
        };
        let mut name_ids: BTreeMap<&'static str, BTreeMap<String, String>> = BTreeMap::new();

        for block in blocks {
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
        for loop_block in canonical.loops.values_mut() {
            for reference in &mut loop_block.phase_refs {
                *reference = self
                    .require_phase(reference, "loop", &loop_block.identity.id)?
                    .identity
                    .id
                    .clone();
            }
            for reference in &mut loop_block.subloop_refs {
                *reference = self
                    .require_loop(reference, "loop", &loop_block.identity.id)?
                    .identity
                    .id
                    .clone();
            }
            for reference in &mut loop_block.connection_refs {
                *reference = self
                    .require_connection(reference, "loop", &loop_block.identity.id)?
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
            .or_else(|| self.loop_block(reference).map(|block| &block.identity));
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

    /// Resolves a loop by id or unambiguous name.
    pub fn loop_block(&self, reference: &str) -> Option<&LoopBlock> {
        self.named_block("loop", reference, &self.loops)
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
            RegistryBlock::Loop(block) => insert_named_block(
                "loop",
                block.identity.clone(),
                &mut self.loops,
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

        for loop_block in self.loops.values() {
            for reference in &loop_block.phase_refs {
                self.require_phase(reference, "loop", &loop_block.identity.id)?;
            }
            for reference in &loop_block.subloop_refs {
                self.require_loop(reference, "loop", &loop_block.identity.id)?;
            }
            let loop_connection_ids = loop_block
                .connection_refs
                .iter()
                .map(|reference| {
                    self.require_connection(reference, "loop", &loop_block.identity.id)
                        .map(|connection| connection.identity.id.as_str())
                })
                .collect::<Result<BTreeSet<_>, RegistryError>>()?;

            for phase_ref in &loop_block.phase_refs {
                let phase = self.require_phase(phase_ref, "loop", &loop_block.identity.id)?;
                for step in &phase.steps {
                    for connection_ref in &step.connection_refs {
                        let connection =
                            self.require_connection(connection_ref, "step", &step.id)?;
                        if !loop_connection_ids.contains(connection.identity.id.as_str()) {
                            return Err(RegistryError::MissingReference {
                                from_kind: "loop",
                                from_id: loop_block.identity.id.clone(),
                                reference_kind: "step connection",
                                reference: connection_ref.clone(),
                            });
                        }
                    }
                }
            }
        }

        self.validate_loop_cycles()
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

    fn require_loop(
        &self,
        reference: &str,
        from_kind: &'static str,
        from_id: &str,
    ) -> Result<&LoopBlock, RegistryError> {
        self.loop_block(reference)
            .ok_or_else(|| RegistryError::MissingReference {
                from_kind,
                from_id: from_id.to_owned(),
                reference_kind: "loop",
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
            self.loop_block(reference).is_some(),
        ]
        .into_iter()
        .filter(|matched| *matched)
        .count()
    }

    fn validate_loop_cycles(&self) -> Result<(), RegistryError> {
        // WHY: keep the visited cache for the whole registry validation pass so duplicated
        // subloop tails are validated once without changing duplicate execution semantics.
        let mut visited = BTreeMap::<String, LoopTailDepth>::new();
        for loop_id in self.loops.keys() {
            let mut visiting = BTreeSet::new();
            self.visit_loop(loop_id, 1, &mut visiting, &mut visited)?;
        }
        Ok(())
    }

    fn visit_loop(
        &self,
        loop_id: &str,
        depth: usize,
        visiting: &mut BTreeSet<String>,
        visited: &mut BTreeMap<String, LoopTailDepth>,
    ) -> Result<(), RegistryError> {
        if depth > MAX_LOOP_NESTING_DEPTH {
            return Err(RegistryError::LoopDepthExceeded {
                loop_id: loop_id.to_owned(),
                depth,
                max: MAX_LOOP_NESTING_DEPTH,
            });
        }
        if let Some(tail) = visited.get(loop_id) {
            let resolved_depth = depth + tail.depth - 1;
            if resolved_depth > MAX_LOOP_NESTING_DEPTH {
                return Err(RegistryError::LoopDepthExceeded {
                    loop_id: tail.deepest_loop_id.clone(),
                    depth: resolved_depth,
                    max: MAX_LOOP_NESTING_DEPTH,
                });
            }
            return Ok(());
        }
        if !visiting.insert(loop_id.to_owned()) {
            return Err(RegistryError::LoopCycle {
                loop_id: loop_id.to_owned(),
            });
        }

        let loop_block = self.require_loop(loop_id, "loop", loop_id)?;
        let mut tail = LoopTailDepth {
            deepest_loop_id: loop_id.to_owned(),
            depth: 1,
        };
        for subloop_ref in &loop_block.subloop_refs {
            let subloop = self.require_loop(subloop_ref, "loop", loop_id)?;
            self.visit_loop(&subloop.identity.id, depth + 1, visiting, visited)?;
            let child_tail = visited
                .get(&subloop.identity.id)
                .expect("visited child loop has tail depth");
            if child_tail.depth + 1 > tail.depth {
                tail = LoopTailDepth {
                    deepest_loop_id: child_tail.deepest_loop_id.clone(),
                    depth: child_tail.depth + 1,
                };
            }
        }

        visiting.remove(loop_id);
        visited.insert(loop_id.to_owned(), tail);
        Ok(())
    }
}

struct LoopTailDepth {
    deepest_loop_id: String,
    depth: usize,
}
