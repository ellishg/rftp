use crate::progress::Progress;
use async_ssh2;
use futures::io::{AsyncReadExt, AsyncWriteExt, BufReader, BufWriter};
use std::error::Error;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use tui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    widgets::{Block, Borders, SelectableList, Widget},
};

#[derive(Clone, PartialEq, Eq, Ord)]
pub enum LocalFileEntry {
    File(PathBuf),
    Directory(PathBuf),
    Parent(PathBuf),
}

#[derive(Clone, PartialEq, Eq, Ord)]
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

#[derive(Clone)]
enum SelectedFileEntryIndex {
    Local(usize),
    Remote(usize),
    None,
}

#[derive(Clone)]
pub struct FileList {
    local_directory: PathBuf,
    remote_directory: PathBuf,
    local_entries: Vec<LocalFileEntry>,
    remote_entries: Vec<RemoteFileEntry>,
    selected: SelectedFileEntryIndex,
}

// TODO: Tune this variable to make downloads/uploads faster.
const CHUNK_SIZE: usize = 1024 * 1024 * 8;

/// Reads the remote file `source`, creates/truncates the local file `dest`,
/// and writes the data to `dest`.
pub async fn download(
    source: RemoteFileEntry,
    dest: impl AsRef<Path>,
    sftp: &async_ssh2::Sftp,
    progress: &Progress,
) -> Result<(), Box<dyn Error>> {
    assert!(source.is_file(), "Source must be a file!");
    let source = sftp.open(&source.path()).await?;
    let dest = async_std::fs::File::create(dest.as_ref()).await?;
    let mut reader = BufReader::new(source);
    let mut writer = BufWriter::new(dest);
    let mut buffer = vec![0; CHUNK_SIZE];

    loop {
        let bytes_read = reader.read(&mut buffer).await?;
        if bytes_read == 0 {
            break;
        } else {
            writer.write_all(&buffer[..bytes_read]).await?;
            progress.inc(bytes_read as u64);
        }
    }

    writer.close().await?;
    progress.finish();
    Ok(())
}

/// Reads the local file `source`, creates/truncates the remote file `dest`,
/// and writes the data to `dest`.
pub async fn upload(
    source: LocalFileEntry,
    dest: impl AsRef<Path>,
    sftp: &async_ssh2::Sftp,
    progress: &Progress,
) -> Result<(), Box<dyn Error>> {
    assert!(source.is_file(), "Source must be a file!");
    let source = async_std::fs::File::open(source.path()).await?;
    let dest = sftp.create(dest.as_ref()).await?;
    let mut reader = BufReader::new(source);
    let mut writer = BufWriter::new(dest);
    let mut buffer = vec![0; CHUNK_SIZE];

    loop {
        let bytes_read = reader.read(&mut buffer).await?;
        if bytes_read == 0 {
            break;
        } else {
            writer.write_all(&buffer[..bytes_read]).await?;
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

    pub async fn len(&self) -> std::io::Result<Option<u64>> {
        match self {
            LocalFileEntry::File(path) => {
                let metadata = tokio::fs::metadata(path).await?;
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
            LocalFileEntry::Parent(_) => "\u{2b05}".to_string(),
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
            RemoteFileEntry::Parent(_) => "\u{2b05}".to_string(),
        }
    }
}

impl FileList {
    pub async fn new(
        local_path: impl AsRef<Path>,
        remote_path: impl AsRef<Path>,
        sftp: &async_ssh2::Sftp,
        keep_hidden_files: bool,
    ) -> Result<Self, Box<dyn Error>> {
        let mut list = FileList {
            local_directory: PathBuf::new(),
            remote_directory: PathBuf::new(),
            local_entries: vec![],
            remote_entries: vec![],
            selected: SelectedFileEntryIndex::None,
        };
        list.set_local_working_path(local_path, keep_hidden_files)?;
        list.set_remote_working_path(remote_path, sftp, keep_hidden_files)
            .await?;
        Ok(list)
    }

    pub fn fetch_local_files(&mut self, keep_hidden_files: bool) -> std::io::Result<()> {
        self.local_entries = vec![];
        if self.local_directory.parent().is_some() {
            let dotdot = self.local_directory.join("..");
            self.local_entries.push(LocalFileEntry::Parent(dotdot));
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
        self.local_entries.sort_unstable();
        Ok(())
    }

    pub async fn fetch_remote_files(
        &mut self,
        sftp: &async_ssh2::Sftp,
        keep_hidden_files: bool,
    ) -> Result<(), async_ssh2::Error> {
        self.remote_entries = vec![];
        if self.remote_directory.parent().is_some() {
            let dotdot = self.remote_directory.join("..");
            self.remote_entries.push(RemoteFileEntry::Parent(dotdot));
        }
        self.remote_entries
            .extend(
                sftp.readdir(&self.remote_directory)
                    .await?
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
        self.remote_entries.sort_unstable();
        Ok(())
    }

    pub fn set_local_working_path(
        &mut self,
        path: impl AsRef<Path>,
        keep_hidden_files: bool,
    ) -> std::io::Result<()> {
        self.local_directory = std::fs::canonicalize(path)?;
        self.fetch_local_files(keep_hidden_files)?;
        // Make sure we have a valid entry selected.
        self.next_selected();
        self.prev_selected();
        Ok(())
    }

    pub async fn set_remote_working_path(
        &mut self,
        path: impl AsRef<Path>,
        sftp: &async_ssh2::Sftp,
        keep_hidden_files: bool,
    ) -> Result<(), async_ssh2::Error> {
        // TODO: canonicalize
        self.remote_directory = path.as_ref().to_path_buf();
        self.fetch_remote_files(sftp, keep_hidden_files).await?;
        // Make sure we have a valid entry selected.
        self.next_selected();
        self.prev_selected();
        Ok(())
    }

    pub fn get_local_working_path(&self) -> &Path {
        &self.local_directory
    }

    pub fn get_remote_working_path(&self) -> &Path {
        &self.remote_directory
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

    pub fn draw<B>(&self, frame: &mut tui::terminal::Frame<B>, rect: tui::layout::Rect)
    where
        B: tui::backend::Backend,
    {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Ratio(1, 2), Constraint::Ratio(1, 2)].as_ref())
            .split(rect);
        let (local_rect, remote_rect) = (chunks[0], chunks[1]);

        let mut render = |title, items: Vec<String>, selected, rect| {
            // TODO: Would like to style items by file type.
            //       https://github.com/fdehau/tui-rs/issues/254
            SelectableList::default()
                .block(Block::default().title(title).borders(Borders::ALL))
                .items(items.as_slice())
                .select(selected)
                .highlight_style(Style::default().bg(Color::Yellow))
                .highlight_symbol(">>")
                .render(frame, rect);
        };

        let title = format!("Local: {:?}", self.get_local_working_path());
        let items = self
            .local_entries
            .iter()
            .map(|entry| entry.to_text())
            .collect();
        let selected = self.get_local_selected_index();
        render(&title, items, selected, local_rect);

        let title = format!("Remote: {:?}", self.get_remote_working_path());
        let items = self
            .remote_entries
            .iter()
            .map(|entry| entry.to_text())
            .collect();
        let selected = self.get_remote_selected_index();
        render(&title, items, selected, remote_rect);
    }
}

impl PartialOrd for LocalFileEntry {
    fn partial_cmp(&self, other: &LocalFileEntry) -> Option<std::cmp::Ordering> {
        self.path().partial_cmp(other.path())
    }
}

impl PartialOrd for RemoteFileEntry {
    fn partial_cmp(&self, other: &RemoteFileEntry) -> Option<std::cmp::Ordering> {
        self.path().partial_cmp(other.path())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::buffer_without_style;
    use tui::{backend::TestBackend, buffer::Buffer, Terminal};

    #[test]
    fn test_file_list() {
        let file_list: FileList = FileList {
            local_directory: PathBuf::from("/a/b/c"),
            remote_directory: PathBuf::from("home/files"),
            local_entries: vec![
                LocalFileEntry::Parent(PathBuf::from("/a/b/c/..")),
                LocalFileEntry::File(PathBuf::from("/a/b/c/myfile.txt")),
                LocalFileEntry::File(PathBuf::from("/a/b/c/myotherfile.dat")),
                LocalFileEntry::Directory(PathBuf::from("/a/b/c/important")),
            ],
            remote_entries: vec![
                RemoteFileEntry::Parent(PathBuf::from("home/files/..")),
                RemoteFileEntry::File(PathBuf::from("home/files/pic.png"), 128),
                RemoteFileEntry::File(PathBuf::from("home/files/movie.mkv"), 512),
                RemoteFileEntry::Directory(PathBuf::from("home/files/games")),
                RemoteFileEntry::Directory(PathBuf::from("home/files/trash")),
            ],
            selected: SelectedFileEntryIndex::Remote(2),
        };

        let mut terminal = Terminal::new(TestBackend::new(50, 8)).unwrap();

        terminal
            .draw(|mut frame| {
                let rect = frame.size();
                file_list.draw(&mut frame, rect);
            })
            .unwrap();

        assert_eq!(
            buffer_without_style(terminal.backend().buffer()),
            Buffer::with_lines(vec![
                "┌Local: \"/a/b/c\"────────┐┌Remote: \"home/files\"───┐",
                "│\u{2b05}                      ││   \u{2b05}                   │",
                "│myfile.txt             ││   pic.png             │",
                "│myotherfile.dat        ││>> movie.mkv           │",
                "│important/             ││   games/              │",
                "│                       ││   trash/              │",
                "│                       ││                       │",
                "└───────────────────────┘└───────────────────────┘",
            ])
        );
    }
}
