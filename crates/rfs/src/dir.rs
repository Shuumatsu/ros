//! Directories and path resolution — the "tree of names" layer.
//!
//! A directory is just a file whose bytes are an array of fixed-size
//! [`DirEntry`] records (`DESIGN.md` §5). So every operation here is built on the
//! inode `read_at`/`write_at` from [`crate::fs`]: listing a directory reads its
//! entries, adding a name writes one. A slot with `inode == 0` is free — inode 0
//! is the root, which never appears as a child, so 0 is a safe "empty" marker.
//!
//! Path resolution (`DESIGN.md` §6) is then just: start at the root inode and,
//! for each `/`-separated component, look it up in the current directory. There
//! is no `.` or `..` — this filesystem has no special directory entries yet.

use alloc::vec::Vec;

use crate::fs::Fs;
use crate::layout::{DirEntry, InodeType, NAME_MAX};

/// Size of one on-disk directory entry.
const DIRENT_SIZE: usize = core::mem::size_of::<DirEntry>();

/// Whether `name` is a usable single path component.
fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= NAME_MAX && !name.contains('/')
}

impl Fs {
    // -------------------------------------------------------------- lookup / ls

    /// Look up `name` directly inside directory `dir`, returning its inode.
    pub fn dir_lookup(&self, dir: u32, name: &str) -> Option<u32> {
        debug_assert_eq!(
            self.inode_type(dir),
            Some(InodeType::Dir),
            "dir_lookup on a non-directory"
        );
        self.find_entry(dir, name).map(|(_, entry)| entry.inode)
    }

    /// All live entries of directory `dir` (free slots skipped).
    pub fn dir_list(&self, dir: u32) -> Vec<DirEntry> {
        debug_assert_eq!(self.inode_type(dir), Some(InodeType::Dir), "dir_list on a non-directory");
        (0..self.entry_count(dir))
            .map(|idx| self.read_entry(dir, idx))
            .filter(|entry| entry.inode != 0)
            .collect()
    }

    /// Whether directory `dir` has no live entries.
    pub fn dir_is_empty(&self, dir: u32) -> bool {
        debug_assert_eq!(
            self.inode_type(dir),
            Some(InodeType::Dir),
            "dir_is_empty on a non-directory"
        );
        (0..self.entry_count(dir)).all(|idx| self.read_entry(dir, idx).inode == 0)
    }

    /// Resolve an absolute path to an inode, walking one component per `/`.
    /// Redundant, leading and trailing slashes are ignored. `"/"` is the root.
    pub fn resolve(&self, path: &str) -> Option<u32> {
        let mut inode = self.root_inode();
        for component in path.split('/').filter(|c| !c.is_empty()) {
            if self.inode_type(inode) != Some(InodeType::Dir) {
                return None; // a non-directory in the middle of the path
            }
            inode = self.dir_lookup(inode, component)?;
        }
        Some(inode)
    }

    // ------------------------------------------------------------ create/remove

    /// Create `name` of kind `kind` inside directory `dir`, returning the new
    /// inode. Returns `None` if `dir` already has that name, if the name is
    /// invalid, or if the inode table is full.
    pub fn create(&self, dir: u32, name: &str, kind: InodeType) -> Option<u32> {
        debug_assert_eq!(self.inode_type(dir), Some(InodeType::Dir), "create in a non-directory");
        if !valid_name(name) || self.dir_lookup(dir, name).is_some() {
            return None;
        }
        let id = self.alloc_inode(kind)?;
        self.dir_add(dir, name, id);
        Some(id)
    }

    /// Remove `name` from directory `dir`. Directories must be empty. On the
    /// last link the target inode and its data are reclaimed. Returns whether
    /// something was removed.
    pub fn remove(&self, dir: u32, name: &str) -> bool {
        debug_assert_eq!(self.inode_type(dir), Some(InodeType::Dir), "remove from a non-directory");
        let Some((slot, entry)) = self.find_entry(dir, name) else {
            return false;
        };
        let target = entry.inode;
        if self.inode_type(target) == Some(InodeType::Dir) && !self.dir_is_empty(target) {
            return false; // refuse to orphan a non-empty directory's contents
        }
        // Tombstone the slot, then drop a link; reclaim the inode at zero links.
        self.write_entry(dir, slot, &DirEntry::empty());
        let remaining = self.modify_disk_inode(target, |di| {
            di.nlink = di.nlink.saturating_sub(1);
            di.nlink
        });
        if remaining == 0 {
            self.free_inode(target);
        }
        true
    }

    // ------------------------------------------------------------------ internals

    /// Number of entry slots the directory currently spans (live or tombstoned).
    fn entry_count(&self, dir: u32) -> usize { self.inode_size(dir) / DIRENT_SIZE }

    /// Read entry slot `idx`. `read_at` gives us plain bytes at an arbitrary
    /// alignment, so decode with the unaligned reader.
    fn read_entry(&self, dir: u32, idx: usize) -> DirEntry {
        let mut buf = [0u8; DIRENT_SIZE];
        self.read_at(dir, idx * DIRENT_SIZE, &mut buf);
        bytemuck::pod_read_unaligned(&buf)
    }

    /// Write `entry` into slot `idx`, growing the directory if appending.
    fn write_entry(&self, dir: u32, idx: usize, entry: &DirEntry) {
        self.write_at(dir, idx * DIRENT_SIZE, bytemuck::bytes_of(entry));
    }

    /// Find the live entry named `name`, returning its slot index and contents.
    fn find_entry(&self, dir: u32, name: &str) -> Option<(usize, DirEntry)> {
        (0..self.entry_count(dir))
            .map(|idx| (idx, self.read_entry(dir, idx)))
            .find(|(_, entry)| entry.inode != 0 && entry.name() == name)
    }

    /// Append `name -> inode`, reusing the first tombstoned slot if there is one.
    /// Caller must have already ruled out a duplicate.
    fn dir_add(&self, dir: u32, name: &str, inode: u32) {
        assert!(inode != 0, "rfs: inode 0 (root) cannot be a directory child");
        assert!(valid_name(name), "rfs: invalid directory entry name {name:?}");
        let entry = DirEntry::new(inode, name);
        let count = self.entry_count(dir);
        let slot = (0..count).find(|&idx| self.read_entry(dir, idx).inode == 0).unwrap_or(count);
        self.write_entry(dir, slot, &entry);
    }
}

#[cfg(test)]
mod tests {
    use crate::fs::Fs;
    use crate::layout::InodeType;
    use crate::test_support::{format_on, fresh, mount_on, test_ram};
    use alloc::string::{String, ToString};
    use alloc::vec::Vec;
    use pretty_assertions::assert_eq;

    /// Sorted live names of a directory, for order-independent comparison.
    fn names(fs: &Fs, dir: u32) -> Vec<String> {
        let mut v: Vec<String> = fs.dir_list(dir).iter().map(|e| e.name().to_string()).collect();
        v.sort();
        v
    }

    #[test]
    fn create_then_lookup() {
        let fs = fresh();
        let root = fs.root_inode();
        let f = fs.create(root, "hello.txt", InodeType::File).unwrap();
        assert_eq!(fs.dir_lookup(root, "hello.txt"), Some(f));
        assert_eq!(fs.dir_lookup(root, "nope"), None);
        assert_eq!(fs.inode_type(f), Some(InodeType::File));
    }

    #[test]
    fn duplicate_and_invalid_names_rejected() {
        let fs = fresh();
        let root = fs.root_inode();
        assert!(fs.create(root, "a", InodeType::File).is_some());
        assert_eq!(fs.create(root, "a", InodeType::File), None, "duplicate refused");
        assert_eq!(fs.create(root, "", InodeType::File), None, "empty name refused");
        assert_eq!(fs.create(root, "a/b", InodeType::File), None, "name with slash refused");
        let too_long = "x".repeat(64);
        assert_eq!(fs.create(root, &too_long, InodeType::File), None, "over-long name refused");
    }

    #[test]
    fn ls_lists_live_children() {
        let fs = fresh();
        let root = fs.root_inode();
        for n in ["a", "b", "c"] {
            fs.create(root, n, InodeType::File).unwrap();
        }
        assert_eq!(names(&fs, root), ["a", "b", "c"].map(String::from));
    }

    #[test]
    fn nested_directories_and_path_resolution() {
        let fs = fresh();
        let root = fs.root_inode();
        let d = fs.create(root, "d", InodeType::Dir).unwrap();
        let e = fs.create(d, "e", InodeType::Dir).unwrap();
        let f = fs.create(e, "f.txt", InodeType::File).unwrap();

        assert_eq!(fs.resolve("/"), Some(root));
        assert_eq!(fs.resolve("/d"), Some(d));
        assert_eq!(fs.resolve("/d/e"), Some(e));
        assert_eq!(fs.resolve("/d/e/f.txt"), Some(f));
        assert_eq!(fs.resolve("/d/missing"), None);
        assert_eq!(fs.resolve("/d/e/f.txt/x"), None, "cannot descend into a file");
    }

    #[test]
    fn path_ignores_redundant_slashes() {
        let fs = fresh();
        let root = fs.root_inode();
        let d = fs.create(root, "d", InodeType::Dir).unwrap();
        let f = fs.create(d, "f", InodeType::File).unwrap();
        assert_eq!(fs.resolve("//d//f/"), Some(f));
    }

    #[test]
    fn read_write_file_via_resolved_path() {
        let fs = fresh();
        let root = fs.root_inode();
        let d = fs.create(root, "docs", InodeType::Dir).unwrap();
        let f = fs.create(d, "note", InodeType::File).unwrap();
        fs.write_at(f, 0, b"content");

        let id = fs.resolve("/docs/note").unwrap();
        let mut buf = [0u8; 16];
        let n = fs.read_at(id, 0, &mut buf);
        assert_eq!(&buf[..n], b"content");
    }

    #[test]
    fn remove_file_frees_inode() {
        let fs = fresh();
        let root = fs.root_inode();
        let f = fs.create(root, "tmp", InodeType::File).unwrap();
        fs.write_at(f, 0, b"stuff");
        assert!(fs.remove(root, "tmp"));
        assert_eq!(fs.dir_lookup(root, "tmp"), None);
        // Inode reclaimed: the next allocation reuses it.
        assert_eq!(fs.create(root, "again", InodeType::File), Some(f));
    }

    #[test]
    fn cannot_remove_nonempty_directory() {
        let fs = fresh();
        let root = fs.root_inode();
        let d = fs.create(root, "d", InodeType::Dir).unwrap();
        fs.create(d, "child", InodeType::File).unwrap();
        assert!(!fs.remove(root, "d"), "non-empty directory must not be removed");
        assert_eq!(fs.dir_lookup(root, "d"), Some(d), "directory still present");
    }

    #[test]
    fn remove_empty_directory() {
        let fs = fresh();
        let root = fs.root_inode();
        fs.create(root, "d", InodeType::Dir).unwrap();
        assert!(fs.remove(root, "d"));
        assert_eq!(fs.dir_lookup(root, "d"), None);
    }

    #[test]
    fn tombstoned_slot_is_reused_without_growing() {
        let fs = fresh();
        let root = fs.root_inode();
        for n in ["a", "b", "c"] {
            fs.create(root, n, InodeType::File).unwrap();
        }
        let size_before = fs.inode_size(root);

        fs.remove(root, "b");
        fs.create(root, "d", InodeType::File).unwrap();
        assert_eq!(
            fs.inode_size(root),
            size_before,
            "reused the tombstone; directory did not grow"
        );
        assert_eq!(names(&fs, root), ["a", "c", "d"].map(String::from));
    }

    #[test]
    fn tree_survives_reopen() {
        let ram = test_ram();
        {
            let fs = format_on(&ram);
            let d = fs.create(fs.root_inode(), "d", InodeType::Dir).unwrap();
            let f = fs.create(d, "f", InodeType::File).unwrap();
            fs.write_at(f, 0, b"persisted");
            fs.sync();
        }
        let fs = mount_on(&ram);
        let id = fs.resolve("/d/f").unwrap();
        let mut buf = [0u8; 16];
        let n = fs.read_at(id, 0, &mut buf);
        assert_eq!(&buf[..n], b"persisted");
    }
}
