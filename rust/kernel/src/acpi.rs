//! Checked ACPI root-table discovery supplied by the Limine RSDP response.

use alloc::vec::Vec;

const RSDP_V1_LENGTH: usize = 20;
const RSDP_V2_LENGTH: usize = 36;
const SDT_HEADER_LENGTH: usize = 36;
const MAX_TABLE_LENGTH: usize = 1024 * 1024;
const MAX_ROOT_ENTRIES: usize = 1024;
const ACPI_WINDOW: u64 = 0xffff_fe80_0000_0000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AcpiError {
    NoHhdm,
    NullAddress,
    AddressOverflow,
    InvalidRsdp,
    InvalidChecksum,
    InvalidTableLength,
    InvalidRootTable,
    InvalidMadt,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RootTable {
    Rsdt,
    Xsdt,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MadtInfo {
    pub local_apic_address: u32,
    pub flags: u32,
    pub enabled_processors: usize,
    pub io_apics: usize,
    pub interrupt_source_overrides: usize,
    pub io_apic: Option<IoApicDescriptor>,
    pub timer_gsi: u32,
    pub keyboard_gsi: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IoApicDescriptor {
    pub physical_address: u32,
    pub global_irq_base: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct McfgInfo {
    pub regions: usize,
    pub first_region: Option<PciEcamRegion>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PciEcamRegion {
    pub base_address: u64,
    pub segment_group: u16,
    pub start_bus: u8,
    pub end_bus: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AcpiInfo {
    pub revision: u8,
    pub root: RootTable,
    pub table_count: usize,
    pub madt: Option<MadtInfo>,
    pub mcfg: Option<McfgInfo>,
}

pub fn initialize(rsdp_physical: u64) -> Result<AcpiInfo, AcpiError> {
    let rsdp_v1 = physical_bytes(rsdp_physical, RSDP_V1_LENGTH)?;
    if &rsdp_v1[..8] != b"RSD PTR " || !checksum(&rsdp_v1) {
        return Err(AcpiError::InvalidRsdp);
    }
    let revision = rsdp_v1[15];
    let rsdt_address = read_u32(&rsdp_v1, 16) as u64;

    let (root, root_physical) = if revision >= 2 {
        let rsdp = physical_bytes(rsdp_physical, RSDP_V2_LENGTH)?;
        let length = read_u32(&rsdp, 20) as usize;
        if length < RSDP_V2_LENGTH || length > MAX_TABLE_LENGTH {
            return Err(AcpiError::InvalidRsdp);
        }
        let complete_rsdp = physical_bytes(rsdp_physical, length)?;
        if !checksum(&complete_rsdp) {
            return Err(AcpiError::InvalidChecksum);
        }
        let xsdt_address = read_u64(&complete_rsdp, 24);
        if xsdt_address != 0 {
            (RootTable::Xsdt, xsdt_address)
        } else {
            (RootTable::Rsdt, rsdt_address)
        }
    } else {
        (RootTable::Rsdt, rsdt_address)
    };

    let root_table = sdt(root_physical)?;
    let expected_signature = match root {
        RootTable::Rsdt => b"RSDT",
        RootTable::Xsdt => b"XSDT",
    };
    if &root_table[..4] != expected_signature {
        return Err(AcpiError::InvalidRootTable);
    }

    let entry_size = match root {
        RootTable::Rsdt => 4,
        RootTable::Xsdt => 8,
    };
    let entries = root_table.len() - SDT_HEADER_LENGTH;
    if entries % entry_size != 0 || entries / entry_size > MAX_ROOT_ENTRIES {
        return Err(AcpiError::InvalidRootTable);
    }

    let mut info = AcpiInfo {
        revision,
        root,
        table_count: entries / entry_size,
        madt: None,
        mcfg: None,
    };
    for offset in (SDT_HEADER_LENGTH..root_table.len()).step_by(entry_size) {
        let table_physical = if entry_size == 4 {
            read_u32(&root_table, offset) as u64
        } else {
            read_u64(&root_table, offset)
        };
        let table = sdt(table_physical)?;
        match &table[..4] {
            b"APIC" => info.madt = Some(parse_madt(&table)?),
            b"MCFG" => info.mcfg = Some(parse_mcfg(&table)?),
            _ => {}
        }
    }
    Ok(info)
}

pub fn self_check() -> bool {
    checksum(&[1, 2, 3, 250])
        && !checksum(&[1, 2, 3, 249])
        && matches!(RootTable::Xsdt, RootTable::Xsdt)
}

fn sdt(physical_address: u64) -> Result<Vec<u8>, AcpiError> {
    let header = physical_bytes(physical_address, SDT_HEADER_LENGTH)?;
    let length = read_u32(&header, 4) as usize;
    if length < SDT_HEADER_LENGTH || length > MAX_TABLE_LENGTH {
        return Err(AcpiError::InvalidTableLength);
    }
    let table = physical_bytes(physical_address, length)?;
    if !checksum(&table) {
        return Err(AcpiError::InvalidChecksum);
    }
    Ok(table)
}

fn parse_madt(table: &[u8]) -> Result<MadtInfo, AcpiError> {
    if table.len() < SDT_HEADER_LENGTH + 8 {
        return Err(AcpiError::InvalidMadt);
    }
    let mut info = MadtInfo {
        local_apic_address: read_u32(table, SDT_HEADER_LENGTH),
        flags: read_u32(table, SDT_HEADER_LENGTH + 4),
        enabled_processors: 0,
        io_apics: 0,
        interrupt_source_overrides: 0,
        io_apic: None,
        timer_gsi: 0,
        keyboard_gsi: 1,
    };
    let mut offset = SDT_HEADER_LENGTH + 8;
    while offset < table.len() {
        if table.len() - offset < 2 {
            return Err(AcpiError::InvalidMadt);
        }
        let kind = table[offset];
        let length = table[offset + 1] as usize;
        if length < 2 || offset + length > table.len() {
            return Err(AcpiError::InvalidMadt);
        }
        match kind {
            0 if length >= 8 && read_u32(table, offset + 4) & 1 != 0 => {
                info.enabled_processors += 1
            }
            1 if length >= 12 => {
                info.io_apics += 1;
                if info.io_apic.is_none() {
                    info.io_apic = Some(IoApicDescriptor {
                        physical_address: read_u32(table, offset + 4),
                        global_irq_base: read_u32(table, offset + 8),
                    });
                }
            }
            2 if length >= 10 => {
                info.interrupt_source_overrides += 1;
                if table[offset + 2] == 0 {
                    match table[offset + 3] {
                        0 => info.timer_gsi = read_u32(table, offset + 4),
                        1 => info.keyboard_gsi = read_u32(table, offset + 4),
                        _ => {}
                    }
                }
            }
            9 if length >= 16 && read_u32(table, offset + 8) & 1 != 0 => {
                info.enabled_processors += 1
            }
            _ => {}
        }
        offset += length;
    }
    Ok(info)
}

fn parse_mcfg(table: &[u8]) -> Result<McfgInfo, AcpiError> {
    const MCFG_FIXED_LENGTH: usize = SDT_HEADER_LENGTH + 8;
    const MCFG_REGION_LENGTH: usize = 16;
    if table.len() < MCFG_FIXED_LENGTH
        || (table.len() - MCFG_FIXED_LENGTH) % MCFG_REGION_LENGTH != 0
    {
        return Err(AcpiError::InvalidTableLength);
    }
    let regions = (table.len() - MCFG_FIXED_LENGTH) / MCFG_REGION_LENGTH;
    let first_region = (regions != 0).then(|| PciEcamRegion {
        base_address: read_u64(table, MCFG_FIXED_LENGTH),
        segment_group: u16::from_le_bytes(
            table[MCFG_FIXED_LENGTH + 8..MCFG_FIXED_LENGTH + 10]
                .try_into()
                .expect("checked MCFG field"),
        ),
        start_bus: table[MCFG_FIXED_LENGTH + 10],
        end_bus: table[MCFG_FIXED_LENGTH + 11],
    });
    Ok(McfgInfo {
        regions,
        first_region,
    })
}

fn physical_bytes(physical_address: u64, length: usize) -> Result<Vec<u8>, AcpiError> {
    if physical_address == 0 {
        return Err(AcpiError::NullAddress);
    }
    let length_u64 = u64::try_from(length).map_err(|_| AcpiError::AddressOverflow)?;
    physical_address
        .checked_add(length_u64)
        .ok_or(AcpiError::AddressOverflow)?;
    if crate::paging::translate(ACPI_WINDOW).is_some() {
        return Err(AcpiError::NoHhdm);
    }

    let mut bytes = Vec::with_capacity(length);
    let mut current = physical_address;
    let mut remaining = length;
    while remaining != 0 {
        let page = current & !(crate::memory::PAGE_SIZE - 1);
        let page_offset = (current - page) as usize;
        let copied = remaining.min(crate::memory::PAGE_SIZE as usize - page_offset);
        crate::paging::map(crate::paging::current_address_space(), ACPI_WINDOW, page, 0)
            .map_err(|_| AcpiError::NoHhdm)?;
        let source = unsafe {
            core::slice::from_raw_parts((ACPI_WINDOW as usize + page_offset) as *const u8, copied)
        };
        bytes.extend_from_slice(source);
        crate::paging::unmap(crate::paging::current_address_space(), ACPI_WINDOW)
            .map_err(|_| AcpiError::NoHhdm)?;
        current = current
            .checked_add(copied as u64)
            .ok_or(AcpiError::AddressOverflow)?;
        remaining -= copied;
    }
    Ok(bytes)
}

fn checksum(bytes: &[u8]) -> bool {
    bytes.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)) == 0
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("checked ACPI field"),
    )
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(
        bytes[offset..offset + 8]
            .try_into()
            .expect("checked ACPI field"),
    )
}
