use crate::interrupt::InterruptCoordinator;
use flow_agent_core::{EmitMode, RuntimeError};
use std::{
    io::{self, BufRead},
    path::PathBuf,
    process::ExitCode,
};

use super::{execution::command_exit_code, run::run_command};

pub(super) const CHAT_REFERENCE_REQUIRED: &str =
    "flow chat requires one nonblank stdin Flow reference";
const MAX_CHAT_REFERENCE_CHARS: usize = core_script::MAX_BLOCK_NAME_CHARS + 1;
const MAX_CHAT_REFERENCE_BYTES: usize = MAX_CHAT_REFERENCE_CHARS * 4;

pub(super) fn chat(
    workspace: PathBuf,
    interrupts: &InterruptCoordinator,
) -> Result<ExitCode, RuntimeError> {
    let flow_reference = read_chat_reference(&mut io::stdin().lock())?;
    let output = run_command(
        &workspace,
        &flow_reference,
        EmitMode::Jsonl,
        None,
        interrupts,
    )?;
    Ok(command_exit_code(output.failed))
}

pub(super) fn read_chat_reference(reader: &mut impl BufRead) -> Result<String, RuntimeError> {
    let mut reference = String::with_capacity(MAX_CHAT_REFERENCE_BYTES);
    let mut allowed_chars = core_script::MAX_BLOCK_NAME_CHARS;
    let mut retained_chars = 0usize;
    let mut last_content_byte = 0usize;
    let mut trailing_whitespace_overflowed = false;
    loop {
        let next = read_stdin_char(reader)?;
        let end_of_input = next.is_none();
        if matches!(next, None | Some('\n')) {
            reference.truncate(last_content_byte);
            if !reference.is_empty() {
                break;
            }
            if end_of_input {
                return Err(RuntimeError::Usage(CHAT_REFERENCE_REQUIRED.to_owned()));
            }
            allowed_chars = core_script::MAX_BLOCK_NAME_CHARS;
            retained_chars = 0;
            trailing_whitespace_overflowed = false;
            continue;
        }

        let value = next.expect("line terminator handled above");
        if value.is_whitespace() {
            if reference.is_empty() {
                continue;
            }
            if retained_chars < allowed_chars {
                reference.push(value);
                retained_chars += 1;
            } else {
                trailing_whitespace_overflowed = true;
            }
            continue;
        }

        if reference.is_empty() && value == '/' {
            allowed_chars = MAX_CHAT_REFERENCE_CHARS;
        }
        if trailing_whitespace_overflowed || retained_chars >= allowed_chars {
            return Err(RuntimeError::Usage(CHAT_REFERENCE_REQUIRED.to_owned()));
        }
        reference.push(value);
        retained_chars += 1;
        last_content_byte = reference.len();
    }

    if reference.starts_with('/') {
        reference.remove(0);
    }
    if reference.is_empty() {
        return Err(RuntimeError::Usage(CHAT_REFERENCE_REQUIRED.to_owned()));
    }
    Ok(reference)
}

fn read_stdin_char(reader: &mut impl BufRead) -> Result<Option<char>, RuntimeError> {
    let Some(first) = read_stdin_byte(reader)? else {
        return Ok(None);
    };
    let width = match first {
        0x00..=0x7f => 1,
        0xc2..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf4 => 4,
        _ => return Err(invalid_stdin_utf8()),
    };
    let mut encoded = [0u8; 4];
    encoded[0] = first;
    for byte in &mut encoded[1..width] {
        *byte = read_stdin_byte(reader)?.ok_or_else(invalid_stdin_utf8)?;
    }
    let value = std::str::from_utf8(&encoded[..width]).map_err(|_| invalid_stdin_utf8())?;
    Ok(value.chars().next())
}

fn read_stdin_byte(reader: &mut impl BufRead) -> Result<Option<u8>, RuntimeError> {
    let mut byte = [0u8; 1];
    loop {
        match reader.read(&mut byte) {
            Ok(0) => return Ok(None),
            Ok(1) => return Ok(Some(byte[0])),
            Ok(_) => unreachable!("one-byte buffer cannot read more than one byte"),
            Err(source) if source.kind() == io::ErrorKind::Interrupted => {}
            Err(source) => {
                return Err(RuntimeError::Io {
                    path: PathBuf::from("<stdin>"),
                    source,
                });
            }
        }
    }
}

fn invalid_stdin_utf8() -> RuntimeError {
    RuntimeError::Io {
        path: PathBuf::from("<stdin>"),
        source: io::Error::new(
            io::ErrorKind::InvalidData,
            "stream did not contain valid UTF-8",
        ),
    }
}
