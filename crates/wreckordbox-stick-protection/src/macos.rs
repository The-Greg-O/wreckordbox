//! What Stick Protection reads from the machine, on macOS.
//!
//! macOS has no mount table file, no `statx` and no `/proc`. The same facts
//! come from elsewhere, each failing closed when it cannot be read:
//!
//! - **The volume holding a path** is asked of the kernel with `statfs` on
//!   the path's nearest existing ancestor, and handed on as one line shaped
//!   like `/proc/self/mountinfo`'s, so the rule that reads the table is the
//!   same one on every system. When `statfs` is refused for a path under
//!   `/Volumes`, the folder under `/Volumes` stands for the mount, of an
//!   unknown kind.
//! - **Removable** is the kernel's `MNT_REMOVABLE` flag (an external
//!   drive). A local volume without it is only called fixed when it is the
//!   system's own; any other is "cannot tell".
//! - **The boot** is `kern.bootsessionuuid`.
//! - **The mount incarnation** has no kernel id. It is told by the folder
//!   the volume is mounted on (its inode in the parent's listing, which is
//!   the covered folder's: macOS makes a new one under `/Volumes` for every
//!   mount, and APFS never reuses an inode number) together with the device
//!   node the volume was mounted from (made anew each time the device
//!   appears).

use std::ffi::OsString;
use std::os::unix::ffi::{OsStrExt as _, OsStringExt as _};
use std::os::unix::fs::{DirEntryExt as _, MetadataExt as _};
use std::path::{Path, PathBuf};

const MNT_REMOVABLE: u32 = 0x0000_0200;
const MNT_LOCAL: u32 = 0x0000_1000;
const MNT_ROOTFS: u32 = 0x0000_4000;

/// The device field of a line for a removable (external) volume.
const REMOVABLE: &str = "removable";
/// The device field for a volume that is certainly not removable.
const FIXED: &str = "fixed";
/// The device field when it cannot be told.
const UNKNOWN: &str = "unknown";

/// A NUL-ended C string field of `statfs`, as a path.
fn text(field: &[std::ffi::c_char]) -> PathBuf {
    let bytes: Vec<u8> = field
        .iter()
        .map(|c| c.to_ne_bytes()[0])
        .take_while(|&b| b != 0)
        .collect();
    PathBuf::from(OsString::from_vec(bytes))
}

/// The nearest ancestor of `path` that exists (`path` itself when it does).
fn existing(path: &Path) -> Option<&Path> {
    path.ancestors().find(|dir| dir.symlink_metadata().is_ok())
}

/// A mount point as `/proc/self/mountinfo` writes one: space, tab, newline
/// and backslash as octal escapes.
fn escape(point: &Path) -> String {
    let mut out = Vec::new();
    for &byte in point.as_os_str().as_bytes() {
        match byte {
            b' ' | b'\t' | b'\n' | b'\\' => out.extend(format!("\\{byte:03o}").bytes()),
            other => out.push(other),
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn line(device: &str, point: &Path) -> String {
    format!("0 0 {device} / {} - - - -\n", escape(point))
}

/// The mount table as far as `resolved` (canonical) needs it: the one mount
/// holding it.
pub(crate) fn mount_table(resolved: &Path) -> std::io::Result<String> {
    let at = existing(resolved).ok_or_else(|| std::io::Error::other("no existing ancestor"))?;
    match rustix::fs::statfs(at) {
        Ok(found) => {
            let point = text(&found.f_mntonname);
            let kind = text(&found.f_fstypename);
            let system = found.f_flags & MNT_ROOTFS != 0
                || point.starts_with("/System/Volumes")
                || kind == Path::new("devfs")
                || kind == Path::new("autofs");
            let device = if found.f_flags & MNT_REMOVABLE != 0 {
                REMOVABLE
            } else if found.f_flags & MNT_LOCAL == 0 || system {
                FIXED
            } else {
                UNKNOWN
            };
            // The system's data volume is reached through the root's
            // folders (`/Users`, `/private`...), not under its own mount
            // point: such a path is on the root volume, as far as paths go.
            let point = if resolved.starts_with(&point) {
                point
            } else {
                PathBuf::from("/")
            };
            Ok(line(device, &point))
        }
        Err(error) => {
            // Refused (a volume this process may not look at): under
            // `/Volumes`, the folder there is the mount, of an unknown kind.
            let mut parts = resolved.components();
            let under_volumes = (
                parts.next(),
                parts.next().map(|c| c.as_os_str().to_owned()),
                parts.next(),
            );
            match under_volumes {
                (Some(std::path::Component::RootDir), Some(volumes), Some(name))
                    if volumes == "Volumes" =>
                {
                    Ok(line(UNKNOWN, &Path::new("/Volumes").join(name.as_os_str())))
                }
                _ => Err(error.into()),
            }
        }
    }
}

/// Whether the volume whose table line carries `device` is removable.
pub(crate) fn removable(device: &str) -> Option<bool> {
    match device {
        REMOVABLE => Some(true),
        FIXED => Some(false),
        _ => None,
    }
}

/// This boot's id; `None` when it cannot be read.
pub(crate) fn boot_id() -> Option<String> {
    static BOOT: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    BOOT.get_or_init(|| {
        let out = std::process::Command::new("/usr/sbin/sysctl")
            .args(["-n", "kern.bootsessionuuid"])
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let id = String::from_utf8(out.stdout).ok()?;
        let id = id.trim();
        (!id.is_empty()).then(|| id.to_owned())
    })
    .clone()
}

/// The device node a volume was mounted from: `/dev/disk4s1`, or the disk
/// named in a `msdos://disk4s1/NAME` style source.
fn device_node(from: &Path) -> Option<PathBuf> {
    if from.starts_with("/dev") {
        return Some(from.to_path_buf());
    }
    let from = from.to_str()?;
    let (_, rest) = from.split_once("://")?;
    let disk = rest.split('/').next()?;
    (disk.starts_with("disk") && disk.bytes().all(|b| b.is_ascii_alphanumeric()))
        .then(|| Path::new("/dev").join(disk))
}

/// An id of this mount incarnation of the volume mounted at `root`
/// (canonical); `None` when it cannot be established.
pub(crate) fn mount_id(root: &Path) -> Option<u64> {
    let (parent, name) = (root.parent()?, root.file_name()?);
    let covered = std::fs::read_dir(parent)
        .ok()?
        .filter_map(Result::ok)
        .find(|entry| entry.file_name() == name)?
        .ino();
    let node = rustix::fs::statfs(root)
        .ok()
        .and_then(|found| device_node(&text(&found.f_mntfromname)))
        .and_then(|node| std::fs::metadata(node).ok())
        .map_or((0, 0), |meta| (meta.ino(), meta.ctime()));
    // FNV-1a over the three numbers.
    let mut id: u64 = 0xcbf2_9ce4_8422_2325;
    let parts = [
        covered.to_le_bytes(),
        node.0.to_le_bytes(),
        node.1.to_le_bytes(),
    ];
    for byte in parts.iter().flatten() {
        id ^= u64::from(*byte);
        id = id.wrapping_mul(0x0000_0100_0000_01b3);
    }
    Some(id)
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "a test fails loudly on its own setup")]
mod tests {
    use super::*;

    #[test]
    fn a_mount_point_with_a_space_survives_the_table() {
        let made = line(UNKNOWN, Path::new("/Volumes/DJ STICK\\1"));
        let mount = crate::containing_mount(&made, Path::new("/Volumes/DJ STICK\\1/PIONEER"))
            .expect("the mount is found");
        assert_eq!(mount.point, Path::new("/Volumes/DJ STICK\\1"));
        assert_eq!(removable(&mount.device), None);
    }

    #[test]
    fn a_home_folder_is_on_a_fixed_volume() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let resolved = dir.path().canonicalize().expect("it exists");
        let table = mount_table(&resolved.join("not/yet/there")).expect("statfs answers");
        let mount = crate::containing_mount(&table, &resolved).expect("the mount is found");
        assert_eq!(removable(&mount.device), Some(false));
    }

    #[test]
    fn a_refused_volume_under_volumes_is_its_own_mount_of_an_unknown_kind() {
        // The line the fallback makes, read by the shared rule.
        let made = line(UNKNOWN, Path::new("/Volumes/GIG"));
        let facts = crate::facts_in(&made, Path::new("/Volumes/GIG/PIONEER/rekordbox"))
            .expect("the mount is found");
        assert!(facts.media);
        assert_eq!(facts.removable, None);
        assert!(crate::is_stick(&facts), "unreadable: fails closed");
    }

    #[test]
    fn the_device_node_is_read_from_either_source_form() {
        assert_eq!(
            device_node(Path::new("/dev/disk4s1")),
            Some(PathBuf::from("/dev/disk4s1"))
        );
        assert_eq!(
            device_node(Path::new("msdos://disk4s1/GIG")),
            Some(PathBuf::from("/dev/disk4s1"))
        );
        assert_eq!(device_node(Path::new("//server/share")), None);
        assert_eq!(device_node(Path::new("x://../etc")), None);
    }

    #[test]
    fn a_folder_made_again_is_another_mount() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let root = dir.path().canonicalize().expect("it exists").join("GIG");
        std::fs::create_dir(&root).expect("made");
        let first = mount_id(&root).expect("an id");
        assert_eq!(mount_id(&root), Some(first), "stable while it stays");
        std::fs::remove_dir(&root).expect("removed");
        assert_eq!(mount_id(&root), None, "gone: no identity");
        std::fs::create_dir(&root).expect("made again");
        assert_ne!(mount_id(&root), Some(first));
    }
}
