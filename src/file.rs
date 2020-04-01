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

pub struct LocalFileList {
    directory: PathBuf,
    entries: Vec<LocalFileEntry>,
    // selected: Option<usize>,
}

pub struct RemoteFileList {
    directory: PathBuf,
    entries: Vec<RemoteFileEntry>,
}

const CHUNK_SIZE: usize = 1024 * 1024 * 8;

/// Reads the remote file `source`, creates/truncates the local file `dest`,
/// and writes the data to `dest`.
pub async fn download(
    source: RemoteFileEntry,
    dest: impl AsRef<Path>,
    sftp: &ssh2::Sftp,
    progress: &Progress,
) -> std::io::Result<()> {
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
) -> std::io::Result<()> {
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

/// Reads the local directory `path` and returns its entries.
pub fn read_local_dir(path: impl AsRef<Path>) -> Result<Vec<LocalFileEntry>> {
    // TODO: Can be async
    // let mut entries = tokio::fs::read_dir(path).await?;
    // let mut local_entries = vec![];
    // while let Some(entry) = entries.next_entry().await? {
    //     let file_type = entry.file_type().await?;
    //     if file_type.is_file() {
    //         local_entries.push(LocalFileEntry::File(entry.path()));
    //     } else {
    //         local_entries.push(LocalFileEntry::Directory(entry.path()));
    //     }
    // }
    let mut local_entries = vec![];
    if path.as_ref().parent().is_some() {
        let dotdot = {
            let mut path = path.as_ref().to_path_buf();
            path.push("..");
            LocalFileEntry::Parent(path)
        };
        local_entries.push(dotdot);
    }
    for entry in std::fs::read_dir(path)? {
        let path = entry?.path();
        if path.is_file() {
            local_entries.push(LocalFileEntry::File(path));
        } else {
            local_entries.push(LocalFileEntry::Directory(path));
        }
    }
    Ok(local_entries)
}

/// Reads the remote directory `path` and returns its entries.
pub fn read_remote_dir(path: impl AsRef<Path>, sftp: &ssh2::Sftp) -> Result<Vec<RemoteFileEntry>> {
    // TODO: Make async.
    let mut remote_entries = vec![];
    if path.as_ref().parent().is_some() {
        let dotdot = {
            let mut path = path.as_ref().to_path_buf();
            path.push("..");
            RemoteFileEntry::Parent(path)
        };
        remote_entries.push(dotdot);
    }
    remote_entries.extend(sftp.readdir(path.as_ref())?.iter().map(|(path, stat)| {
        if stat.is_file() {
            RemoteFileEntry::File(path.to_path_buf(), stat.size.unwrap())
        } else {
            RemoteFileEntry::Directory(path.to_path_buf())
        }
    }));
    Ok(remote_entries)
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

impl LocalFileList {
    pub fn new(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let mut list = LocalFileList {
            directory: path.as_ref().to_path_buf(),
            entries: vec![],
            // selected: None,
        };
        list.fetch()?;
        Ok(list)
    }

    pub fn fetch(&mut self) -> std::io::Result<()> {
        self.entries = read_local_dir(&self.directory)?;
        Ok(())
    }

    pub fn remove_hidden(&mut self) {
        self.entries.retain(|entry| !entry.is_hidden());
    }

    // pub fn get_selected(&self) -> Option<&LocalFileEntry> {
    //     self.selected.map(|i| &self.entries[i])
    // }

    pub fn set_working_path(&mut self, path: impl AsRef<Path>) -> std::io::Result<()> {
        self.directory = std::fs::canonicalize(path.as_ref().to_path_buf())?;
        Ok(())
    }

    pub fn get_working_path(&self) -> PathBuf {
        self.directory.to_path_buf()
    }

    pub fn get_entries(&self) -> &Vec<LocalFileEntry> {
        &self.entries
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

impl RemoteFileList {
    pub fn new(path: impl AsRef<Path>, sftp: &ssh2::Sftp) -> std::io::Result<Self> {
        let mut list = RemoteFileList {
            directory: path.as_ref().to_path_buf(),
            entries: vec![],
        };
        list.fetch(sftp)?;
        Ok(list)
    }

    pub fn fetch(&mut self, sftp: &ssh2::Sftp) -> std::io::Result<()> {
        self.entries = read_remote_dir(&self.directory, sftp)?;
        Ok(())
    }

    pub fn remove_hidden(&mut self) {
        self.entries.retain(|entry| !entry.is_hidden());
    }

    pub fn set_working_path(&mut self, path: impl AsRef<Path>) {
        // TODO: canonicalize
        self.directory = path.as_ref().to_path_buf();
    }

    pub fn get_working_path(&self) -> PathBuf {
        self.directory.to_path_buf()
    }

    pub fn get_entries(&self) -> &Vec<RemoteFileEntry> {
        &self.entries
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
}
