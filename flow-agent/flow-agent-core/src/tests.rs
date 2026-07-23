use core_policy::ProtectedPathMatchMode;
use proto::{EventEnvelope, EventType};
use std::{
    collections::BTreeSet,
    fs,
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Barrier, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

#[path = "../../tests/support.rs"]
mod test_support;
use crate::runtime::*;
use test_support::{
    PeakRssSampler, TempWorkspace, copy_dir, current_resident_set_size, expected_stream,
    fixture_dir, workspace_copy,
};

mod support;
use support::*;

mod helpers;
use helpers::*;

mod context;
mod event_writer;
mod fs_guards;
mod performance;
mod protocol;
mod registry_runtime;
mod sandbox;
mod session_listing;
mod session_logs;
mod surface_contracts;
mod tail;
mod workspace_security;
