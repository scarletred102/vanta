#![no_std]

use vanta_abi::{CapabilityId, Syscall};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LinuxOp {
    Read,
    Write,
    Open,
    Close,
    FStat,
    LSeek,
    GetDents,
    MMap,
    MUnmap,
    Brk,
    Pipe,
    Dup2,
    GetPid,
    Fork,
    ExecVe,
    Exit,
    Wait4,
    Kill,
    RtSigAction,
    Unsupported(u64),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Translation {
    pub linux_number: u64,
    pub operation: LinuxOp,
    pub native: Option<Syscall>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UnsupportedSyscall {
    pub number: u64,
}

pub fn translate(number: u64) -> Result<Translation, UnsupportedSyscall> {
    let (operation, native) = match number {
        0 => (LinuxOp::Read, Some(Syscall::Read)),
        1 => (LinuxOp::Write, Some(Syscall::Write)),
        2 | 257 => (LinuxOp::Open, Some(Syscall::OpenAt)),
        3 => (LinuxOp::Close, Some(Syscall::Close)),
        5 | 262 => (LinuxOp::FStat, Some(Syscall::FStat)),
        8 => (LinuxOp::LSeek, Some(Syscall::LSeek)),
        78 | 217 => (LinuxOp::GetDents, Some(Syscall::GetDents)),
        9 => (LinuxOp::MMap, Some(Syscall::MMap)),
        11 => (LinuxOp::MUnmap, Some(Syscall::MUnmap)),
        12 => (LinuxOp::Brk, Some(Syscall::Brk)),
        13 => (LinuxOp::RtSigAction, Some(Syscall::SigAction)),
        22 => (LinuxOp::Pipe, Some(Syscall::Pipe2)),
        33 => (LinuxOp::Dup2, Some(Syscall::Dup3)),
        39 => (LinuxOp::GetPid, Some(Syscall::GetPid)),
        56 => (LinuxOp::Fork, None),
        57 => (LinuxOp::Fork, None),
        59 => (LinuxOp::ExecVe, Some(Syscall::ExecVe)),
        60 => (LinuxOp::Exit, Some(Syscall::Exit)),
        61 => (LinuxOp::Wait4, Some(Syscall::WaitPid)),
        62 => (LinuxOp::Kill, Some(Syscall::Kill)),
        number => return Err(UnsupportedSyscall { number }),
    };
    Ok(Translation {
        linux_number: number,
        operation,
        native,
    })
}

pub fn is_static_elf_supported(interpreter: Option<&[u8]>) -> bool {
    interpreter.is_none()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ElfError {
    TooSmall,
    NotX86_64,
    UnsupportedType,
    InvalidProgramTable,
    DynamicInterpreter,
    NoLoadSegments,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LoadSegment {
    pub virtual_address: u64,
    pub file_offset: u64,
    pub file_size: u64,
    pub memory_size: u64,
    pub flags: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StaticElf {
    pub entry: u64,
    pub segments: [Option<LoadSegment>; 16],
    pub segment_count: usize,
}

impl StaticElf {
    pub fn parse(bytes: &[u8]) -> Result<Self, ElfError> {
        if bytes.len() < 64 || &bytes[..4] != b"\x7fELF" || bytes[4] != 2 || bytes[5] != 1 {
            return Err(ElfError::NotX86_64);
        }
        let elf_type = read_u16(bytes, 16).ok_or(ElfError::TooSmall)?;
        if elf_type != 2 && elf_type != 3 {
            return Err(ElfError::UnsupportedType);
        }
        let entry = read_u64(bytes, 24).ok_or(ElfError::TooSmall)?;
        let phoff = read_u64(bytes, 32).ok_or(ElfError::TooSmall)? as usize;
        let phentsize = read_u16(bytes, 54).ok_or(ElfError::TooSmall)? as usize;
        let phnum = read_u16(bytes, 56).ok_or(ElfError::TooSmall)? as usize;
        if phentsize < 56
            || phnum > 16
            || phoff
                .checked_add(
                    phentsize
                        .checked_mul(phnum)
                        .ok_or(ElfError::InvalidProgramTable)?,
                )
                .filter(|end| *end <= bytes.len())
                .is_none()
        {
            return Err(ElfError::InvalidProgramTable);
        }
        let mut segments = [None; 16];
        let mut segment_count = 0;
        for index in 0..phnum {
            let offset = phoff + index * phentsize;
            let kind = read_u32(bytes, offset).ok_or(ElfError::InvalidProgramTable)?;
            if kind == 3 {
                return Err(ElfError::DynamicInterpreter);
            }
            if kind != 1 {
                continue;
            }
            let file_offset = read_u64(bytes, offset + 8).ok_or(ElfError::InvalidProgramTable)?;
            let virtual_address =
                read_u64(bytes, offset + 16).ok_or(ElfError::InvalidProgramTable)?;
            let file_size = read_u64(bytes, offset + 32).ok_or(ElfError::InvalidProgramTable)?;
            let memory_size = read_u64(bytes, offset + 40).ok_or(ElfError::InvalidProgramTable)?;
            let flags = read_u32(bytes, offset + 4).ok_or(ElfError::InvalidProgramTable)?;
            if memory_size < file_size
                || file_offset
                    .checked_add(file_size)
                    .filter(|end| *end <= bytes.len() as u64)
                    .is_none()
            {
                return Err(ElfError::InvalidProgramTable);
            }
            segments[segment_count] = Some(LoadSegment {
                virtual_address,
                file_offset,
                file_size,
                memory_size,
                flags,
            });
            segment_count += 1;
        }
        if segment_count == 0 {
            return Err(ElfError::NoLoadSegments);
        }
        Ok(Self {
            entry,
            segments,
            segment_count,
        })
    }
}

fn read_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        bytes.get(offset..offset + 2)?.try_into().ok()?,
    ))
}
fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        bytes.get(offset..offset + 4)?.try_into().ok()?,
    ))
}
fn read_u64(bytes: &[u8], offset: usize) -> Option<u64> {
    Some(u64::from_le_bytes(
        bytes.get(offset..offset + 8)?.try_into().ok()?,
    ))
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LinuxSyscallRequest {
    pub number: u64,
    pub args: [u64; 6],
    pub authority: CapabilityId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BrokerDecision {
    Native { syscall: Syscall, args: [u64; 4] },
    ProcessPrimitive { operation: LinuxOp },
    Unsupported { number: u64 },
}

pub fn broker(request: LinuxSyscallRequest) -> BrokerDecision {
    let Ok(translation) = translate(request.number) else {
        return BrokerDecision::Unsupported {
            number: request.number,
        };
    };
    let Some(syscall) = translation.native else {
        return BrokerDecision::ProcessPrimitive {
            operation: translation.operation,
        };
    };
    BrokerDecision::Native {
        syscall,
        args: [
            request.args[0],
            request.args[1],
            request.args[2],
            request.args[3],
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_static_x86_64_load_metadata() {
        let mut image = [0u8; 120];
        image[..4].copy_from_slice(b"\x7fELF");
        image[4] = 2;
        image[5] = 1;
        image[16..18].copy_from_slice(&2u16.to_le_bytes());
        image[24..32].copy_from_slice(&0x401000u64.to_le_bytes());
        image[32..40].copy_from_slice(&64u64.to_le_bytes());
        image[54..56].copy_from_slice(&56u16.to_le_bytes());
        image[56..58].copy_from_slice(&1u16.to_le_bytes());
        image[64..68].copy_from_slice(&1u32.to_le_bytes());
        image[68..72].copy_from_slice(&5u32.to_le_bytes());
        image[72..80].copy_from_slice(&0u64.to_le_bytes());
        image[80..88].copy_from_slice(&0x400000u64.to_le_bytes());
        image[96..104].copy_from_slice(&1u64.to_le_bytes());
        image[104..112].copy_from_slice(&1u64.to_le_bytes());
        let elf = StaticElf::parse(&image).unwrap();
        assert_eq!(elf.entry, 0x401000);
        assert_eq!(elf.segment_count, 1);
    }

    #[test]
    fn broker_maps_file_io_and_reports_process_primitives() {
        let request = LinuxSyscallRequest {
            number: 1,
            args: [1, 2, 3, 4, 5, 6],
            authority: CapabilityId::INVALID,
        };
        assert_eq!(
            broker(request),
            BrokerDecision::Native {
                syscall: Syscall::Write,
                args: [1, 2, 3, 4]
            }
        );
        let fork = LinuxSyscallRequest {
            number: 57,
            args: [0; 6],
            authority: CapabilityId::INVALID,
        };
        assert_eq!(
            broker(fork),
            BrokerDecision::ProcessPrimitive {
                operation: LinuxOp::Fork
            }
        );
    }

    #[test]
    fn rejects_interpreters_and_unknown_syscalls() {
        assert!(!is_static_elf_supported(Some(
            b"/lib64/ld-linux-x86-64.so.2"
        )));
        assert_eq!(translate(9999), Err(UnsupportedSyscall { number: 9999 }));
    }
}
