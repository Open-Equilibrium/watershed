use super::{AnchoredDir, DirectoryErrorMode, directory_lease, path_io_error};
use crate::runtime::types::RuntimeError;
use cap_fs_ext::MetadataExt as _;
use std::collections::{BTreeMap, BTreeSet};

/// Inspect names and metadata only, while cooperating publishers cannot change names.
pub(crate) fn verify_protected_directory_aliases(
    roots: &[AnchoredDir],
) -> Result<(), RuntimeError> {
    let mut ordered = BTreeMap::new();
    for root in roots {
        root.validate_private()?;
        let identity = root.identity()?;
        ordered
            .entry((identity.device, identity.inode))
            .or_insert(root);
    }
    let _leases = ordered
        .values()
        .map(|root| {
            directory_lease(&root.dir, false).map_err(|source| path_io_error(&root.path, source))
        })
        .collect::<Result<Vec<_>, _>>()?;

    let mut directories = BTreeSet::new();
    let mut aliases = BTreeMap::new();
    for root in ordered.values() {
        let identity = root.identity()?;
        if !directories.insert((identity.device, identity.inode)) {
            continue;
        }
        let mut pending = vec![(
            (*root).clone(),
            root.dir
                .entries()
                .map_err(|source| path_io_error(&root.path, source))?,
        )];
        while let Some((parent, entries)) = pending.last_mut() {
            let Some(entry) = entries.next() else {
                pending.pop();
                continue;
            };
            let entry = entry.map_err(|source| path_io_error(&parent.path, source))?;
            let leaf = entry.file_name();
            let path = parent.path.join(&leaf);
            let metadata = parent
                .dir
                .symlink_metadata(&leaf)
                .map_err(|source| path_io_error(&path, source))?;
            let identity = (metadata.dev(), metadata.ino());
            if metadata.is_dir() {
                let child = parent
                    .child(&leaf, false, DirectoryErrorMode::Protocol)?
                    .ok_or_else(|| {
                        RuntimeError::Protocol(format!(
                            "{} protected directory disappeared during admission",
                            path.display()
                        ))
                    })?;
                let opened = child.identity()?;
                if (opened.device, opened.inode) != identity {
                    return Err(RuntimeError::Protocol(format!(
                        "{} protected directory changed during admission",
                        path.display()
                    )));
                }
                if directories.insert(identity) {
                    let entries = child
                        .dir
                        .entries()
                        .map_err(|source| path_io_error(&path, source))?;
                    pending.push((child, entries));
                }
            } else if metadata.is_file() && metadata.nlink() > 0 {
                if metadata.nlink() > 1 {
                    let (expected, covered, first_path) =
                        aliases
                            .entry(identity)
                            .or_insert((metadata.nlink(), 0_u64, path));
                    if *expected != metadata.nlink() {
                        return Err(RuntimeError::Protocol(format!(
                            "{} protected file aliases changed during admission",
                            first_path.display()
                        )));
                    }
                    *covered += 1;
                }
            } else {
                return Err(RuntimeError::Protocol(format!(
                    "{} protected inventory contains an unsupported file type",
                    path.display()
                )));
            }
        }
    }
    for (_, (expected, covered, path)) in aliases {
        if expected != covered {
            return Err(RuntimeError::Protocol(format!(
                "{} has file aliases outside the protected set or an incomplete inventory; remove outside aliases manually before starting Tools",
                path.display()
            )));
        }
    }
    Ok(())
}
