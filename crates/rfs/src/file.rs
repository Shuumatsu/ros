//! Ergonomic, path- and cursor-based access on top of the inode primitives.
//!
//! [`Fs`]'s core methods work in terms of `(inode, offset)`. This layer adds the
//! conveniences a caller actually wants: create / open / remove by path, and a
//! [`File`] handle that carries a cursor so you can `read` / `write` / `seek`
//! like `std::fs::File`, without threading offsets by hand.

use alloc::vec::Vec;

use crate::fs::Fs;
use crate::layout::InodeType;

impl Fs {
    /// Open an existing regular file by path. `None` if the path does not
    /// resolve or does not name a regular file.
    pub fn open(&self, path: &str) -> Option<File<'_>> {
        let inode = self.resolve(path)?;
        (self.inode_type(inode) == Some(InodeType::File)).then(|| File::new(self, inode))
    }

    /// Create a regular file at `path` and open it. `None` if the name is taken
    /// or invalid, the parent directory does not exist, or inodes are exhausted.
    pub fn create_file(&self, path: &str) -> Option<File<'_>> {
        let inode = self.create_path(path, InodeType::File)?;
        Some(File::new(self, inode))
    }

    /// Create an inode of `kind` at `path`, returning its inode number. The
    /// parent directory must already exist — this is not `mkdir -p`.
    pub fn create_path(&self, path: &str, kind: InodeType) -> Option<u32> {
        let (parent, name) = self.split_parent(path)?;
        self.create(parent, name, kind)
    }

    /// Remove whatever `path` names (see [`Fs::remove`] for the rules).
    pub fn remove_path(&self, path: &str) -> bool {
        match self.split_parent(path) {
            Some((parent, name)) => self.remove(parent, name),
            None => false,
        }
    }

    /// Hard-link `new_path` to the existing file at `existing` — argument order as
    /// in `ln existing new_path`. See [`Fs::link`] for the rules.
    pub fn link_path(&self, existing: &str, new_path: &str) -> bool {
        let Some(target) = self.resolve(existing) else {
            return false;
        };
        match self.split_parent(new_path) {
            Some((parent, name)) => self.link(parent, name, target),
            None => false,
        }
    }

    /// Split a path into (parent-directory inode, final component). `None` if
    /// there is no final component (e.g. `"/"`) or the parent does not resolve.
    fn split_parent<'p>(&self, path: &'p str) -> Option<(u32, &'p str)> {
        let trimmed = path.trim_end_matches('/');
        let (parent_path, name) = match trimmed.rfind('/') {
            Some(i) => (&trimmed[..i], &trimmed[i + 1..]),
            None => ("", trimmed),
        };
        if name.is_empty() {
            return None;
        }
        let parent =
            if parent_path.is_empty() { self.root_inode() } else { self.resolve(parent_path)? };
        Some((parent, name))
    }
}

/// A cursor into an open file. Borrows its [`Fs`], so many files may be open at
/// once; each keeps its own position.
pub struct File<'a> {
    fs: &'a Fs,
    inode: u32,
    pos: usize,
}

impl<'a> File<'a> {
    /// Wrap `inode` as an open file positioned at the start.
    pub fn new(fs: &'a Fs, inode: u32) -> Self { Self { fs, inode, pos: 0 } }

    /// Current file length in bytes.
    pub fn len(&self) -> usize { self.fs.inode_size(self.inode) }

    /// Whether the file is empty.
    pub fn is_empty(&self) -> bool { self.len() == 0 }

    /// Current cursor position.
    pub fn pos(&self) -> usize { self.pos }

    /// Resize the file to `len` bytes, freeing the blocks a shrink gives up. The
    /// cursor does not move — a read past the new end simply returns nothing.
    pub fn set_len(&mut self, len: usize) { self.fs.set_len(self.inode, len); }

    /// Move the cursor to an absolute byte offset.
    pub fn seek(&mut self, pos: usize) { self.pos = pos; }

    /// Move the cursor back to the start.
    pub fn rewind(&mut self) { self.pos = 0; }

    /// Read from the cursor into `buf`, advancing it. Returns bytes read (0 at
    /// EOF).
    pub fn read(&mut self, buf: &mut [u8]) -> usize {
        let n = self.fs.read_at(self.inode, self.pos, buf);
        self.pos += n;
        n
    }

    /// Write `buf` at the cursor, advancing it and growing the file as needed.
    /// Returns bytes written (always `buf.len()`).
    pub fn write(&mut self, buf: &[u8]) -> usize {
        let n = self.fs.write_at(self.inode, self.pos, buf);
        self.pos += n;
        n
    }

    /// Append the file's bytes from the cursor to EOF onto `out`, advancing the
    /// cursor. Returns bytes read.
    pub fn read_to_end(&mut self, out: &mut Vec<u8>) -> usize {
        let remaining = self.len().saturating_sub(self.pos);
        let start = out.len();
        out.resize(start + remaining, 0);
        let n = self.fs.read_at(self.inode, self.pos, &mut out[start..]);
        out.truncate(start + n);
        self.pos += n;
        n
    }
}

#[cfg(test)]
mod tests {
    use crate::layout::InodeType;
    use crate::test_support::{format_on, fresh, mount_on, pattern, test_ram};
    use alloc::vec::Vec;
    use pretty_assertions::assert_eq;

    #[test]
    fn create_write_open_read() {
        let fs = fresh();
        fs.create_path("/dir", InodeType::Dir).unwrap();
        {
            let mut f = fs.create_file("/dir/greeting").unwrap();
            assert_eq!(f.write(b"hello "), 6);
            assert_eq!(f.write(b"world"), 5);
            assert_eq!(f.len(), 11);
            assert_eq!(f.pos(), 11);
        }
        let mut f = fs.open("/dir/greeting").unwrap();
        let mut out = Vec::new();
        f.read_to_end(&mut out);
        assert_eq!(out, b"hello world".to_vec());
    }

    #[test]
    fn cursor_seek_and_partial_read() {
        let fs = fresh();
        let mut f = fs.create_file("/f").unwrap();
        f.write(b"0123456789");
        f.seek(3);
        let mut buf = [0u8; 4];
        assert_eq!(f.read(&mut buf), 4);
        assert_eq!(&buf, b"3456");
        assert_eq!(f.pos(), 7);
        f.rewind();
        assert_eq!(f.pos(), 0);
    }

    #[test]
    fn open_missing_or_wrong_type() {
        let fs = fresh();
        fs.create_path("/d", InodeType::Dir).unwrap();
        assert!(fs.open("/nope").is_none(), "missing path is not openable");
        assert!(fs.open("/d").is_none(), "a directory is not openable as a file");
    }

    #[test]
    fn create_file_requires_existing_parent() {
        let fs = fresh();
        assert!(fs.create_file("/missing/f").is_none(), "parent directory must exist");
        assert!(fs.create_file("/f").is_some());
        assert!(fs.create_file("/f").is_none(), "cannot create over an existing name");
    }

    #[test]
    fn remove_path_deletes() {
        let fs = fresh();
        fs.create_file("/f").unwrap().write(b"data");
        assert!(fs.remove_path("/f"));
        assert!(fs.open("/f").is_none());
        assert!(!fs.remove_path("/f"), "removing a gone file reports false");
    }

    #[test]
    fn set_len_shortens_and_extends_through_the_handle() {
        let fs = fresh();
        let mut f = fs.create_file("/f").unwrap();
        f.write(&pattern(10_000));

        f.set_len(1_000);
        assert_eq!(f.len(), 1_000);
        f.rewind();
        let mut out = Vec::new();
        f.read_to_end(&mut out);
        assert_eq!(out, pattern(1_000), "keeps the head, drops the tail");

        f.set_len(1_500);
        f.rewind();
        out.clear();
        f.read_to_end(&mut out);
        assert_eq!(out.len(), 1_500);
        assert!(out[1_000..].iter().all(|&b| b == 0), "extension reads as zeros");
    }

    #[test]
    fn link_path_shares_content_between_two_paths() {
        let fs = fresh();
        fs.create_path("/a", InodeType::Dir).unwrap();
        fs.create_path("/b", InodeType::Dir).unwrap();
        fs.create_file("/a/f").unwrap().write(b"linked");

        assert!(fs.link_path("/a/f", "/b/g"), "ln /a/f /b/g");
        let mut out = Vec::new();
        fs.open("/b/g").unwrap().read_to_end(&mut out);
        assert_eq!(out, b"linked".to_vec(), "same bytes through the new path");

        assert!(fs.remove_path("/a/f"));
        out.clear();
        fs.open("/b/g").unwrap().read_to_end(&mut out);
        assert_eq!(out, b"linked".to_vec(), "unlinking one path keeps the file");

        assert!(!fs.link_path("/nope", "/b/h"), "missing source refused");
        assert!(!fs.link_path("/b/g", "/missing/h"), "missing parent refused");
    }

    #[test]
    fn large_file_through_handle() {
        let fs = fresh();
        let data = pattern(100_000);
        {
            let mut f = fs.create_file("/big").unwrap();
            assert_eq!(f.write(&data), data.len());
        }
        let mut f = fs.open("/big").unwrap();
        let mut out = Vec::new();
        f.read_to_end(&mut out);
        assert_eq!(out, data, "large file round-trips through the File handle");
    }

    #[test]
    fn end_to_end_across_remount() {
        let ram = test_ram();
        {
            let fs = format_on(&ram);
            fs.create_path("/etc", InodeType::Dir).unwrap();
            fs.create_file("/etc/motd").unwrap().write(b"welcome to rfs\n");
            fs.sync();
        }
        let fs = mount_on(&ram);
        let mut f = fs.open("/etc/motd").unwrap();
        let mut out = Vec::new();
        f.read_to_end(&mut out);
        assert_eq!(out, b"welcome to rfs\n".to_vec());
    }
}
