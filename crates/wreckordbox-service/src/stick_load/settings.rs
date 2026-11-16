//! The deck's settings files a load gives a Stick (wbx-8z2).
//!
//! A deck takes its colours, columns and fields from five files in the
//! Stick's export folder: `DEVSETTING.DAT`, `MYSETTING.DAT`,
//! `MYSETTING2.DAT`, `DJMMYSETTING.DAT` and `djprofile.nxs`. A Stick loaded
//! without them showed the deck's defaults; copying the five from the DJ's
//! Gig stick restored everything (confirmed on the deck 2026-11-15).
//!
//! Where they come from: the Library's **My Settings**, the folder the
//! window keeps beside its state folder (`usb-settings`), which
//! `library rebuild-from-stick` fills with all five from the Stick it
//! rebuilds from (the window's Round-trip import brings only the three
//! `*MYSETTING*.DAT` files). Without one there, `DEVSETTING.DAT` is rekordbox's
//! own defaults (as every rekordbox export writes one) and the others are
//! reported as not written, never invented.
//!
//! A file the Stick already holds is the DJ's own, set on a player: it is
//! kept unless the load was asked to replace the Stick's settings (and is
//! then replaced only from My Settings, never with defaults). The files go
//! into the export's own publication (`ExportOptions::settings_files`), so
//! they are written with the export or not at all. The plan id is bound to
//! each file's action and bytes, so a load cannot write other settings than
//! its dry run reported.

use std::path::{Path, PathBuf};

use super::SettingsLine;

/// The five files, in the order a plan lists them.
pub(crate) const FILES: [&str; 5] = [
    "DEVSETTING.DAT",
    "MYSETTING.DAT",
    "MYSETTING2.DAT",
    "DJMMYSETTING.DAT",
    "djprofile.nxs",
];

/// The largest `djprofile.nxs` taken; the deck's is a few kilobytes.
const PROFILE_LIMIT: usize = 1 << 20;

/// The Library's My Settings: beside the state folder, as the window keeps
/// them (`usb_import::settings_stash`).
pub(crate) fn my_settings(state_dir: &Path) -> PathBuf {
    state_dir.parent().unwrap_or(state_dir).join("usb-settings")
}

/// What a load does with the settings files.
#[derive(Debug, Clone, Default)]
pub(crate) struct PlannedSettings {
    pub lines: Vec<SettingsLine>,
    /// The files written (`write` and `replace`), relative to the export
    /// folder, with their bytes.
    pub files: Vec<(String, Vec<u8>)>,
}

/// CRC-16/XMODEM, which the players check.
fn xmodem(bytes: &[u8]) -> u16 {
    let mut crc = 0u16;
    for byte in bytes {
        crc ^= u16::from(*byte) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 == 0 {
                crc << 1
            } else {
                (crc << 1) ^ 0x1021
            };
        }
    }
    crc
}

/// Why `bytes` is not a file of `name` a deck can take, if it is not.
/// The `.DAT` files: the 104-byte header whose first word is 96 and whose
/// last is the body's length, the body, its checksum (`DJMMYSETTING.DAT`'s
/// covers the header too) and two bytes (as Round-trip import checks them).
pub(crate) fn damaged(name: &str, bytes: &[u8]) -> Option<String> {
    let broken = |why: &str| Some(format!("{name} in My Settings is damaged ({why})"));
    if name == "djprofile.nxs" {
        return match bytes.len() {
            0 => broken("it is empty"),
            n if n > PROFILE_LIMIT => broken("it is too large"),
            _ => None,
        };
    }
    if bytes.len() < 108 || bytes.len() > 4096 || bytes[0..4] != [96, 0, 0, 0] {
        return broken("wrong length");
    }
    let size = u32::from_le_bytes([bytes[100], bytes[101], bytes[102], bytes[103]]) as usize;
    if size + 108 != bytes.len() {
        return broken("wrong length");
    }
    let end = 104 + size;
    let covered = if name == "DJMMYSETTING.DAT" {
        &bytes[..end]
    } else {
        &bytes[104..end]
    };
    if u16::from_le_bytes([bytes[end], bytes[end + 1]]) != xmodem(covered) {
        return broken("its checksum does not match");
    }
    None
}

/// What the Library has for `name`: its bytes, or why there are none.
fn from_library(state_dir: &Path, name: &str) -> Result<Vec<u8>, String> {
    let path = my_settings(state_dir).join(name);
    match std::fs::read(&path) {
        Ok(bytes) => damaged(name, &bytes).map_or(Ok(bytes), Err),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(format!(
            "the Library has no {name} in its My Settings: carry them from a Stick with `wreckordbox library rebuild-from-stick`{}",
            // Round-trip brings these three and no other.
            if matches!(name, "MYSETTING.DAT" | "MYSETTING2.DAT" | "DJMMYSETTING.DAT") {
                " or the window's Round-trip import"
            } else {
                ""
            }
        )),
        Err(e) => Err(format!("{name} in My Settings could not be read: {e}")),
    }
}

/// Works out what a load does with each file on the Stick at `root`
/// (read only: whether each is there).
pub(crate) fn plan(state_dir: &Path, root: &Path, replace: bool) -> PlannedSettings {
    let export = rbl_export::export_root(root);
    let mut planned = PlannedSettings::default();
    for name in FILES {
        let on_stick = export.join(name).exists();
        let source = from_library(state_dir, name);
        let line = |action: &str, from: Option<&str>, reason: Option<String>| SettingsLine {
            file: name.to_owned(),
            action: action.to_owned(),
            from: from.map(str::to_owned),
            reason,
        };
        let line = match (on_stick, source) {
            (true, _) if !replace => line("keep", None, None),
            (true, Ok(bytes)) => {
                planned.files.push((name.to_owned(), bytes));
                line("replace", Some("my_settings"), None)
            }
            (true, Err(why)) => line("keep", None, Some(format!("not replaced: {why}"))),
            (false, Ok(bytes)) => {
                planned.files.push((name.to_owned(), bytes));
                line("write", Some("my_settings"), None)
            }
            (false, Err(why)) if name == "DEVSETTING.DAT" => {
                planned.files.push((
                    name.to_owned(),
                    rbl_devices::settings::DevSetting::default().encode(),
                ));
                // My Settings has one that cannot be used: say so.
                let unusable = my_settings(state_dir).join(name).exists();
                line("write", Some("defaults"), unusable.then_some(why))
            }
            (false, Err(why)) => line("unavailable", None, Some(why)),
        };
        planned.lines.push(line);
    }
    planned
}

/// One line per file: what a plan id is bound to (the action, and the
/// bytes of what is written).
pub(crate) fn bound(planned: &PlannedSettings) -> Vec<String> {
    planned
        .lines
        .iter()
        .map(|line| {
            let bytes = planned.files.iter().find(|(name, _)| *name == line.file);
            match bytes {
                Some((_, bytes)) => format!(
                    "{} {} from {} {:016x}",
                    line.action,
                    line.file,
                    line.from.as_deref().unwrap_or(""),
                    rbl_export::manifest::hash(bytes)
                ),
                None => format!("{} {}", line.action, line.file),
            }
        })
        .collect()
}

/// Of the files planned to be written, those the Stick at `root` now holds
/// byte for byte, and those it does not.
pub(crate) fn read_back(root: &Path, planned: &PlannedSettings) -> (Vec<String>, Vec<String>) {
    let export = rbl_export::export_root(root);
    let (mut written, mut not_written) = (Vec::new(), Vec::new());
    for (name, bytes) in &planned.files {
        if std::fs::read(export.join(name)).is_ok_and(|on_stick| on_stick == *bytes) {
            written.push(name.clone());
        } else {
            not_written.push(name.clone());
        }
    }
    (written, not_written)
}

/// One of the five files as a Stick holds it: its bytes, or why it cannot
/// be taken (`None`: the Stick has none).
pub(crate) type OnStick = (String, Option<Result<Vec<u8>, String>>);

/// Reads the five files from the Stick at `root` (its resolved root, inside
/// a non-recovering Stick read): only files that are really on the Stick.
pub(crate) fn read_from_stick(root: &Path) -> Vec<OnStick> {
    let export = rbl_export::export_root(root);
    FILES
        .iter()
        .map(|name| {
            let path = export.join(name);
            let found = path
                .canonicalize()
                .ok()
                .filter(|real| real.starts_with(root))
                .map(|real| {
                    let bytes = std::fs::read(real)
                        .map_err(|e| format!("{name} on the Stick could not be read: {e}"))?;
                    damaged(name, &bytes)
                        .map(|why| why.replace("in My Settings", "on the Stick"))
                        .map_or(Ok(bytes), Err)
                });
            ((*name).to_owned(), found)
        })
        .collect()
}

/// What `library.rebuild_from_stick` does with each file: `carry` (into
/// the Library's My Settings, replacing what is there), `absent` (the Stick
/// has none; My Settings keeps its own), `damaged` (not carried).
pub(crate) fn carry_lines(found: &[OnStick]) -> Vec<SettingsLine> {
    found
        .iter()
        .map(|(name, found)| {
            let (action, reason) = match found {
                Some(Ok(_)) => ("carry", None),
                None => ("absent", None),
                Some(Err(why)) => ("damaged", Some(why.clone())),
            };
            SettingsLine {
                file: name.clone(),
                action: action.to_owned(),
                from: found.as_ref().map(|_| "stick".to_owned()),
                reason,
            }
        })
        .collect()
}

/// Puts each file `carry_lines` carries into the Library's My Settings.
/// Not a Library edit and not a Stick write: the window's own folder.
pub(crate) fn carry(state_dir: &Path, found: &[OnStick]) -> std::io::Result<()> {
    let folder = my_settings(state_dir);
    for (name, found) in found {
        if let Some(Ok(bytes)) = found {
            rbl_core::durable::create_dir_all(&folder)?;
            rbl_core::durable::write(&folder.join(name), bytes)?;
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn rekordbox_defaults_are_a_file_a_deck_takes() {
        let dev = rbl_devices::settings::DevSetting::default().encode();
        assert_eq!(damaged("DEVSETTING.DAT", &dev), None);
        let mut broken = dev;
        broken[0x70] ^= 1;
        assert!(damaged("DEVSETTING.DAT", &broken).is_some());
    }
}
