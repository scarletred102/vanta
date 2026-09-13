//! Synthetic procfs filesystem providing dynamic process and kernel runtime telemetry.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::vfs::{Filesystem, InodeMetadata, StatFs, VfsError};

pub struct ProcFs;

impl ProcFs {
    pub fn new() -> Self {
        Self
    }

    fn generate_content(&self, ino: u64) -> Option<Vec<u8>> {
        match ino {
            3 => { // /proc/cpuinfo
                Some(alloc::vec::Vec::from(
                    "processor\t: 0\nvendor_id\t: GenuineIntel\nmodel name\t: Vanta Virtual CPU\ncpu MHz\t\t: 3000.000\nflags\t\t: fpu vme de pse tsc msr pae mce cx8 apic sep mtrr pge mca cmov pat pse36 clflush mmx fxsr sse sse2 ss ht syscall nx lm constant_tsc rep_good nopl cpuid tsc_known_freq pni pclmulqdq ssse3 fma cx16 pcid sse4_1 sse4_2 x2apic movbe popcnt tsc_deadline_timer aes xsave avx f16c rdrand hypervisor lahf_lm abm 3dnowprefetch cpuid_fault ssbd ibrs ibpb stibp fsgsbase tsc_adjust bmi1 avx2 smep bmi2 erms invpcid rdseed adx smap clflushopt xsaveopt xsavec xgetbv1 xsaves arat md_clear arch_capabilities\n\n"
                ))
            }
            4 => { // /proc/meminfo
                let stats = crate::memory::stats();
                let total_kb = (stats.tracked_frames as u64) * 4;
                let free_kb = (crate::memory::free_frames_count() as u64) * 4;
                let swap_total_kb = crate::swap::swap_total_sectors() / 2;
                let cached_kb = (crate::page_cache::cached_pages_count() as u64) * 4;
                let s = format!(
                    "MemTotal:       {:8} kB\nMemFree:        {:8} kB\nMemAvailable:   {:8} kB\nBuffers:           1024 kB\nCached:         {:8} kB\nSwapTotal:      {:8} kB\nSwapFree:       {:8} kB\n",
                    total_kb, free_kb, free_kb, cached_kb, swap_total_kb, swap_total_kb
                );
                Some(s.into_bytes())
            }
            5 => { // /proc/version
                Some(alloc::vec::Vec::from(
                    "Linux version 6.1.0-vanta (vanta@build) (gcc 12.2.0) #1 SMP PREEMPT\n"
                ))
            }
            6 => { // /proc/uptime
                let ms = crate::timer::current_tick();
                let sec = ms / 1000;
                let rem = (ms % 1000) / 10;
                let s = format!("{}.{:02} {}.{:02}\n", sec, rem, sec, rem);
                Some(s.into_bytes())
            }
            8 => { // /proc/mounts
                let s = "rootfs / rootfs rw 0 0\n/dev/vda2 / ext4 rw,relatime 0 0\ntmpfs /tmp tmpfs rw,relatime 0 0\ndevtmpfs /dev devtmpfs rw,relatime 0 0\nproc /proc proc rw,relatime 0 0\nsysfs /sys sysfs rw,relatime 0 0\n";
                Some(alloc::vec::Vec::from(s))
            }
            9 => { // /proc/filesystems
                let s = "nodev\tsysfs\nnodev\trootfs\nnodev\tramfs\nnodev\ttmpfs\nnodev\tdevtmpfs\nnodev\tproc\n\text4\n\tredoxfs\n";
                Some(alloc::vec::Vec::from(s))
            }
            21 => { // /proc/self/maps
                Some(crate::scheduler::current_maps_content().into_bytes())
            }
            22 => { // /proc/self/status
                let pid = crate::scheduler::current_pid();
                let ppid = crate::scheduler::current_parent_pid();
                let s = format!(
                    "Name:\tvanta-app\nState:\tR (running)\nTgid:\t{}\nPid:\t{}\nPPid:\t{}\nThreads:\t1\n",
                    pid, pid, ppid
                );
                Some(s.into_bytes())
            }
            23 => { // /proc/self/cmdline
                let exe = crate::scheduler::current_exe_path();
                let mut bytes = exe.into_bytes();
                bytes.push(0);
                Some(bytes)
            }
            24 => { // /proc/self/stat
                let pid = crate::scheduler::current_pid();
                let ppid = crate::scheduler::current_parent_pid();
                let s = format!("{} (vanta-app) R {} 0 0 0 0 0 0 0 0 0 0 0 0 20 0 1 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0\n", pid, ppid);
                Some(s.into_bytes())
            }
            71 => { // /proc/net/dns
                let (queries, hits, entries) = crate::dns::get_dns_stats();
                let s = format!("queries: {}\nhits: {}\nentries: {}\n", queries, hits, entries);
                Some(s.into_bytes())
            }
            72 => { // /proc/net/tcp
                Some(crate::network::generate_proc_net_tcp().into_bytes())
            }
            73 => { // /proc/net/dhcp
                if let Some(lease) = crate::dhcp::get_dhcp_lease() {
                    let s = format!(
                        "state: BOUND\nip: {}.{}.{}.{}\nnetmask: {}.{}.{}.{}\ngateway: {}.{}.{}.{}\ndns: {}.{}.{}.{}\nserver_id: {}.{}.{}.{}\nlease_time: {}\nnak_retries: {}\n",
                        lease.ip[0], lease.ip[1], lease.ip[2], lease.ip[3],
                        lease.netmask[0], lease.netmask[1], lease.netmask[2], lease.netmask[3],
                        lease.gateway[0], lease.gateway[1], lease.gateway[2], lease.gateway[3],
                        lease.dns[0], lease.dns[1], lease.dns[2], lease.dns[3],
                        lease.server_id[0], lease.server_id[1], lease.server_id[2], lease.server_id[3],
                        lease.lease_time,
                        crate::dhcp::get_nak_retry_count(),
                    );
                    Some(s.into_bytes())
                } else {
                    Some(alloc::vec::Vec::from("state: STATIC\n"))
                }
            }
            _ => None,
        }
    }
}

impl Filesystem for ProcFs {
    fn root_inode(&self) -> u64 {
        1
    }

    fn lookup(&self, parent: u64, name: &str) -> Result<u64, VfsError> {
        if parent == 1 {
            match name {
                "." | "" => Ok(1),
                ".." => Ok(1),
                "self" => Ok(2),
                "cpuinfo" => Ok(3),
                "meminfo" => Ok(4),
                "version" => Ok(5),
                "uptime" => Ok(6),
                "net" => Ok(7),
                "mounts" => Ok(8),
                "filesystems" => Ok(9),
                _ => {
                    // Check if numeric PID directory
                    if name.chars().all(|c| c.is_ascii_digit()) {
                        Ok(2) // Map any numeric PID to process dir
                    } else {
                        Err(VfsError::NotFound)
                    }
                }
            }
        } else if parent == 2 { // /proc/self/
            match name {
                "." | "" => Ok(2),
                ".." => Ok(1),
                "exe" => Ok(20),
                "maps" => Ok(21),
                "status" => Ok(22),
                "cmdline" => Ok(23),
                "stat" => Ok(24),
                "fd" => Ok(25),
                _ => Err(VfsError::NotFound),
            }
        } else if parent == 7 { // /proc/net/
            match name {
                "." | "" => Ok(7),
                ".." => Ok(1),
                "dns" => Ok(71),
                "tcp" => Ok(72),
                "dhcp" => Ok(73),
                _ => Err(VfsError::NotFound),
            }
        } else if parent == 25 { // /proc/self/fd/
            match name {
                "." | "" => Ok(25),
                ".." => Ok(2),
                _ => {
                    if name.chars().all(|c| c.is_ascii_digit()) {
                        Ok(20) // numeric fd symlink
                    } else {
                        Err(VfsError::NotFound)
                    }
                }
            }
        } else {
            Err(VfsError::NotFound)
        }
    }

    fn read_inode(&self, ino: u64) -> Result<InodeMetadata, VfsError> {
        let (mode, size) = match ino {
            1 | 7 | 25 => (0o040555, 0),     // directories
            2 => (0o040555, 0),              // /proc/self behaves as directory
            20 => (0o120777, 0),             // /proc/self/exe is symlink
            _ => {
                let s = self.generate_content(ino).map(|c| c.len() as u64).unwrap_or(0);
                (0o100444, s)
            }
        };
        Ok(InodeMetadata {
            ino,
            size,
            mode,
            uid: 0,
            gid: 0,
            atime: 0,
            mtime: 0,
            ctime: 0,
        })
    }

    fn create(&self, _parent: u64, _name: &str, _mode: u32) -> Result<u64, VfsError> {
        Err(VfsError::ReadOnlyFilesystem)
    }

    fn mkdir(&self, _parent: u64, _name: &str, _mode: u32) -> Result<u64, VfsError> {
        Err(VfsError::ReadOnlyFilesystem)
    }

    fn unlink(&self, _parent: u64, _name: &str) -> Result<(), VfsError> {
        Err(VfsError::ReadOnlyFilesystem)
    }

    fn rmdir(&self, _parent: u64, _name: &str) -> Result<(), VfsError> {
        Err(VfsError::ReadOnlyFilesystem)
    }

    fn symlink(&self, _parent: u64, _name: &str, _target: &str) -> Result<u64, VfsError> {
        Err(VfsError::ReadOnlyFilesystem)
    }

    fn readlink(&self, ino: u64) -> Result<String, VfsError> {
        if ino == 20 { // /proc/self/exe
            let exe = crate::scheduler::current_exe_path();
            if !exe.is_empty() {
                Ok(exe)
            } else {
                Ok(String::from("/compat/linux/proc-conformance"))
            }
        } else if ino == 2 {
            let pid = crate::scheduler::current_pid();
            Ok(format!("{}", pid))
        } else {
            Err(VfsError::InvalidPath)
        }
    }

    fn rename(&self, _old_parent: u64, _old_name: &str, _new_parent: u64, _new_name: &str) -> Result<(), VfsError> {
        Err(VfsError::ReadOnlyFilesystem)
    }

    fn read(&self, ino: u64, offset: u64, buf: &mut [u8]) -> Result<usize, VfsError> {
        let content = self.generate_content(ino).ok_or(VfsError::NotFound)?;
        let start = (offset as usize).min(content.len());
        let end = start.saturating_add(buf.len()).min(content.len());
        let count = end - start;
        buf[..count].copy_from_slice(&content[start..end]);
        Ok(count)
    }

    fn write(&self, _ino: u64, _offset: u64, _buf: &[u8]) -> Result<usize, VfsError> {
        Err(VfsError::ReadOnlyFilesystem)
    }

    fn truncate(&self, _ino: u64, _size: u64) -> Result<(), VfsError> {
        Err(VfsError::ReadOnlyFilesystem)
    }

    fn statfs(&self) -> Result<StatFs, VfsError> {
        Ok(StatFs {
            f_type: 0x9fa0, // PROC_SUPER_MAGIC
            f_bsize: 4096,
            f_blocks: 0,
            f_bfree: 0,
            f_bavail: 0,
            f_files: 0,
            f_ffree: 0,
        })
    }

    fn sync(&self) -> Result<(), VfsError> {
        Ok(())
    }

    fn read_dir(&self, ino: u64) -> Result<Vec<String>, VfsError> {
        match ino {
            1 => Ok(alloc::vec![
                ".".into(), "..".into(), "self".into(), "cpuinfo".into(),
                "meminfo".into(), "version".into(), "uptime".into(),
                "net".into(), "mounts".into(), "filesystems".into()
            ]),
            2 => Ok(alloc::vec![
                ".".into(), "..".into(), "exe".into(), "maps".into(),
                "status".into(), "cmdline".into(), "stat".into(), "fd".into()
            ]),
            7 => Ok(alloc::vec![
                ".".into(), "..".into(), "dns".into(), "tcp".into(), "dhcp".into()
            ]),
            25 => Ok(alloc::vec![
                ".".into(), "..".into(), "0".into(), "1".into(), "2".into()
            ]),
            _ => Err(VfsError::NotDirectory),
        }
    }

    fn is_page_cached(&self) -> bool {
        false
    }
}
