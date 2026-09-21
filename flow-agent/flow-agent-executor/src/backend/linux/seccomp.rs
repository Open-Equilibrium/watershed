use std::{
    fs::File,
    io::{Seek, SeekFrom, Write},
    os::fd::OwnedFd,
};

const BPF_LD_W_ABS: u16 = 0x20;
const BPF_JMP_JEQ_K: u16 = 0x15;
const BPF_JMP_JSET_K: u16 = 0x45;
const BPF_RET_K: u16 = 0x06;
const SECCOMP_RET_KILL_PROCESS: u32 = 0x8000_0000;
const SECCOMP_RET_ERRNO: u32 = 0x0005_0000;
const SECCOMP_RET_ALLOW: u32 = 0x7fff_0000;
const AUDIT_ARCH_X86_64: u32 = 0xc000_003e;
const EPERM: u32 = 1;
const ENOSYS: u32 = 38;
const CLONE_NAMESPACE_FLAGS: u32 = 0x7e02_0000;
const X32_SYSCALL_BIT: u32 = 0x4000_0000;
const CLONE_SYSCALL: u32 = 56;
const CLONE3_SYSCALL: u32 = 435;
const DENY_EPERM_SYSCALLS: [u32; 17] = [
    101, // ptrace
    155, // pivot_root
    165, // mount
    166, // umount2
    248, // add_key
    249, // request_key
    250, // keyctl
    272, // unshare
    308, // setns
    425, // io_uring_setup
    428, // open_tree
    429, // move_mount
    430, // fsopen
    431, // fsconfig
    432, // fsmount
    433, // fspick
    442, // mount_setattr
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Instruction {
    code: u16,
    jump_true: u8,
    jump_false: u8,
    value: u32,
}

impl Instruction {
    const fn statement(code: u16, value: u32) -> Self {
        Self {
            code,
            jump_true: 0,
            jump_false: 0,
            value,
        }
    }

    const fn jump(code: u16, value: u32, jump_true: u8, jump_false: u8) -> Self {
        Self {
            code,
            jump_true,
            jump_false,
            value,
        }
    }

    fn append_to(self, bytes: &mut Vec<u8>) {
        bytes.extend_from_slice(&self.code.to_ne_bytes());
        bytes.push(self.jump_true);
        bytes.push(self.jump_false);
        bytes.extend_from_slice(&self.value.to_ne_bytes());
    }
}

pub(super) fn sealed_filter() -> Result<OwnedFd, String> {
    let bytes = filter_bytes();
    let descriptor = rustix::fs::memfd_create(
        "flow-executor-seccomp",
        rustix::fs::MemfdFlags::CLOEXEC | rustix::fs::MemfdFlags::ALLOW_SEALING,
    )
    .map_err(|error| format!("failed to create seccomp program: {error}"))?;
    let mut file = File::from(descriptor);
    file.write_all(&bytes)
        .map_err(|error| format!("failed to write seccomp program: {error}"))?;
    file.flush()
        .map_err(|error| format!("failed to flush seccomp program: {error}"))?;
    file.seek(SeekFrom::Start(0))
        .map_err(|error| format!("failed to rewind seccomp program: {error}"))?;
    let descriptor = OwnedFd::from(file);
    rustix::fs::fcntl_add_seals(
        &descriptor,
        rustix::fs::SealFlags::SEAL
            | rustix::fs::SealFlags::SHRINK
            | rustix::fs::SealFlags::GROW
            | rustix::fs::SealFlags::WRITE,
    )
    .map_err(|error| format!("failed to seal seccomp program: {error}"))?;
    Ok(descriptor)
}

fn filter_bytes() -> Vec<u8> {
    let program = filter_instructions();
    let mut bytes = Vec::with_capacity(program.len() * 8);
    for instruction in program {
        instruction.append_to(&mut bytes);
    }
    bytes
}

fn filter_instructions() -> Vec<Instruction> {
    let mut program = vec![
        Instruction::statement(BPF_LD_W_ABS, 4),
        Instruction::jump(BPF_JMP_JEQ_K, AUDIT_ARCH_X86_64, 1, 0),
        Instruction::statement(BPF_RET_K, SECCOMP_RET_KILL_PROCESS),
        Instruction::statement(BPF_LD_W_ABS, 0),
        Instruction::jump(BPF_JMP_JSET_K, X32_SYSCALL_BIT, 0, 1),
        Instruction::statement(BPF_RET_K, SECCOMP_RET_ERRNO | EPERM),
    ];
    for syscall in DENY_EPERM_SYSCALLS {
        program.push(Instruction::jump(BPF_JMP_JEQ_K, syscall, 0, 1));
        program.push(Instruction::statement(BPF_RET_K, SECCOMP_RET_ERRNO | EPERM));
    }
    // Normal clone/fork remains usable; new namespaces are denied.
    program.extend([
        Instruction::jump(BPF_JMP_JEQ_K, CLONE_SYSCALL, 0, 3),
        Instruction::statement(BPF_LD_W_ABS, 16),
        Instruction::jump(BPF_JMP_JSET_K, CLONE_NAMESPACE_FLAGS, 0, 1),
        Instruction::statement(BPF_RET_K, SECCOMP_RET_ERRNO | EPERM),
        Instruction::jump(BPF_JMP_JEQ_K, CLONE3_SYSCALL, 0, 1),
        Instruction::statement(BPF_RET_K, SECCOMP_RET_ERRNO | ENOSYS),
        Instruction::statement(BPF_RET_K, SECCOMP_RET_ALLOW),
    ]);
    program
}

#[cfg(test)]
mod tests {
    use super::{Instruction, filter_bytes, filter_instructions};
    use std::{fs::File, io::Read};

    // Only the forward-only BPF operations used by this filter are needed.
    fn decision(program: &[Instruction], arch: u32, syscall: u32, flags: u32) -> u32 {
        let mut accumulator = 0;
        let mut cursor = 0;
        while let Some(instruction) = program.get(cursor) {
            match instruction.code {
                0x20 => {
                    accumulator = match instruction.value {
                        0 => syscall,
                        4 => arch,
                        16 => flags,
                        offset => panic!("unexpected seccomp data offset {offset}"),
                    };
                }
                0x15 | 0x45 => {
                    let matches = if instruction.code == 0x15 {
                        accumulator == instruction.value
                    } else {
                        accumulator & instruction.value != 0
                    };
                    cursor += usize::from(if matches {
                        instruction.jump_true
                    } else {
                        instruction.jump_false
                    });
                }
                0x06 => return instruction.value,
                code => panic!("unexpected BPF operation {code:#x}"),
            }
            cursor += 1;
        }
        panic!("filter did not return a decision");
    }

    #[test]
    fn filter_enforces_the_boundary_without_blocking_ordinary_processes() {
        let program = filter_instructions();
        // Keep syscall numbers, ABI values and outcomes independent of production constants.
        let check = |arch, syscall, flags, expected| {
            assert_eq!(
                decision(&program, arch, syscall, flags),
                expected,
                "arch={arch:#x}, syscall={syscall}, flags={flags:#x}"
            );
        };
        for syscall in [
            101, 155, 165, 166, 248, 249, 250, 272, 308, 425, 428, 429, 430, 431, 432, 433, 442,
        ] {
            check(0xc000_003e, syscall, 0, 0x0005_0001); // EPERM
        }
        for syscall in [0, 1, 41, 42, 57, 58, 59, 102] {
            check(0xc000_003e, syscall, 0, 0x7fff_0000); // ALLOW
        }
        for flags in [0, 17, 0x0000_0100, 0x0001_0f00] {
            check(0xc000_003e, 56, flags, 0x7fff_0000); // Ordinary clone
        }
        for namespace in [
            0x0002_0000,
            0x0200_0000,
            0x0400_0000,
            0x0800_0000,
            0x1000_0000,
            0x2000_0000,
            0x4000_0000,
            0x7e02_0000,
        ] {
            check(0xc000_003e, 56, namespace | 17, 0x0005_0001);
        }
        check(0xc000_003e, 435, 0, 0x0005_0026); // clone3 -> ENOSYS
        for syscall in [0, 56, 101, 435] {
            check(0xc000_003e, syscall | 0x4000_0000, 0, 0x0005_0001); // x32
        }
        for arch in [0x4000_0003, 0xc000_00b7] {
            for syscall in [0, 101, 0x4000_0000] {
                check(arch, syscall, 0, 0x8000_0000); // KILL_PROCESS before syscall checks
            }
        }
    }

    #[test]
    fn sealed_filter_is_ready_for_stock_bubblewrap_to_read() {
        let mut filter = File::from(super::sealed_filter().expect("filter is sealed"));
        let mut bytes = Vec::new();
        filter
            .read_to_end(&mut bytes)
            .expect("sealed filter is readable");

        assert_eq!(bytes, filter_bytes());
    }
}
