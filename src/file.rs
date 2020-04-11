use crate::progress::Progress;
use ssh2;
use std::borrow::Cow;
use std::env;
use std::error::Error;
use std::fs::{canonicalize, metadata, read_dir, File};
use std::io;
use std::io::{BufWriter, Read, Write};
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
pub fn download(
    source: RemoteFileEntry,
    dest: impl AsRef<Path>,
    sftp: &ssh2::Sftp,
    progress: &Progress,
) -> io::Result<()> {
    assert!(source.is_file(), "Source must be a file!");
    let source = source.path();
    let mut source = sftp.open(&source)?;
    let dest = File::create(dest)?;
    let mut writer = BufWriter::new(dest);
    let mut buffer = vec![0; CHUNK_SIZE];

    loop {
        let bytes_read = source.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        } else {
            writer.write_all(&buffer[..bytes_read])?;
            progress.inc(bytes_read as u64);
        }
    }

    writer.flush()?;
    progress.finish();
    Ok(())
}

/// Reads the local file `source`, creates/truncates the remote file `dest`,
/// and writes the data to `dest`.
pub fn upload(
    source: LocalFileEntry,
    dest: impl AsRef<Path>,
    sftp: &ssh2::Sftp,
    progress: &Progress,
) -> io::Result<()> {
    assert!(source.is_file(), "Source must be a file!");
    let mut source = File::open(source.path())?;
    let mut dest = sftp.create(dest.as_ref())?;
    let mut buffer = vec![0; CHUNK_SIZE];

    loop {
        let bytes_read = source.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        } else {
            dest.write_all(&buffer[..bytes_read])?;
            progress.inc(bytes_read as u64);
        }
    }

    progress.finish();
    Ok(())
}

pub trait FileEntry {
    fn path(&self) -> &Path;
    fn is_dir(&self) -> bool;
    fn is_file(&self) -> bool;
    fn is_parent(&self) -> bool;
    fn len(&self) -> Option<u64>;

    fn file_name_lossy(&self) -> Option<Cow<str>> {
        self.path()
            .file_name()
            .map(|file_name| file_name.to_string_lossy())
    }

    fn is_hidden(&self) -> bool {
        if self.is_parent() {
            false
        } else {
            // A file is hidden if its name begins with a '.'
            let filename = self.path().file_name().unwrap();
            filename.to_str().unwrap().chars().next() == Some('.')
        }
    }

    // TODO: Return Text
    fn to_text(&self) -> String {
        if self.is_parent() {
            "\u{2b05}".to_string()
        } else {
            if self.is_file() {
                self.file_name_lossy().unwrap().to_string()
            } else if self.is_dir() {
                format!("{}/", self.file_name_lossy().unwrap())
            } else {
                unreachable!()
            }
        }
    }
}

impl FileEntry for LocalFileEntry {
    fn path(&self) -> &Path {
        match self {
            LocalFileEntry::File(path) => path,
            LocalFileEntry::Directory(path) => path,
            LocalFileEntry::Parent(path) => path,
        }
    }

    fn is_dir(&self) -> bool {
        match self {
            LocalFileEntry::File(_) => false,
            LocalFileEntry::Directory(_) => true,
            LocalFileEntry::Parent(_) => true,
        }
    }

    fn is_file(&self) -> bool {
        match self {
            LocalFileEntry::File(_) => true,
            LocalFileEntry::Directory(_) => false,
            LocalFileEntry::Parent(_) => false,
        }
    }

    fn is_parent(&self) -> bool {
        match self {
            LocalFileEntry::File(_) => false,
            LocalFileEntry::Directory(_) => false,
            LocalFileEntry::Parent(_) => true,
        }
    }

    fn len(&self) -> Option<u64> {
        match self {
            LocalFileEntry::File(path) => metadata(path).ok().map(|metadata| metadata.len()),
            LocalFileEntry::Directory(_) => None,
            LocalFileEntry::Parent(_) => None,
        }
    }
}

impl FileEntry for RemoteFileEntry {
    fn path(&self) -> &Path {
        match self {
            RemoteFileEntry::File(path, _) => path,
            RemoteFileEntry::Directory(path) => path,
            RemoteFileEntry::Parent(path) => path,
        }
    }

    fn is_dir(&self) -> bool {
        match self {
            RemoteFileEntry::File(_, _) => false,
            RemoteFileEntry::Directory(_) => true,
            RemoteFileEntry::Parent(_) => true,
        }
    }

    fn is_file(&self) -> bool {
        match self {
            RemoteFileEntry::File(_, _) => true,
            RemoteFileEntry::Directory(_) => false,
            RemoteFileEntry::Parent(_) => false,
        }
    }

    fn is_parent(&self) -> bool {
        match self {
            RemoteFileEntry::File(_, _) => false,
            RemoteFileEntry::Directory(_) => false,
            RemoteFileEntry::Parent(_) => true,
        }
    }

    fn len(&self) -> Option<u64> {
        match self {
            RemoteFileEntry::File(_, len) => Some(*len),
            RemoteFileEntry::Directory(_) => None,
            RemoteFileEntry::Parent(_) => None,
        }
    }
}

impl FileList {
    pub fn new(
        session: &ssh2::Session,
        sftp: &ssh2::Sftp,
        keep_hidden_files: bool,
    ) -> Result<Self, Box<dyn Error>> {
        let local_path = env::current_dir()?;
        let remote_path = get_remote_home_dir(session).unwrap_or(PathBuf::from("./"));

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

    pub fn fetch_local_files(&mut self, keep_hidden_files: bool) -> io::Result<()> {
        self.local_entries = vec![];
        if let Some(parent) = self.local_directory.parent() {
            self.local_entries
                .push(LocalFileEntry::Parent(parent.to_path_buf()));
        }
        for entry in read_dir(&self.local_directory)? {
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

    pub fn fetch_remote_files(
        &mut self,
        sftp: &ssh2::Sftp,
        keep_hidden_files: bool,
    ) -> io::Result<()> {
        self.remote_entries = vec![];
        if let Some(parent) = self.remote_directory.parent() {
            self.remote_entries
                .push(RemoteFileEntry::Parent(parent.to_path_buf()));
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
        self.remote_entries.sort_unstable();
        Ok(())
    }

    pub fn set_local_working_path(
        &mut self,
        path: impl AsRef<Path>,
        keep_hidden_files: bool,
    ) -> io::Result<()> {
        self.local_directory = canonicalize(path)?;
        self.fetch_local_files(keep_hidden_files)?;
        // Make sure we have a valid entry selected.
        self.next_selected();
        self.prev_selected();
        Ok(())
    }

    pub fn set_remote_working_path(
        &mut self,
        path: impl AsRef<Path>,
        sftp: &ssh2::Sftp,
        keep_hidden_files: bool,
    ) -> io::Result<()> {
        // TODO: canonicalize
        self.remote_directory = path.as_ref().to_path_buf();
        self.fetch_remote_files(sftp, keep_hidden_files)?;
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

fn get_remote_home_dir(session: &ssh2::Session) -> Result<PathBuf, Box<dyn Error>> {
    let mut channel = session.channel_session()?;
    channel.exec("pwd")?;
    let mut result = String::new();
    channel.read_to_string(&mut result)?;
    let result = result.trim();
    channel.wait_close()?;
    let exit_status = channel.exit_status()?;
    if exit_status == 0 {
        Ok(PathBuf::from(result))
    } else {
        Err(Box::from(format!(
            "channel closed with exit status {}",
            exit_status
        )))
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
