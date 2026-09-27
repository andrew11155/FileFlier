//! Drives through UDisks2 (the service GNOME Files, Dolphin and Nemo use): lists
//! volumes that aren't mounted yet, mounts them, and safely ejects removable drives.

use std::collections::HashMap;
use std::path::PathBuf;

use zbus::zvariant::{OwnedObjectPath, OwnedValue};

const DEST: &str = "org.freedesktop.UDisks2";

#[derive(Clone, Debug, PartialEq)]
pub struct Volume {
    /// D-Bus object path of the block device.
    pub object: String,
    pub label: String,
    pub size: u64,
    pub mount_points: Vec<PathBuf>,
    /// On a removable/ejectable drive (USB sticks, SD cards, phones).
    pub removable: bool,
    pub drive: Option<String>,
    pub can_eject: bool,
    pub can_power_off: bool,
}

/// Plain data pulled out of UDisks' object tree (kept separate so filtering is testable).
#[derive(Default, Debug, Clone)]
pub struct RawBlock {
    pub object: String,
    pub label: String,
    pub usage: String,
    pub size: u64,
    pub hint_ignore: bool,
    pub hint_system: bool,
    pub hint_name: String,
    pub drive: String,
    pub has_fs: bool,
    pub mount_points: Vec<PathBuf>,
}

#[derive(Default, Debug, Clone)]
pub struct RawDrive {
    pub removable: bool,
    pub ejectable: bool,
    pub can_power_off: bool,
}

/// Which volumes to show, the way desktop file managers do: real filesystems that
/// aren't hidden, and not system partitions unless they're on removable media.
pub fn volumes(blocks: &[RawBlock], drives: &HashMap<String, RawDrive>) -> Vec<Volume> {
    let mut out: Vec<Volume> = blocks
        .iter()
        .filter(|b| b.has_fs && b.usage == "filesystem" && !b.hint_ignore && b.size > 0)
        .filter_map(|b| {
            let d = drives.get(&b.drive);
            let removable = d.is_some_and(|d| d.removable || d.ejectable);
            if b.hint_system && !removable {
                return None;
            }
            let label = [&b.hint_name, &b.label]
                .into_iter()
                .find(|s| !s.trim().is_empty())
                .cloned()
                .unwrap_or_else(|| format!("{} Volume", crate::app::human_size(b.size)));
            Some(Volume {
                object: b.object.clone(),
                label,
                size: b.size,
                mount_points: b.mount_points.clone(),
                removable,
                drive: (!b.drive.is_empty() && b.drive != "/").then(|| b.drive.clone()),
                can_eject: d.is_some_and(|d| d.ejectable),
                can_power_off: d.is_some_and(|d| d.can_power_off),
            })
        })
        .collect();
    out.sort_by_key(|a| a.label.to_lowercase());
    out
}

type Props = HashMap<String, OwnedValue>;
type Objects = HashMap<OwnedObjectPath, HashMap<String, Props>>;

fn get<T: TryFrom<OwnedValue>>(p: &Props, k: &str) -> Option<T> {
    p.get(k)?.try_clone().ok()?.try_into().ok()
}

fn bytes_to_string(b: Vec<u8>) -> String {
    String::from_utf8_lossy(b.strip_suffix(&[0]).unwrap_or(&b)).into_owned()
}

fn connect() -> Result<zbus::blocking::Connection, String> {
    zbus::blocking::Connection::system().map_err(|e| format!("Couldn't reach the system bus: {e}"))
}

/// All volumes UDisks knows about, or an error if UDisks isn't available.
pub fn list() -> Result<Vec<Volume>, String> {
    let conn = connect()?;
    let proxy =
        zbus::blocking::Proxy::new(&conn, DEST, "/org/freedesktop/UDisks2", "org.freedesktop.DBus.ObjectManager")
            .map_err(|e| e.to_string())?;
    let objects: Objects = proxy.call("GetManagedObjects", &()).map_err(|e| e.to_string())?;
    let mut blocks = Vec::new();
    let mut drives = HashMap::new();
    for (path, ifaces) in &objects {
        if let Some(d) = ifaces.get("org.freedesktop.UDisks2.Drive") {
            drives.insert(
                path.to_string(),
                RawDrive {
                    removable: get(d, "Removable").unwrap_or(false) || get(d, "MediaRemovable").unwrap_or(false),
                    ejectable: get(d, "Ejectable").unwrap_or(false),
                    can_power_off: get(d, "CanPowerOff").unwrap_or(false),
                },
            );
        }
        let Some(b) = ifaces.get("org.freedesktop.UDisks2.Block") else { continue };
        let fs = ifaces.get("org.freedesktop.UDisks2.Filesystem");
        let mount_points: Vec<PathBuf> = fs
            .and_then(|f| get::<Vec<Vec<u8>>>(f, "MountPoints"))
            .unwrap_or_default()
            .into_iter()
            .map(|m| PathBuf::from(bytes_to_string(m)))
            .collect();
        blocks.push(RawBlock {
            object: path.to_string(),
            label: get(b, "IdLabel").unwrap_or_default(),
            usage: get(b, "IdUsage").unwrap_or_default(),
            size: get(b, "Size").unwrap_or(0),
            hint_ignore: get(b, "HintIgnore").unwrap_or(false),
            hint_system: get(b, "HintSystem").unwrap_or(false),
            hint_name: get(b, "HintName").unwrap_or_default(),
            drive: get::<OwnedObjectPath>(b, "Drive").map(|o| o.to_string()).unwrap_or_default(),
            has_fs: fs.is_some(),
            mount_points,
        });
    }
    Ok(volumes(&blocks, &drives))
}

fn options() -> HashMap<&'static str, zbus::zvariant::Value<'static>> {
    HashMap::new()
}

/// Mounts a volume; returns where it was mounted. The desktop may ask for a password.
pub fn mount(object: &str) -> Result<PathBuf, String> {
    let conn = connect()?;
    let proxy = zbus::blocking::Proxy::new(&conn, DEST, object, "org.freedesktop.UDisks2.Filesystem")
        .map_err(|e| e.to_string())?;
    let path: String = proxy.call("Mount", &(options(),)).map_err(|e| nice_error(&e.to_string()))?;
    Ok(PathBuf::from(path))
}

/// Unmounts every volume on the drive, then ejects or powers it off so it's safe to unplug.
pub fn eject(vol: &Volume, all: &[Volume]) -> Result<(), String> {
    let conn = connect()?;
    let siblings: Vec<&Volume> = match &vol.drive {
        Some(d) => all.iter().filter(|v| v.drive.as_ref() == Some(d)).collect(),
        None => vec![vol],
    };
    for v in siblings.iter().filter(|v| !v.mount_points.is_empty()) {
        let fs = zbus::blocking::Proxy::new(&conn, DEST, v.object.as_str(), "org.freedesktop.UDisks2.Filesystem")
            .map_err(|e| e.to_string())?;
        let () = fs.call("Unmount", &(options(),)).map_err(|e| nice_error(&e.to_string()))?;
    }
    if let Some(d) = &vol.drive {
        let drive = zbus::blocking::Proxy::new(&conn, DEST, d.as_str(), "org.freedesktop.UDisks2.Drive")
            .map_err(|e| e.to_string())?;
        if vol.can_eject {
            let _: Result<(), _> = drive.call("Eject", &(options(),));
        } else if vol.can_power_off {
            let _: Result<(), _> = drive.call("PowerOff", &(options(),));
        }
    }
    Ok(())
}

fn nice_error(e: &str) -> String {
    if e.contains("target is busy") || e.contains("DeviceBusy") {
        "The drive is in use. Close any files or apps using it, then try again.".into()
    } else if e.contains("NotAuthorized") {
        "Not allowed to do that (the password prompt was cancelled or denied).".into()
    } else if e.contains("AlreadyMounted") {
        "It's already mounted.".into()
    } else {
        e.trim_start_matches("org.freedesktop.UDisks2.Error.Failed: ").to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_like_a_file_manager() {
        let blk = |object: &str, label: &str, system: bool, drive: &str| RawBlock {
            object: object.into(),
            label: label.into(),
            usage: "filesystem".into(),
            size: 32_000_000_000,
            hint_system: system,
            drive: drive.into(),
            has_fs: true,
            ..Default::default()
        };
        let blocks = vec![
            blk("/b/nvme0n1p2", "", true, "/d/nvme"),       // system root: hidden
            blk("/b/sda1", "USB STICK", true, "/d/usb"),    // system hint, but removable: shown
            blk("/b/nvme0n1p3", "Games", false, "/d/nvme"), // extra internal partition: shown
            RawBlock { usage: "other".into(), ..blk("/b/swap", "swap", false, "/d/nvme") }, // swap: hidden
            RawBlock { hint_ignore: true, ..blk("/b/efi", "EFI", false, "/d/nvme") }, // ignored
            blk("/b/sdb1", "", false, "/d/usb"),            // no label: size name
        ];
        let mut drives = HashMap::new();
        drives.insert("/d/usb".to_string(), RawDrive { removable: true, ejectable: true, ..Default::default() });
        drives.insert("/d/nvme".to_string(), RawDrive::default());
        let v = volumes(&blocks, &drives);
        let labels: Vec<&str> = v.iter().map(|v| v.label.as_str()).collect();
        assert_eq!(labels, vec!["32 GB Volume", "Games", "USB STICK"]);
        assert!(v.iter().find(|v| v.label == "USB STICK").unwrap().can_eject);
        assert!(!v.iter().find(|v| v.label == "Games").unwrap().removable);
    }
}
