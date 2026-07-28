use core_policy::ProtectedPathMatchMode;
use proto::{EventEnvelope, EventType};
use std::{
    collections::BTreeSet,
    fs,
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Barrier, Condvar, Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

#[path = "../../tests/support.rs"]
mod test_support;
use crate::runtime::{
    apply::*, config_io::*, context::*, context_persistence::*, event_construction::*,
    event_writer::*, failures::*, fixture_effects::*, fixture_tools::*, fs_guards::*,
    live_events::*, planning::*, resume::*, session::*, session_authority::*, session_bundle::*,
    session_lock::*, session_reading::*, session_reservation::*, tail::*, types::*, validate::*,
};
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
mod protocol_lifecycle;
mod protocol_payload;
mod registry_runtime;
mod sandbox;
mod session_bundle;
mod session_corruption;
mod session_listing;
mod session_reservation;
mod session_resume;
mod surface_contracts;
mod tail;
mod workspace_security;
