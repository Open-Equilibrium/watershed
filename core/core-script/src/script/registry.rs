impl ResolvedRegistry {
    /// Loads and validates a registry root with M1 read caps.
    pub fn load(root: &Path) -> Result<Self, RegistryError> {
        Self::load_with_limits(root, MAX_REGISTRY_FILE_BYTES, MAX_REGISTRY_TOTAL_BYTES)
    }

    fn load_with_limits(
        root: &Path,
        max_file_bytes: u64,
        max_total_bytes: u64,
    ) -> Result<Self, RegistryError> {
        Self::load_with_all_limits(
            root,
            max_file_bytes,
            max_total_bytes,
            MAX_REGISTRY_FILES,
            MAX_REGISTRY_TRAVERSAL_DEPTH,
        )
    }

    fn load_with_all_limits(
        root: &Path,
        max_file_bytes: u64,
        max_total_bytes: u64,
        max_files: usize,
        max_depth: usize,
    ) -> Result<Self, RegistryError> {
        let mut paths = Vec::new();
        let limits = RegistryTraversalLimits {
            max_file_bytes,
            max_total_bytes,
            max_files,
            max_depth,
        };
        let mut state = RegistryTraversalState::default();
        collect_registry_files_with_limits(root, root, &mut paths, limits, 0, &mut state)?;
        paths.sort_by(|left, right| left.path.cmp(&right.path));
        let mut blocks = Vec::new();
        let mut total_bytes = 0u64;

        for file in paths {
            if file.bytes > max_file_bytes {
                return Err(RegistryError::ReadLimitExceeded {
                    path: file.path,
                    bytes: file.bytes,
                    max: max_file_bytes,
                });
            }
            let source = read_registry_file_to_string(&file, max_file_bytes)?;
            let bytes = u64::try_from(source.len()).unwrap_or(u64::MAX);
            total_bytes = total_bytes.saturating_add(bytes);
            if total_bytes > max_total_bytes {
                return Err(RegistryError::ReadLimitExceeded {
                    path: root.to_path_buf(),
                    bytes: total_bytes,
                    max: max_total_bytes,
                });
            }
            let source_name = file
                .path
                .strip_prefix(root)
                .unwrap_or(file.path.as_path())
                .to_string_lossy()
                .replace('\\', "/");
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
        };
        let mut names: BTreeMap<&'static str, BTreeSet<String>> = BTreeMap::new();

        for block in blocks {
            registry.insert(block, &mut names)?;
        }

        registry.validate_references()?;
        Ok(registry)
    }

    /// Serializes the resolved registry as canonical JSON without a trailing newline.
    pub fn canonical_json(&self) -> Result<String, RegistryError> {
        let mut out = canonical_resolved_registry_json(self)?;
        if out.ends_with('\n') {
            out.pop();
        }
        Ok(out)
    }

    /// Resolves a loop by id or unambiguous name.
    pub fn loop_block(&self, reference: &str) -> Option<&LoopBlock> {
        self.loops.get(reference).or_else(|| {
            self.loops
                .values()
                .find(|block| normalized_eq(&block.identity.name, reference))
        })
    }

    /// Resolves a phase by id or unambiguous name.
    pub fn phase_block(&self, reference: &str) -> Option<&PhaseBlock> {
        self.phases.get(reference).or_else(|| {
            self.phases
                .values()
                .find(|block| normalized_eq(&block.identity.name, reference))
        })
    }

    /// Resolves a tool by id or unambiguous name.
    pub fn tool_block(&self, reference: &str) -> Option<&ToolBlock> {
        self.tools.get(reference).or_else(|| {
            self.tools
                .values()
                .find(|block| normalized_eq(&block.identity.name, reference))
        })
    }

    /// Resolves an instruction by id or unambiguous name.
    pub fn instruction_block(&self, reference: &str) -> Option<&InstructionBlock> {
        self.instructions.get(reference).or_else(|| {
            self.instructions
                .values()
                .find(|block| normalized_eq(&block.identity.name, reference))
        })
    }

    /// Resolves a connection by id or unambiguous name.
    pub fn connection_block(&self, reference: &str) -> Option<&ConnectionBlock> {
        self.connections.get(reference).or_else(|| {
            self.connections
                .values()
                .find(|block| normalized_eq(&block.identity.name, reference))
        })
    }

    fn insert(
        &mut self,
        block: RegistryBlock,
        names: &mut BTreeMap<&'static str, BTreeSet<String>>,
    ) -> Result<(), RegistryError> {
        validate_registry_block_semantics(&block)?;
        match block {
            RegistryBlock::Tool(block) => insert_named_block(
                "tool",
                block.identity.clone(),
                &mut self.tools,
                names,
                block,
            ),
            RegistryBlock::Instruction(block) => insert_named_block(
                "instruction",
                block.identity.clone(),
                &mut self.instructions,
                names,
                block,
            ),
            RegistryBlock::Phase(block) => insert_named_block(
                "phase",
                block.identity.clone(),
                &mut self.phases,
                names,
                block,
            ),
            RegistryBlock::Connection(block) => insert_named_block(
                "connection",
                block.identity.clone(),
                &mut self.connections,
                names,
                block,
            ),
            RegistryBlock::Loop(block) => insert_named_block(
                "loop",
                block.identity.clone(),
                &mut self.loops,
                names,
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
            for reference in &loop_block.connection_refs {
                self.require_connection(reference, "loop", &loop_block.identity.id)?;
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
        let matches = [
            self.tool_block(reference).is_some(),
            self.instruction_block(reference).is_some(),
            self.phase_block(reference).is_some(),
            self.loop_block(reference).is_some(),
        ]
        .into_iter()
        .filter(|matched| *matched)
        .count();
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

