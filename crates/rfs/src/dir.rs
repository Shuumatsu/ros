//! Directories and root-based path resolution.
//!
//! Directory entries are dense fixed-size records. Removal moves the last
//! entry into the vacated slot, so entry order is unspecified.

use alloc::vec::Vec;

use crate::fs::Fs;
use crate::layout::{DirEntry, InodeType, NAME_MAX};

const DIRENT_SIZE: usize = core::mem::size_of::<DirEntry>();

fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= NAME_MAX && !name.contains('/')
}

impl Fs {
    pub fn dir_lookup(&self, dir: u32, name: &str) -> Option<u32> {
        debug_assert_eq!(
            self.inode_type(dir),
            Some(InodeType::Dir),
            "dir_lookup on a non-directory"
        );
        self.find_entry(dir, name).map(|(_, entry)| entry.inode)
    }

    /// Returns every entry in unspecified order.
    pub fn dir_list(&self, dir: u32) -> Vec<DirEntry> {
        debug_assert_eq!(self.inode_type(dir), Some(InodeType::Dir), "dir_list on a non-directory");
        (0..self.entry_count(dir)).map(|idx| self.read_entry(dir, idx)).collect()
    }

    pub fn dir_is_empty(&self, dir: u32) -> bool {
        debug_assert_eq!(
            self.inode_type(dir),
            Some(InodeType::Dir),
            "dir_is_empty on a non-directory"
        );
        self.inode_size(dir) == 0
    }

    /// Resolves from the root; empty slash-separated components are ignored.
    pub fn resolve(&self, path: &str) -> Option<u32> {
        let mut inode = self.root_inode();
        for component in path.split('/').filter(|c| !c.is_empty()) {
            if self.inode_type(inode) != Some(InodeType::Dir) {
                return None;
            }
            inode = self.dir_lookup(inode, component)?;
        }
        Some(inode)
    }

    /// Create `name` of kind `kind` inside directory `dir`, returning the new
    /// inode. Returns `None` if `dir` already has that name, if the name is
    /// invalid, or if the inode table is full.
    pub fn create(&self, dir: u32, name: &str, kind: InodeType) -> Option<u32> {
        debug_assert_eq!(self.inode_type(dir), Some(InodeType::Dir), "create in a non-directory");
        if !valid_name(name) || self.dir_lookup(dir, name).is_some() {
            return None;
        }
        let id = self.alloc_inode(kind)?;
        self.append_entry(dir, name, id);
        Some(id)
    }

    /// Adds a hard link to a file. Directory links are refused.
    pub fn link(&self, dir: u32, name: &str, target: u32) -> bool {
        debug_assert_eq!(self.inode_type(dir), Some(InodeType::Dir), "link in a non-directory");
        if self.inode_type(target) != Some(InodeType::File) {
            return false;
        }
        if !valid_name(name) || self.dir_lookup(dir, name).is_some() {
            return false;
        }
        // Publish ownership before the name so interruption can leak but cannot dangle.
        self.modify_disk_inode(target, |di| di.nlink += 1);
        self.append_entry(dir, name, target);
        true
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
            return false;
        }
        // Copy before shrinking so interruption can duplicate a name but cannot lose one.
        let last = self.entry_count(dir) - 1;
        if slot != last {
            let moved = self.read_entry(dir, last);
            self.write_entry(dir, slot, &moved);
        }
        self.set_len(dir, last * DIRENT_SIZE);
        let remaining = self.modify_disk_inode(target, |di| {
            di.nlink = di.nlink.saturating_sub(1);
            di.nlink
        });
        if remaining == 0 {
            self.free_inode(target);
        }
        true
    }

    fn entry_count(&self, dir: u32) -> usize { self.inode_size(dir) / DIRENT_SIZE }

    fn read_entry(&self, dir: u32, idx: usize) -> DirEntry {
        let mut buf = [0u8; DIRENT_SIZE];
        self.read_at(dir, idx * DIRENT_SIZE, &mut buf);
        bytemuck::pod_read_unaligned(&buf)
    }

    fn write_entry(&self, dir: u32, idx: usize, entry: &DirEntry) {
        self.write_at(dir, idx * DIRENT_SIZE, bytemuck::bytes_of(entry));
    }

    fn find_entry(&self, dir: u32, name: &str) -> Option<(usize, DirEntry)> {
        (0..self.entry_count(dir))
            .map(|idx| (idx, self.read_entry(dir, idx)))
            .find(|(_, entry)| entry.name() == name)
    }

    fn append_entry(&self, dir: u32, name: &str, inode: u32) {
        assert!(valid_name(name), "rfs: invalid directory entry name {name:?}");
        self.write_entry(dir, self.entry_count(dir), &DirEntry::new(inode, name));
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
    fn removing_a_middle_entry_compacts_and_shrinks() {
        let fs = fresh();
        let root = fs.root_inode();
        for n in ["a", "b", "c", "d"] {
            fs.create(root, n, InodeType::File).unwrap();
        }
        let size_before = fs.inode_size(root);

        assert!(fs.remove(root, "b"));
        assert_eq!(fs.inode_size(root), size_before - 32, "one record shorter");
        assert_eq!(names(&fs, root), ["a", "c", "d"].map(String::from), "survivors all reachable");
        assert_eq!(fs.dir_list(root).len(), 3, "no free slots to skip");

        fs.create(root, "e", InodeType::File).unwrap();
        assert_eq!(fs.inode_size(root), size_before, "back to four records");
        assert_eq!(names(&fs, root), ["a", "c", "d", "e"].map(String::from));
    }

    #[test]
    fn removing_the_last_entry_needs_no_move() {
        let fs = fresh();
        let root = fs.root_inode();
        for n in ["a", "b"] {
            fs.create(root, n, InodeType::File).unwrap();
        }
        assert!(fs.remove(root, "b"));
        assert_eq!(names(&fs, root), ["a"].map(String::from));
        assert_eq!(fs.inode_size(root), 32);
    }

    #[test]
    fn a_drained_directory_costs_nothing() {
        let fs = fresh();
        let root = fs.root_inode();
        let d = fs.create(root, "d", InodeType::Dir).unwrap();
        for n in ["x", "y", "z"] {
            fs.create(d, n, InodeType::File).unwrap();
        }
        for n in ["y", "x", "z"] {
            assert!(fs.remove(d, n));
        }
        assert_eq!(fs.inode_size(d), 0, "an emptied directory is size 0");
        assert!(fs.dir_is_empty(d));
        assert_eq!(fs.dir_list(d).len(), 0);
        assert!(fs.remove(root, "d"), "and it can then be removed");
    }

    #[test]
    fn directory_spanning_blocks_survives_churn() {
        let fs = fresh();
        let root = fs.root_inode();
        let d = fs.create(root, "d", InodeType::Dir).unwrap();
        let all: Vec<String> = (0..40).map(|i| alloc::format!("f{i}")).collect();
        for n in &all {
            fs.create(d, n, InodeType::File).unwrap();
        }
        assert_eq!(fs.inode_size(d), 40 * 32);

        let gone: Vec<String> = all.iter().step_by(2).cloned().collect();
        let mut kept: Vec<String> = all.iter().skip(1).step_by(2).cloned().collect();
        for n in &gone {
            assert!(fs.remove(d, n), "removing {n}");
        }

        kept.sort();
        assert_eq!(names(&fs, d), kept, "every survivor still resolves");
        assert_eq!(fs.inode_size(d), 20 * 32, "directory shrank by exactly what was removed");
        for n in &gone {
            assert_eq!(fs.dir_lookup(d, n), None, "{n} is gone");
        }
    }

    #[test]
    fn hard_link_shares_one_inode() {
        let fs = fresh();
        let root = fs.root_inode();
        let f = fs.create(root, "original", InodeType::File).unwrap();
        fs.write_at(f, 0, b"shared");

        assert!(fs.link(root, "alias", f), "second name accepted");
        assert_eq!(fs.dir_lookup(root, "alias"), Some(f), "both names, one inode");
        assert_eq!(fs.read_disk_inode(f, |di| di.nlink), 2);

        assert!(fs.remove(root, "original"));
        assert_eq!(fs.dir_lookup(root, "original"), None);
        let mut buf = [0u8; 8];
        let n = fs.read_at(f, 0, &mut buf);
        assert_eq!(&buf[..n], b"shared", "content survives via the remaining name");
        assert_eq!(fs.read_disk_inode(f, |di| di.nlink), 1);

        assert!(fs.remove(root, "alias"));
        assert_eq!(fs.create(root, "reused", InodeType::File), Some(f), "inode reclaimed");
    }

    #[test]
    fn link_refuses_directories_and_duplicates() {
        let fs = fresh();
        let root = fs.root_inode();
        let d = fs.create(root, "d", InodeType::Dir).unwrap();
        let f = fs.create(root, "f", InodeType::File).unwrap();
        assert!(!fs.link(root, "d_alias", d), "linking a directory would allow cycles");
        assert!(!fs.link(root, "f", f), "duplicate name refused");
        assert!(!fs.link(root, "bad/name", f), "invalid name refused");
        let freed = fs.create(root, "gone", InodeType::File).unwrap();
        assert!(fs.remove(root, "gone"));
        assert!(!fs.link(root, "ghost", freed), "a freed inode is not a link target");
        assert_eq!(fs.read_disk_inode(f, |di| di.nlink), 1, "no refusal bumped the link count");
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

    #[test]
    fn churned_tree_survives_reopen() {
        let ram = test_ram();
        {
            let fs = format_on(&ram);
            let root = fs.root_inode();
            let d = fs.create(root, "d", InodeType::Dir).unwrap();
            for i in 0..20 {
                fs.create(d, &alloc::format!("f{i}"), InodeType::File).unwrap();
            }
            let keeper = fs.dir_lookup(d, "f7").unwrap();
            fs.write_at(keeper, 0, b"kept");
            fs.link(d, "f7_alias", keeper);
            for i in (0..20).step_by(2) {
                assert!(fs.remove(d, &alloc::format!("f{i}")));
            }
            fs.sync();
        }
        let fs = mount_on(&ram);
        let d = fs.resolve("/d").unwrap();
        let mut expected: Vec<String> = (0..20)
            .filter(|i| i % 2 == 1)
            .map(|i| alloc::format!("f{i}"))
            .chain(["f7_alias".to_string()])
            .collect();
        expected.sort();
        assert_eq!(names(&fs, d), expected, "the compacted directory reads back intact");

        let keeper = fs.resolve("/d/f7").unwrap();
        assert_eq!(fs.resolve("/d/f7_alias"), Some(keeper), "hard link survived the remount");
        assert_eq!(fs.read_disk_inode(keeper, |di| di.nlink), 2);
        let mut buf = [0u8; 8];
        let n = fs.read_at(keeper, 0, &mut buf);
        assert_eq!(&buf[..n], b"kept");
    }
}
