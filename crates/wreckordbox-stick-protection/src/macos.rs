//! What Stick Protection reads from the machine, on macOS.
//!
//! macOS has no mount table file, no `statx` and no `/proc`. The same facts
//! come from elsewhere, each failing closed when it cannot be read:
//!
//! - **The volume holding a path** is asked of the kernel with `statfs` on
//!   the path's nearest existing ancestor, and handed on as one line shaped
//!   like `/proc/self/mountinfo`'s, so the rule that reads the table is the
//!   same one on every system. A path the machine will not answer about
//!   (anything but "nothing there"), or whose volume's mount point the
//!   path does not spell, is never taken to be on the root volume: the
//!   folder under `/Volumes` (elsewhere, the path's own leading folders)
//!   stands for its mount, of an unknown kind, which the rule protects
//!   when its root cannot be read.
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

/// What `statfs` says of the volume holding a path.
struct Found {
    /// `f_mntonname`.
    point: PathBuf,
    /// `f_fstypename`.
    kind: PathBuf,
    /// `f_flags`.
    flags: u32,
}

/// How the machine is asked: whether something is at a path (`lstat`), and
/// which volume holds it (`statfs`). A test puts its own answers here.
struct Lookups<'a> {
    present: &'a dyn Fn(&Path) -> std::io::Result<()>,
    volume: &'a dyn Fn(&Path) -> std::io::Result<Found>,
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

/// The line for a path the machine would not answer about, or answered
/// about in a way that does not fit the path: a mount of an unknown kind.
/// Under `/Volumes` it is the folder there; elsewhere the first `depth`
/// parts of the path. Never the root volume: what cannot be told is not
/// taken for an ordinary folder.
fn unknown(resolved: &Path, depth: usize) -> String {
    let mut parts = resolved.components();
    let under_volumes = matches!(
        (parts.next(), parts.next()),
        (Some(std::path::Component::RootDir), Some(volumes)) if volumes.as_os_str() == "Volumes"
    );
    let depth = if under_volumes { 3 } else { depth.max(2) };
    let point: PathBuf = resolved.components().take(depth).collect();
    line(UNKNOWN, &point)
}

/// The mount table as far as `resolved` (canonical) needs it: the one mount
/// holding it.
pub(crate) fn mount_table(resolved: &Path) -> std::io::Result<String> {
    mount_table_with(
        resolved,
        &Lookups {
            present: &|path| path.symlink_metadata().map(|_| ()),
            volume: &|path| {
                let found = rustix::fs::statfs(path)?;
                Ok(Found {
                    point: text(&found.f_mntonname),
                    kind: text(&found.f_fstypename),
                    flags: found.f_flags,
                })
            },
        },
    )
}

fn mount_table_with(resolved: &Path, lookups: &Lookups) -> std::io::Result<String> {
    // The nearest ancestor that exists. Only "nothing there" moves on to
    // the parent: a path that may not be looked at is not taken to be on
    // whatever volume its parent is on.
    let (mut at, mut refused) = (None, None);
    for dir in resolved.ancestors() {
        match (lookups.present)(dir) {
            Ok(()) => {
                at = Some(dir);
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            // The highest folder that may not be looked at stands for the
            // mount.
            Err(_) => refused = Some(dir.components().count()),
        }
    }
    if let Some(depth) = refused {
        return Ok(unknown(resolved, depth));
    }
    let at = at.ok_or_else(|| std::io::Error::other("no existing ancestor"))?;
    let Ok(found) = (lookups.volume)(at) else {
        return Ok(unknown(resolved, at.components().count()));
    };
    let system = found.flags & MNT_ROOTFS != 0
        || found.point.starts_with("/System/Volumes")
        || found.kind == Path::new("devfs")
        || found.kind == Path::new("autofs");
    let device = if found.flags & MNT_REMOVABLE != 0 {
        REMOVABLE
    } else if found.flags & MNT_LOCAL == 0 || system {
        FIXED
    } else {
        UNKNOWN
    };
    if resolved.starts_with(&found.point) {
        return Ok(line(device, &found.point));
    }
    if system {
        // The system's data volume is reached through the root's folders
        // (`/Users`, `/private`...), not under its own mount point: such a
        // path is on the root volume, as far as paths go.
        return Ok(line(device, Path::new("/")));
    }
    // Another volume's mount point that the path does not spell (a letter
    // case, a link that could not be resolved): the path's own spelling
    // stands for it.
    Ok(unknown(resolved, found.point.components().count()))
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

    fn found(point: &str, kind: &str, flags: u32) -> Found {
        Found {
            point: point.into(),
            kind: kind.into(),
            flags,
        }
    }

    fn denied() -> std::io::Error {
        std::io::ErrorKind::PermissionDenied.into()
    }

    fn missing() -> std::io::Error {
        std::io::ErrorKind::NotFound.into()
    }

    /// The facts the shared rule reads from what `mount_table` made.
    fn facts(table: &str, path: &str) -> crate::VolumeFacts {
        crate::facts_in(table, Path::new(path)).expect("the mount is found")
    }

    #[test]
    fn a_volume_that_may_not_be_looked_at_is_its_own_mount_of_an_unknown_kind() {
        let table = mount_table_with(
            Path::new("/Volumes/GIG/Contents/a.mp3"),
            &Lookups {
                present: &|path| match path.to_str() {
                    Some("/Volumes" | "/") => Ok(()),
                    _ => Err(denied()),
                },
                volume: &|_| Ok(found("/", "apfs", MNT_ROOTFS | MNT_LOCAL)),
            },
        )
        .expect("a line, not an error");
        assert_eq!(table, line(UNKNOWN, Path::new("/Volumes/GIG")));
        let facts = facts(&table, "/Volumes/GIG/Contents/a.mp3");
        assert_eq!(facts.root, Path::new("/Volumes/GIG"));
        assert!(facts.media);
        assert_eq!(facts.removable, None);
        assert!(crate::is_stick(&facts), "unreadable: fails closed");
    }

    #[test]
    fn a_volume_statfs_refuses_is_its_own_mount_of_an_unknown_kind() {
        let table = mount_table_with(
            Path::new("/Volumes/GIG/Contents"),
            &Lookups {
                present: &|_| Ok(()),
                volume: &|_| Err(denied()),
            },
        )
        .expect("a line, not an error");
        assert_eq!(table, line(UNKNOWN, Path::new("/Volumes/GIG")));
    }

    #[test]
    fn a_mount_point_spelled_in_another_case_is_not_taken_for_the_root_volume() {
        let table = mount_table_with(
            Path::new("/Volumes/gig/Contents"),
            &Lookups {
                present: &|_| Ok(()),
                volume: &|_| Ok(found("/Volumes/GIG", "exfat", MNT_REMOVABLE | MNT_LOCAL)),
            },
        )
        .expect("a line, not an error");
        assert_eq!(table, line(UNKNOWN, Path::new("/Volumes/gig")));
        let facts = facts(&table, "/Volumes/gig/Contents");
        assert!(facts.media && facts.removable.is_none());
    }

    #[test]
    fn a_refused_path_outside_volumes_is_not_taken_for_the_root_volume_either() {
        let table = mount_table_with(
            Path::new("/mnt/stick/Contents"),
            &Lookups {
                present: &|path| match path.to_str() {
                    Some("/mnt" | "/") => Ok(()),
                    _ => Err(denied()),
                },
                volume: &|_| Ok(found("/", "apfs", MNT_ROOTFS | MNT_LOCAL)),
            },
        )
        .expect("a line, not an error");
        assert_eq!(table, line(UNKNOWN, Path::new("/mnt/stick")));
        let facts = facts(&table, "/mnt/stick/Contents");
        assert!(crate::is_stick(&facts), "unreadable and may be removable");
    }

    #[test]
    fn a_path_not_there_yet_is_on_its_nearest_existing_ancestors_volume() {
        let asked = std::cell::RefCell::new(Vec::new());
        let table = mount_table_with(
            Path::new("/Volumes/GIG/new/folder"),
            &Lookups {
                present: &|path| match path.to_str() {
                    Some("/Volumes/GIG/new/folder" | "/Volumes/GIG/new") => Err(missing()),
                    _ => Ok(()),
                },
                volume: &|path| {
                    asked.borrow_mut().push(path.to_path_buf());
                    Ok(found("/Volumes/GIG", "exfat", MNT_REMOVABLE | MNT_LOCAL))
                },
            },
        )
        .expect("a line, not an error");
        assert_eq!(*asked.borrow(), [PathBuf::from("/Volumes/GIG")]);
        assert_eq!(table, line(REMOVABLE, Path::new("/Volumes/GIG")));
    }

    #[test]
    fn the_systems_data_volume_is_the_root_volume_as_far_as_paths_go() {
        let table = mount_table_with(
            Path::new("/Users/dj/Music"),
            &Lookups {
                present: &|_| Ok(()),
                volume: &|_| Ok(found("/System/Volumes/Data", "apfs", MNT_LOCAL)),
            },
        )
        .expect("a line, not an error");
        assert_eq!(table, line(FIXED, Path::new("/")));
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
