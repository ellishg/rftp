use crate::progress::Progress;
use ssh2;
use std::ffi::OsStr;
use std::io::{Read, Result, Write};
use std::path::{Path, PathBuf};
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader, BufWriter};

#[derive(Clone)]
pub enum LocalFileEntry {
    File(PathBuf),
    Directory(PathBuf),
    Parent(PathBuf),
}

#[derive(Clone)]
pub enum RemoteFileEntry {
    File(PathBuf, u64),
    Directory(PathBuf),
    Parent(PathBuf),
}

pub enum SelectedFileEntry {
    Local(LocalFileEntry),
    Remote(RemoteFileEntry),
    None,
}

enum SelectedFileEntryIndex {
    Local(usize),
    Remote(usize),
    None,
}

pub struct FileList {
    local_directory: PathBuf,
    remote_directory: PathBuf,
    local_entries: Vec<LocalFileEntry>,
    remote_entries: Vec<RemoteFileEntry>,
    selected: SelectedFileEntryIndex,
}

const CHUNK_SIZE: usize = 1024 * 1024 * 8;

/// Reads the remote file `source`, creates/truncates the local file `dest`,
/// and writes the data to `dest`.
pub async fn download(
    source: RemoteFileEntry,
    dest: impl AsRef<Path>,
    sftp: &ssh2::Sftp,
    progress: &Progress,
) -> Result<()> {
    assert!(source.is_file(), "Source must be a file!");
    let source = source.path();
    let mut source = sftp.open(&source)?;
    let dest = tokio::fs::File::create(dest).await?;
    let mut writer = BufWriter::new(dest);
    let mut buffer = vec![0; CHUNK_SIZE];

    loop {
        // TODO: ssh2::File::read is not async.
        //       https://github.com/alexcrichton/ssh2-rs/issues/58
        let bytes_read = source.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        } else {
            writer.write_all(&buffer[..bytes_read]).await?;
            progress.inc(bytes_read as u64);
        }
    }

    writer.shutdown().await?;
    progress.finish();
    Ok(())
}

/// Reads the local file `source`, creates/truncates the remote file `dest`,
/// and writes the data to `dest`.
pub async fn upload(
    source: LocalFileEntry,
    dest: impl AsRef<Path>,
    sftp: &ssh2::Sftp,
    progress: &Progress,
) -> Result<()> {
    assert!(source.is_file(), "Source must be a file!");
    let source = source.path();
    let source = tokio::fs::File::open(source).await?;
    let mut dest = sftp.create(dest.as_ref())?;
    let mut reader = BufReader::new(source);
    let mut buffer = vec![0; CHUNK_SIZE];

    loop {
        let bytes_read = reader.read(&mut buffer).await?;
        if bytes_read == 0 {
            break;
        } else {
            // TODO: ssh2::File::write is not async.
            //       https://github.com/alexcrichton/ssh2-rs/issues/58
            dest.write_all(&buffer[..bytes_read])?;
            progress.inc(bytes_read as u64);
        }
    }

    progress.finish();
    Ok(())
}

impl LocalFileEntry {
    pub fn path(&self) -> &Path {
        match self {
            LocalFileEntry::File(path) => path,
            LocalFileEntry::Directory(path) => path,
            LocalFileEntry::Parent(path) => path,
        }
    }

    pub fn file_name(&self) -> Option<&OsStr> {
        match self {
            LocalFileEntry::File(path) => path.file_name(),
            LocalFileEntry::Directory(_) => None,
            LocalFileEntry::Parent(_) => None,
        }
    }

    pub fn is_dir(&self) -> bool {
        match self {
            LocalFileEntry::File(_) => false,
            LocalFileEntry::Directory(_) => true,
            LocalFileEntry::Parent(_) => true,
        }
    }

    pub fn is_file(&self) -> bool {
        match self {
            LocalFileEntry::File(_) => true,
            LocalFileEntry::Directory(_) => false,
            LocalFileEntry::Parent(_) => false,
        }
    }

    pub fn len(&self) -> Result<Option<u64>> {
        match self {
            LocalFileEntry::File(path) => {
                let metadata = std::fs::metadata(path)?;
                Ok(Some(metadata.len()))
            }
            LocalFileEntry::Directory(_) => Ok(None),
            LocalFileEntry::Parent(_) => Ok(None),
        }
    }

    pub fn is_hidden(&self) -> bool {
        if let LocalFileEntry::Parent(_) = self {
            false
        } else {
            let filename = self.path().file_name().unwrap();
            filename.to_str().unwrap().chars().next() == Some('.')
        }
    }

    // TODO: Return Text.
    pub fn to_text(&self) -> String {
        match self {
            LocalFileEntry::File(path) => {
                let filename = path.file_name().unwrap().to_str().unwrap();
                format!("{}", filename)
            }
            LocalFileEntry::Directory(path) => {
                let filename = path.file_name().unwrap().to_str().unwrap();
                format!("{}/", filename)
            }
            LocalFileEntry::Parent(_) => "⬅️".to_string(),
        }
    }
}

impl RemoteFileEntry {
    pub fn path(&self) -> &Path {
        match self {
            RemoteFileEntry::File(path, _) => path,
            RemoteFileEntry::Directory(path) => path,
            RemoteFileEntry::Parent(path) => path,
        }
    }

    pub fn file_name(&self) -> Option<&OsStr> {
        match self {
            RemoteFileEntry::File(path, _) => path.file_name(),
            RemoteFileEntry::Directory(_) => None,
            RemoteFileEntry::Parent(_) => None,
        }
    }

    pub fn is_dir(&self) -> bool {
        match self {
            RemoteFileEntry::File(_, _) => false,
            RemoteFileEntry::Directory(_) => true,
            RemoteFileEntry::Parent(_) => true,
        }
    }

    pub fn is_file(&self) -> bool {
        match self {
            RemoteFileEntry::File(_, _) => true,
            RemoteFileEntry::Directory(_) => false,
            RemoteFileEntry::Parent(_) => false,
        }
    }

    pub fn len(&self) -> Option<u64> {
        match self {
            RemoteFileEntry::File(_, len) => Some(*len),
            RemoteFileEntry::Directory(_) => None,
            RemoteFileEntry::Parent(_) => None,
        }
    }

    pub fn is_hidden(&self) -> bool {
        if let RemoteFileEntry::Parent(_) = self {
            false
        } else {
            let filename = self.path().file_name().unwrap();
            filename.to_str().unwrap().chars().next() == Some('.')
        }
    }

    // TODO: Return Text.
    pub fn to_text(&self) -> String {
        match self {
            RemoteFileEntry::File(path, _) => {
                let filename = path.file_name().unwrap().to_str().unwrap();
                format!("{}", filename)
            }
            RemoteFileEntry::Directory(path) => {
                let filename = path.file_name().unwrap().to_str().unwrap();
                format!("{}/", filename)
            }
            RemoteFileEntry::Parent(_) => "⬅️".to_string(),
        }
    }
}

impl FileList {
    pub fn new(
        local_path: impl AsRef<Path>,
        remote_path: impl AsRef<Path>,
        sftp: &ssh2::Sftp,
        keep_hidden_files: bool,
    ) -> Result<Self> {
        let mut list = FileList {
            local_directory: PathBuf::new(),
            remote_directory: PathBuf::new(),
            local_entries: vec![],
            remote_entries: vec![],
            selected: SelectedFileEntryIndex::None,
        };
        list.set_local_working_path(local_path, keep_hidden_files)?;
        list.set_remote_working_path(remote_path, sftp, keep_hidden_files)?;
        Ok(list)
    }

    pub fn fetch_local_files(&mut self, keep_hidden_files: bool) -> Result<()> {
        // TODO: Sort files.
        self.local_entries = vec![];
        if self.local_directory.parent().is_some() {
            let dotdot = {
                let mut path = self.local_directory.to_path_buf();
                path.push("..");
                LocalFileEntry::Parent(path)
            };
            self.local_entries.push(dotdot);
        }
        for entry in std::fs::read_dir(&self.local_directory)? {
            let path = entry?.path();
            if path.is_file() {
                self.local_entries.push(LocalFileEntry::File(path));
            } else {
                self.local_entries.push(LocalFileEntry::Directory(path));
            }
        }
        if !keep_hidden_files {
            self.local_entries.retain(|entry| !entry.is_hidden());
        }
        Ok(())
    }

    pub fn fetch_remote_files(&mut self, sftp: &ssh2::Sftp, keep_hidden_files: bool) -> Result<()> {
        // TODO: Sort files.
        self.remote_entries = vec![];
        if self.remote_directory.parent().is_some() {
            let dotdot = {
                let mut path = self.remote_directory.to_path_buf();
                path.push("..");
                RemoteFileEntry::Parent(path)
            };
            self.remote_entries.push(dotdot);
        }
        self.remote_entries
            .extend(
                sftp.readdir(&self.remote_directory)?
                    .iter()
                    .map(|(path, stat)| {
                        if stat.is_file() {
                            RemoteFileEntry::File(path.to_path_buf(), stat.size.unwrap())
                        } else {
                            RemoteFileEntry::Directory(path.to_path_buf())
                        }
                    }),
            );
        if !keep_hidden_files {
            self.remote_entries.retain(|entry| !entry.is_hidden());
        }
        Ok(())
    }

    pub fn set_local_working_path(
        &mut self,
        path: impl AsRef<Path>,
        keep_hidden_files: bool,
    ) -> Result<()> {
        self.local_directory = std::fs::canonicalize(path)?;
        self.fetch_local_files(keep_hidden_files)?;
        Ok(())
    }

    pub fn set_remote_working_path(
        &mut self,
        path: impl AsRef<Path>,
        sftp: &ssh2::Sftp,
        keep_hidden_files: bool,
    ) -> Result<()> {
        // TODO: canonicalize
        self.remote_directory = path.as_ref().to_path_buf();
        self.fetch_remote_files(sftp, keep_hidden_files)?;
        Ok(())
    }

    pub fn get_local_working_path(&self) -> &Path {
        &self.local_directory
    }

    pub fn get_remote_working_path(&self) -> &Path {
        &self.remote_directory
    }

    pub fn get_file_entries(&self) -> (&Vec<LocalFileEntry>, &Vec<RemoteFileEntry>) {
        (&self.local_entries, &self.remote_entries)
    }

    pub fn get_selected_entry(&self) -> SelectedFileEntry {
        match self.selected {
            SelectedFileEntryIndex::Local(i) => {
                let entry = self.local_entries[i].clone();
                SelectedFileEntry::Local(entry)
            }
            SelectedFileEntryIndex::Remote(i) => {
                let entry = self.remote_entries[i].clone();
                SelectedFileEntry::Remote(entry)
            }
            SelectedFileEntryIndex::None => SelectedFileEntry::None,
        }
    }

    pub fn get_local_selected_index(&self) -> Option<usize> {
        match self.selected {
            SelectedFileEntryIndex::Local(i) => Some(i),
            _ => None,
        }
    }

    pub fn get_remote_selected_index(&self) -> Option<usize> {
        match self.selected {
            SelectedFileEntryIndex::Remote(i) => Some(i),
            _ => None,
        }
    }

    fn apply_op_to_selected<F>(&mut self, f: F)
    where
        F: Fn(isize) -> isize,
    {
        self.selected = match self.selected {
            SelectedFileEntryIndex::Local(i) => {
                assert!(!self.local_entries.is_empty());
                let n = self.local_entries.len();
                SelectedFileEntryIndex::Local(f(i as isize).rem_euclid(n as isize) as usize)
            }
            SelectedFileEntryIndex::Remote(i) => {
                assert!(!self.remote_entries.is_empty());
                let n = self.remote_entries.len();
                SelectedFileEntryIndex::Remote(f(i as isize).rem_euclid(n as isize) as usize)
            }
            SelectedFileEntryIndex::None => {
                if !self.remote_entries.is_empty() {
                    SelectedFileEntryIndex::Remote(0)
                } else if !self.local_entries.is_empty() {
                    SelectedFileEntryIndex::Local(0)
                } else {
                    SelectedFileEntryIndex::None
                }
            }
        }
    }

    pub fn next_selected(&mut self) {
        self.apply_op_to_selected(|i| i + 1)
    }

    pub fn prev_selected(&mut self) {
        self.apply_op_to_selected(|i| i - 1)
    }

    pub fn toggle_selected(&mut self) {
        self.selected = match self.selected {
            SelectedFileEntryIndex::Local(i) => {
                if !self.remote_entries.is_empty() {
                    SelectedFileEntryIndex::Remote(i)
                } else {
                    SelectedFileEntryIndex::None
                }
            }
            SelectedFileEntryIndex::Remote(i) => {
                if !self.local_entries.is_empty() {
                    SelectedFileEntryIndex::Local(i)
                } else {
                    SelectedFileEntryIndex::None
                }
            }
            SelectedFileEntryIndex::None => {
                if !self.remote_entries.is_empty() {
                    SelectedFileEntryIndex::Remote(0)
                } else if !self.local_entries.is_empty() {
                    SelectedFileEntryIndex::Local(0)
                } else {
                    SelectedFileEntryIndex::None
                }
            }
        };
        self.apply_op_to_selected(|i| i);
    }
}
