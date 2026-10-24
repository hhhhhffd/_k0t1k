//! Automatic memory for instances with `MemorySettings::auto`, the same rules as Prism Launcher.
//!
//! Prism (`SysInfo::defaultMaxJvmMem`, `MinMemAlloc`) gives 4096 MiB, or two thirds of the RAM when the PC has less
//! than 6 GiB, with a 512 MiB minimum, and adds no GC flags (Java 17+ already picks G1). Mojang's launcher also
//! defaults to 4 GiB since the July 2026 requirements update. A modpack's own recommendation is applied at import
//! instead ([`cap_recommended_ram`]), as Prism does for CurseForge's `recommendedRam`.

use crate::settings::default_max_memory_mib;

/// Prism's default `MinMemAlloc`.
const MIN_MIB: u32 = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tuning {
    pub min_mib: u32,
    pub max_mib: u32,
}

pub fn tune(total_mib: Option<u64>) -> Tuning {
    let max_mib = default_max_memory_mib(total_mib);
    Tuning { min_mib: MIN_MIB.min(max_mib), max_mib }
}

/// A modpack's recommended heap, limited to 90% of the RAM (Prism's `FlameCreationTask`).
pub fn cap_recommended_ram(recommended_mib: u32, total_mib: Option<u64>) -> u32 {
    match total_mib {
        Some(total) => recommended_mib.min((total * 9 / 10) as u32),
        None => recommended_mib,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_prism_defaults() {
        assert_eq!(tune(Some(32 * 1024)), Tuning { min_mib: 512, max_mib: 4096 });
        assert_eq!(tune(Some(6144)), Tuning { min_mib: 512, max_mib: 4096 });
        // Below 6 GiB: two thirds of the RAM.
        assert_eq!(tune(Some(4096)).max_mib, 2730);
        assert_eq!(tune(None).max_mib, 2048);
    }

    #[test]
    fn recommended_ram_is_capped_at_ninety_percent() {
        assert_eq!(cap_recommended_ram(8192, Some(16 * 1024)), 8192);
        assert_eq!(cap_recommended_ram(8192, Some(8 * 1024)), 7372);
        assert_eq!(cap_recommended_ram(8192, None), 8192);
    }
}
