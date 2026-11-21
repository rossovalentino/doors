//! P18: files staged onto a large user disk from the host, through the
//! same storage-core journal, ext2 and catalog code the storage service
//! runs (as the service provisions a new volume: the profile folders of
//! users 1001 and 1002 and /system/bin), so a program's runtime of
//! thousands of files and tens of megabytes is on C: when the guest boots.
//! Every file is written in bounded journal transactions.
use door_storage_core::{
    persistence::{
        Access, Acl, Context, Disposition, Error as VolumeError, OpenOptions, Path as VolumePath,
        PersistentVolume, State, Subject,
    },
    storage::{BlockDevice, Error, Ext2, Geometry, Gpt, Journal, Vfs, WorkingSet},
};
use serde_json::{Value, json};
use std::{
    fs::{File, OpenOptions as FileOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

/// The whole disk file as a block device.
struct Disk {
    file: File,
    sectors: u64,
}
impl BlockDevice for Disk {
    fn geometry(&self) -> Geometry {
        Geometry {
            sector_size: 512,
            sectors: self.sectors,
            read_only: false,
            flush_supported: true,
        }
    }
    fn read_sectors(
        &mut self,
        lba: u64,
        output: &mut [u8],
    ) -> door_storage_core::storage::Result<()> {
        door_storage_core::storage::check_transfer(self.geometry(), lba, output.len())?;
        self.file
            .seek(SeekFrom::Start(lba * 512))
            .and_then(|_| self.file.read_exact(output))
            .map_err(|_| Error::Io)
    }
    fn write_sectors(&mut self, lba: u64, input: &[u8]) -> door_storage_core::storage::Result<()> {
        door_storage_core::storage::check_transfer(self.geometry(), lba, input.len())?;
        self.file
            .seek(SeekFrom::Start(lba * 512))
            .and_then(|_| self.file.write_all(input))
            .map_err(|_| Error::Io)
    }
    fn flush(&mut self) -> door_storage_core::storage::Result<()> {
        self.file.sync_data().map_err(|_| Error::Io)
    }
}

/// One staged item: its native path on the volume (`/compat/...` is C:\)
/// and, for a file, the host file holding its bytes.
pub struct Item {
    pub path: String,
    pub source: Option<PathBuf>,
}

/// Every file under `root` as items below `target` (folders first).
pub fn tree(root: &Path, target: &str) -> Result<Vec<Item>, String> {
    let mut items = vec![Item {
        path: target.to_owned(),
        source: None,
    }];
    let mut stack = vec![(root.to_path_buf(), target.to_owned())];
    while let Some((dir, at)) = stack.pop() {
        let mut entries: Vec<_> = std::fs::read_dir(&dir)
            .map_err(|e| format!("{}: {e}", dir.display()))?
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())?;
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = format!("{at}/{name}");
            if entry.file_type().map_err(|e| e.to_string())?.is_dir() {
                items.push(Item {
                    path: path.clone(),
                    source: None,
                });
                stack.push((entry.path(), path));
            } else {
                items.push(Item {
                    path,
                    source: Some(entry.path()),
                });
            }
        }
    }
    Ok(items)
}

fn volume(e: VolumeError) -> String {
    format!("staging: {e:?}")
}

/// Stages `items` onto the large disk at `disk` (new: provisioned first).
pub fn stage(disk: &Path, items: &[Item]) -> Result<Value, String> {
    let items: Vec<(String, Option<PathBuf>)> = items
        .iter()
        .map(|i| (i.path.clone(), i.source.clone()))
        .collect();
    on_volume(disk, move |v, system| stage_on(v, system, &items))
}

/// A file's bytes on a large disk (the guest's output, read back).
pub fn read_file(disk: &Path, target: &str) -> Result<Vec<u8>, String> {
    let target = target.to_owned();
    on_volume(disk, move |v, system| {
        let (handle, _) = v
            .open(
                system,
                VolumePath::Native(&target),
                Context::root(),
                OpenOptions {
                    access: Access::READ,
                    share: Access::READ,
                    disposition: Disposition::OpenExisting,
                },
            )
            .map_err(|e| format!("{target}: {e:?}"))?;
        let mut out = Vec::new();
        let mut buffer = vec![0u8; 64 * 1024];
        loop {
            let n = v
                .read(&handle, out.len() as u64, &mut buffer)
                .map_err(|e| format!("{target}: {e:?}"))?;
            if n == 0 {
                break;
            }
            out.extend_from_slice(&buffer[..n]);
        }
        v.close(handle).map_err(volume)?;
        Ok(out)
    })
}

type Volume<'a, 'b> = PersistentVolume<'a, Ext2<Journal<'b, &'b mut Disk>>>;

/// Runs `work` on the disk's home volume (provisioned first when new, as
/// the storage service provisions one, with the Windows C: layout).
fn on_volume<T: Send + 'static>(
    path: &Path,
    work: impl FnOnce(&mut Volume<'_, '_>, Subject) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let path = path.to_path_buf();
    // The catalog's state is a third of a MiB: built on a large stack.
    std::thread::Builder::new()
        .stack_size(256 << 20)
        .spawn(move || mount_and(&path, work))
        .map_err(|e| e.to_string())?
        .join()
        .map_err(|_| "staging thread panicked".to_owned())?
}

fn mount_and<T>(
    path: &Path,
    work: impl FnOnce(&mut Volume<'_, '_>, Subject) -> Result<T, String>,
) -> Result<T, String> {
    let file = FileOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let sectors = file.metadata().map_err(|e| e.to_string())?.len() / 512;
    let mut device = Disk { file, sectors };
    let table = Gpt::read(&mut device).map_err(|e| format!("GPT: {e:?}"))?;
    let home = table
        .partitions()
        .find(|p| p.id == *b"DoorP07ExtGUID00")
        .ok_or("large disk without its home partition")?;
    let log = table
        .partitions()
        .find(|p| p.type_guid == *b"DoorRedoJournal1")
        .ok_or("large disk without its journal")?;
    let mut work_set = Box::new(WorkingSet::new());
    let (mut journal, _) = Journal::open(&mut device, home, log, table.disk_id, &mut work_set)
        .map_err(|e| format!("journal: {e:?}"))?;
    journal.begin().map_err(|e| format!("journal: {e:?}"))?;
    let mut fs = Ext2::mount(journal).map_err(|e| format!("home volume: {e:?}"))?;
    let mut state = Box::new(State::production());
    let mut catalog = Box::new([0u8; door_storage_core::persistence::PRODUCTION_CATALOG_BYTES]);
    // The storage manifest's provisioning identity (the system).
    let system = Subject::offline_system();
    let new = fs
        .open(door_storage_core::persistence::CATALOG_NAME)
        .is_err();
    let mut v = if new {
        let mut v =
            PersistentVolume::initialize(fs, &mut state, &mut catalog, system).map_err(volume)?;
        for (uid, path) in [(1001, "/users/1001"), (1002, "/users/1002")] {
            v.mkdir(
                system,
                VolumePath::Native(path),
                Context::root(),
                Some(Acl::owner_full(uid)),
            )
            .map_err(volume)?;
        }
        v.mkdir(
            system,
            VolumePath::Native("/system/bin"),
            Context::root(),
            None,
        )
        .map_err(volume)?;
        v.commit().map_err(volume)?;
        v
    } else {
        PersistentVolume::mount(fs, &mut state, &mut catalog).map_err(volume)?
    };
    // The Windows C: layout first, as the service makes it at mount, so
    // staged folders inherit Program Files' ACL.
    v.ensure_credentials(system).map_err(volume)?;
    v.ensure_windows_layout(system).map_err(volume)?;
    v.commit().map_err(volume)?;
    let result = work(&mut v, system)?;
    v.commit().map_err(volume)?;
    Ok(result)
}

fn stage_on(
    v: &mut Volume<'_, '_>,
    system: Subject,
    items: &[(String, Option<PathBuf>)],
) -> Result<Value, String> {
    let mut files = 0u64;
    let mut bytes = 0u64;
    let mut folders = 0u64;
    let mut buffer = vec![0u8; 64 * 1024];
    for (target, source) in items {
        // Folders on the way, then the item.
        let mut at = String::new();
        let parts: Vec<&str> = target.split('/').filter(|p| !p.is_empty()).collect();
        let last = if source.is_some() {
            parts.len() - 1
        } else {
            parts.len()
        };
        for part in &parts[..last] {
            at.push('/');
            at.push_str(part);
            match v.mkdir(system, VolumePath::Native(&at), Context::root(), None) {
                Ok(_) => folders += 1,
                Err(VolumeError::AlreadyExists) => (),
                Err(e) => return Err(format!("{at}: {e:?}")),
            }
        }
        let Some(source) = source else {
            v.commit().map_err(volume)?;
            continue;
        };
        let (handle, _) = v
            .open(
                system,
                VolumePath::Native(target),
                Context::root(),
                OpenOptions {
                    access: Access::ALL,
                    share: Access(0),
                    disposition: Disposition::CreateAlways,
                },
            )
            .map_err(|e| format!("{target}: {e:?}"))?;
        let mut input = File::open(source).map_err(|e| format!("{}: {e}", source.display()))?;
        let mut offset = 0u64;
        loop {
            let n = input.read(&mut buffer).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            v.write(&handle, offset, &buffer[..n])
                .map_err(|e| format!("{target}: {e:?}"))?;
            offset += n as u64;
            v.commit().map_err(volume)?;
        }
        v.close(handle).map_err(volume)?;
        v.commit().map_err(volume)?;
        files += 1;
        bytes += offset;
    }
    let usage = v.usage();

    Ok(json!({"files":files,"bytes":bytes,"folders":folders,
        "volume_bytes":usage.map(|u| u.0),"free_bytes":usage.map(|u| u.1)}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_large_disk_takes_a_staged_tree_with_long_names_and_large_files() {
        let root = std::env::temp_dir().join(format!("door-stage-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let tree_root = root.join("tree");
        let long = "a".repeat(120) + ".dll";
        std::fs::create_dir_all(tree_root.join("shared").join("Microsoft.NETCore.App")).unwrap();
        let big: Vec<u8> = (0..3_000_000u32).map(|i| (i * 7 + i / 977) as u8).collect();
        std::fs::write(
            tree_root.join("shared/Microsoft.NETCore.App").join(&long),
            &big,
        )
        .unwrap();
        std::fs::write(tree_root.join("dotnet.exe"), b"MZ host").unwrap();
        let disk = root.join("large.raw");
        super::super::large::write(&disk, 1 << 30).unwrap();
        let items = tree(&tree_root, "/compat/Program Files/dotnet").unwrap();
        let report = stage(&disk, &items).unwrap();
        assert_eq!(report["files"], 2);
        assert_eq!(report["bytes"], 3_000_007);
        let path = format!("/compat/Program Files/dotnet/shared/Microsoft.NETCore.App/{long}");
        assert_eq!(read_file(&disk, &path).unwrap(), big);
        assert_eq!(
            read_file(&disk, "/compat/Program Files/dotnet/dotnet.exe").unwrap(),
            b"MZ host"
        );
        // Mounting again keeps everything (the guest's next boot).
        let again = stage(&disk, &[]).unwrap();
        assert_eq!(again["files"], 0);
        assert_eq!(read_file(&disk, &path).unwrap().len(), big.len());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
