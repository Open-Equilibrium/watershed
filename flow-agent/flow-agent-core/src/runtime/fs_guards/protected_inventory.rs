use super::{AnchoredDir, DirectoryErrorMode, directory_lease, path_io_error};
use crate::runtime::types::RuntimeError;
use cap_fs_ext::MetadataExt as _;
use std::collections::{BTreeMap, BTreeSet};

/// Inspect names and metadata only, while cooperating publishers cannot change names.
pub(crate) fn verify_protected_aliases(
    roots: &[AnchoredDir],
    images: &[(&super::AnchoredFile, &std::fs::File)],
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

    let image_metadata = images
        .iter()
        .map(|(name, image)| verified_image_metadata(name, image))
        .collect::<Result<Vec<_>, _>>()?;
    let mut directories = BTreeSet::new();
    let mut aliases = BTreeMap::new();
    let mut count_alias = |metadata: &cap_std::fs::Metadata,
                           path: &std::path::Path,
                           new_name: bool|
     -> Result<(), RuntimeError> {
        if metadata.nlink() > 1 {
            let (expected, covered, first_path) = aliases
                .entry((metadata.dev(), metadata.ino()))
                .or_insert_with(|| (metadata.nlink(), 0_u64, path.to_owned()));
            if *expected != metadata.nlink() {
                return Err(RuntimeError::Protocol(format!(
                    "{} protected file aliases changed during admission",
                    first_path.display()
                )));
            }
            *covered += u64::from(new_name);
        }
        Ok(())
    };
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
                count_alias(&metadata, &path, true)?;
            } else {
                return Err(RuntimeError::Protocol(format!(
                    "{} protected inventory contains an unsupported file type",
                    path.display()
                )));
            }
        }
    }
    let mut selected_names = BTreeSet::new();
    for ((name, image), before) in images.iter().zip(image_metadata) {
        let metadata = verified_image_metadata(name, image)?;
        if (before.dev(), before.ino(), before.nlink())
            != (metadata.dev(), metadata.ino(), metadata.nlink())
        {
            return Err(RuntimeError::Protocol(format!(
                "{} protected image changed during admission",
                name.path.display()
            )));
        }
        let parent = name.parent.identity()?;
        let parent_identity = (parent.device, parent.inode);
        // Traversed parents already contributed every direct-child name.
        let new_name = !directories.contains(&parent_identity)
            && selected_names.insert((parent_identity, &name.leaf));
        count_alias(&metadata, &name.path, new_name)?;
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

fn verified_image_metadata(
    name: &super::AnchoredFile,
    image: &std::fs::File,
) -> Result<cap_std::fs::Metadata, RuntimeError> {
    use std::os::unix::fs::MetadataExt;

    if name.leaf.file_name() != Some(name.leaf.as_os_str()) {
        return Err(RuntimeError::Protocol(format!(
            "{} protected image must be an anchored direct-child name",
            name.path.display()
        )));
    }
    let metadata = name.metadata()?;
    let retained = image
        .metadata()
        .map_err(|source| path_io_error(&name.path, source))?;
    if !metadata.is_file()
        || !retained.is_file()
        || metadata.nlink() == 0
        || (metadata.dev(), metadata.ino(), metadata.nlink())
            != (
                MetadataExt::dev(&retained),
                MetadataExt::ino(&retained),
                MetadataExt::nlink(&retained),
            )
    {
        return Err(RuntimeError::Protocol(format!(
            "{} protected image name changed or is not a linked regular file",
            name.path.display()
        )));
    }
    Ok(metadata)
}
