//! Automatic memory and GC choice for instances with `MemorySettings::auto`.

use std::path::Path;

use crate::component::ComponentSpec;

/// Heap of an unmodded game.
const VANILLA_MAX_MIB: u64 = 2048;
/// Heap of a game with a mod loader and up to [`FREE_MODS`] mods.
const MODDED_BASE_MIB: u64 = 3072;
const FREE_MODS: usize = 50;
const MIB_PER_EXTRA_MOD: u64 = 40;
const MAX_MIB_CAP: u64 = 8192;
const MIN_MAX_MIB: u64 = 1024;
/// Left to the OS and other programs.
const OS_RESERVE_MIB: u64 = 3072;
/// Used when the platform does not report its RAM.
const UNKNOWN_RAM_CAP_MIB: u64 = 4096;
const MIN_MIN_MIB: u64 = 512;

/// Aikar-style G1 flags for a client. Every flag exists from Java 8 to 21+; the experimental ones
/// are unlocked before they are used.
const G1_FLAGS: [&str; 9] = [
    "-XX:+UseG1GC",
    "-XX:+ParallelRefProcEnabled",
    "-XX:MaxGCPauseMillis=200",
    "-XX:+UnlockExperimentalVMOptions",
    "-XX:+DisableExplicitGC",
    "-XX:G1NewSizePercent=30",
    "-XX:G1MaxNewSizePercent=40",
    "-XX:G1HeapRegionSize=8M",
    "-XX:G1ReservePercent=20",
];
const G1_IHOP_FLAG: &str = "-XX:InitiatingHeapOccupancyPercent=15";

/// Uids of the loaders that make an instance modded.
const LOADER_UIDS: [&str; 4] = ["net.fabricmc.fabric-loader", "org.quiltmc.quilt-loader", "net.minecraftforge", "net.neoforged"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tuning {
    pub min_mib: u32,
    pub max_mib: u32,
    /// Extra JVM arguments; empty when the user already chose a GC.
    pub gc_args: Vec<String>,
}

pub fn has_mod_loader(components: &[ComponentSpec]) -> bool {
    components.iter().any(|component| LOADER_UIDS.contains(&component.uid.as_str()))
}

/// Enabled mods (`*.jar`, not `*.jar.disabled`) directly in the mods folder; 0 when it does not exist.
pub fn count_mods(mods_dir: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(mods_dir) else {
        return 0;
    };
    entries
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().extension().is_some_and(|extension| extension.eq_ignore_ascii_case("jar")))
        .count()
}

/// Heap and GC flags for the machine and instance. `user_jvm_args` are checked for an explicit GC choice.
pub fn tune(total_mib: Option<u64>, modded: bool, mod_count: usize, user_jvm_args: &[String]) -> Tuning {
    let wanted = if modded {
        MODDED_BASE_MIB + mod_count.saturating_sub(FREE_MODS) as u64 * MIB_PER_EXTRA_MOD
    } else {
        VANILLA_MAX_MIB
    };
    let ceiling = match total_mib {
        Some(total) => total.saturating_sub(OS_RESERVE_MIB),
        None => UNKNOWN_RAM_CAP_MIB,
    };
    // The floor wins over the RAM ceiling: a game with less than 1 GiB does not start.
    let max = wanted.min(MAX_MIB_CAP).min(ceiling).max(MIN_MAX_MIB);
    let min = (max / 2).max(MIN_MIN_MIB).min(max);

    let has_gc = user_jvm_args.iter().any(|arg| arg.starts_with("-XX:+Use") && arg.ends_with("GC"));
    let gc_args = if has_gc {
        Vec::new()
    } else {
        G1_FLAGS.iter().chain([&G1_IHOP_FLAG]).map(|flag| (*flag).to_owned()).collect()
    };
    Tuning { min_mib: min as u32, max_mib: max as u32, gc_args }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIB: u64 = 1024;

    #[test]
    fn vanilla_gets_two_gib_and_modded_scales_with_mod_count() {
        assert_eq!(tune(Some(16 * GIB), false, 0, &[]).max_mib, 2048);
        assert_eq!(tune(Some(16 * GIB), false, 300, &[]).max_mib, 2048);
        assert_eq!(tune(Some(16 * GIB), true, 10, &[]).max_mib, 3072);
        assert_eq!(tune(Some(16 * GIB), true, 50, &[]).max_mib, 3072);
        assert_eq!(tune(Some(16 * GIB), true, 150, &[]).max_mib, 3072 + 100 * 40);
        assert_eq!(tune(Some(64 * GIB), true, 1000, &[]).max_mib, 8192);
    }

    #[test]
    fn heap_leaves_room_for_the_os_but_not_below_one_gib() {
        assert_eq!(tune(Some(5 * GIB), true, 10, &[]).max_mib, 5 * 1024 - 3072);
        assert_eq!(tune(Some(4 * GIB), true, 10, &[]).max_mib, 1024);
        assert_eq!(tune(Some(2 * GIB), false, 0, &[]).max_mib, 1024);
        assert_eq!(tune(None, true, 500, &[]).max_mib, 4096);
    }

    #[test]
    fn min_heap_is_half_of_max_and_at_least_512() {
        let tuning = tune(Some(16 * GIB), true, 10, &[]);
        assert_eq!((tuning.min_mib, tuning.max_mib), (1536, 3072));
        let small = tune(Some(2 * GIB), false, 0, &[]);
        assert_eq!((small.min_mib, small.max_mib), (512, 1024));
    }

    #[test]
    fn gc_flags_are_added_unless_the_user_picked_a_gc() {
        let tuned = tune(Some(16 * GIB), false, 0, &[]);
        assert_eq!(tuned.gc_args.first().map(String::as_str), Some("-XX:+UseG1GC"));
        let unlock = tuned.gc_args.iter().position(|arg| arg == "-XX:+UnlockExperimentalVMOptions").unwrap();
        let new_size = tuned.gc_args.iter().position(|arg| arg == "-XX:G1NewSizePercent=30").unwrap();
        assert!(unlock < new_size);
        assert!(tuned.gc_args.contains(&"-XX:InitiatingHeapOccupancyPercent=15".to_owned()));

        for choice in ["-XX:+UseZGC", "-XX:+UseParallelGC", "-XX:+UseG1GC"] {
            assert!(tune(Some(16 * GIB), false, 0, &["-Dx=1".to_owned(), choice.to_owned()]).gc_args.is_empty());
        }
        // Other -XX options are not a GC choice.
        assert!(!tune(Some(16 * GIB), false, 0, &["-XX:+UseStringDeduplication".to_owned()]).gc_args.is_empty());
    }

    #[test]
    fn loader_and_mods_are_detected() {
        let spec = |uid: &str| ComponentSpec { uid: uid.to_owned(), version: "1".to_owned(), dependency_only: false };
        assert!(!has_mod_loader(&[spec("net.minecraft"), spec("org.lwjgl3")]));
        assert!(has_mod_loader(&[spec("net.minecraft"), spec("net.neoforged")]));

        let dir = tempfile::tempdir().unwrap();
        assert_eq!(count_mods(&dir.path().join("missing")), 0);
        for name in ["a.jar", "b.JAR", "c.jar.disabled", "readme.txt"] {
            std::fs::write(dir.path().join(name), b"").unwrap();
        }
        assert_eq!(count_mods(dir.path()), 2);
    }
}
