use crate::progress::Progress;
use crate::utils::{bytes_to_string, get_remote_home_dir, Result};

use ssh2;
use std::borrow::Cow;
use std::env;
use std::fs::{canonicalize, metadata, read_dir, File};
use std::io;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use tui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    widgets::{Block, Borders, List, ListState, Text},
};

const FILELIST_FILE_COLOR: Color = Color::Green;
const FILELIST_DIRECTORY_COLOR: Color = Color::Blue;
const FILELIST_HIGHLIGHT_COLOR: Color = Color::LightMagenta;

#[derive(Clone, PartialEq, Eq, Ord)]
pub enum LocalFileEntry {
    File(PathBuf, u64),
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

/// `CHUNK_SIZE` bytes of data is read from the source and then it is all written to the dest.
const CHUNK_SIZE: usize = 8 * 1024;

/// Reads the remote file `source`, creates/truncates the local file `dest`,
/// and writes the data to `dest`.
pub fn download(
    source: RemoteFileEntry,
    dest: impl AsRef<Path>,
    sftp: &ssh2::Sftp,
    progress: &Progress,
) -> io::Result<()> {
    assert!(source.is_file(), "Source must be a file!");
    let mut source = sftp.open(&source.path())?;
    let mut dest = File::create(dest)?;
    let mut buffer = [0; CHUNK_SIZE];

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
    let mut buffer = [0; CHUNK_SIZE];

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
    /// Return the full path of this entry.
    fn path(&self) -> &Path;
    /// Return true if this entry is a directory or a parent directory.
    fn is_dir(&self) -> bool;
    /// Return true if this entry is a file.
    fn is_file(&self) -> bool;
    /// Return true if this entry is a parent directory.
    fn is_parent(&self) -> bool;
    /// Return the size of this entry.
    fn len(&self) -> Option<u64>;

    /// Return the file name of this entry.
    fn file_name_lossy(&self) -> Option<Cow<str>> {
        self.path()
            .file_name()
            .map(|file_name| file_name.to_string_lossy())
    }

    /// Return true if this entry is a hidden file or directory.
    fn is_hidden(&self) -> bool {
        if self.is_parent() {
            false
        } else {
            // A file is hidden if its name begins with a '.'
            let filename = self.path().file_name().unwrap();
            filename.to_str().unwrap().chars().next() == Some('.')
        }
    }

    /// Returns the text of this entry for displaying to the user.
    fn to_text(&self, width: usize) -> Text {
        if self.is_parent() {
            // TODO: We can either use the emoji "⬅" or ".." for the parent directory.
            // Text::Styled(Cow::Borrowed(".."), Style::default().fg(Color::Red))
            Text::raw("⬅")
        } else if self.is_file() {
            let file_len_string = self
                .len()
                .map(|len| bytes_to_string(len))
                .unwrap_or(String::from(""));
            let width = width - (file_len_string.len() + 1);
            Text::styled(
                format!(
                    "{name:min_width$.max_width$} {len}",
                    name = self.file_name_lossy().unwrap(),
                    min_width = width,
                    max_width = width,
                    len = file_len_string
                ),
                Style::default().fg(FILELIST_FILE_COLOR),
            )
        } else if self.is_dir() {
            Text::styled(
                format!("{}/", self.file_name_lossy().unwrap()),
                Style::default().fg(FILELIST_DIRECTORY_COLOR),
            )
        } else {
            unreachable!()
        }
    }
}

impl FileEntry for LocalFileEntry {
    fn path(&self) -> &Path {
        match self {
            LocalFileEntry::File(path, _) => path,
            LocalFileEntry::Directory(path) => path,
            LocalFileEntry::Parent(path) => path,
        }
    }

    fn is_dir(&self) -> bool {
        match self {
            LocalFileEntry::File(_, _) => false,
            LocalFileEntry::Directory(_) => true,
            LocalFileEntry::Parent(_) => true,
        }
    }

    fn is_file(&self) -> bool {
        match self {
            LocalFileEntry::File(_, _) => true,
            LocalFileEntry::Directory(_) => false,
            LocalFileEntry::Parent(_) => false,
        }
    }

    fn is_parent(&self) -> bool {
        match self {
            LocalFileEntry::File(_, _) => false,
            LocalFileEntry::Directory(_) => false,
            LocalFileEntry::Parent(_) => true,
        }
    }

    fn len(&self) -> Option<u64> {
        match self {
            LocalFileEntry::File(_, len) => Some(*len),
            LocalFileEntry::Directory(_) => None,
            LocalFileEntry::Parent(_) => None,
        }
    }
}

impl LocalFileEntry {
    /// Return a list of `LocalFileEntry`'s that `path` contains if `path`
    /// is a directory.
    ///
    /// The directories `.` and `..` are not included.
    pub fn read_dir(path: &Path) -> io::Result<Vec<LocalFileEntry>> {
        let mut entries = vec![];
        for entry in read_dir(path)? {
            let path = entry?.path();
            if path.is_file() {
                let len = metadata(&path)?.len();
                entries.push(LocalFileEntry::File(path, len))
            // } else if path.is_dir() {
            } else {
                entries.push(LocalFileEntry::Directory(path))
                // } else if path.metadata()?.file_type().is_symlink() {
                // unimplemented!()
                // } else {
                // unimplemented!()
            }
        }
        Ok(entries)
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

impl RemoteFileEntry {
    pub fn read_dir(path: &Path, sftp: &ssh2::Sftp) -> io::Result<Vec<RemoteFileEntry>> {
        Ok(sftp
            .readdir(path)?
            .iter()
            .map(|(path, stat)| {
                if stat.is_file() {
                    RemoteFileEntry::File(path.to_path_buf(), stat.size.unwrap())
                // } else if stat.is_dir() {
                } else {
                    RemoteFileEntry::Directory(path.to_path_buf())
                    // } else if stat.file_type().is_symlink() {
                    // unimplemented!()
                    // } else {
                    // unimplemented!()
                }
            })
            .collect())
    }
}

impl FileList {
    pub fn new(
        session: &ssh2::Session,
        sftp: &ssh2::Sftp,
        keep_hidden_files: bool,
    ) -> Result<Self> {
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

    /// Read the current local directory and populate this file list with the new local entries.
    pub fn fetch_local_files(&mut self, keep_hidden_files: bool) -> io::Result<()> {
        self.local_entries = self
            .local_directory
            .parent()
            .map(|parent| LocalFileEntry::Parent(parent.to_path_buf()))
            .into_iter()
            .chain(LocalFileEntry::read_dir(&self.local_directory)?)
            .collect();

        if !keep_hidden_files {
            self.local_entries.retain(|entry| !entry.is_hidden());
        }
        self.local_entries.sort_unstable();
        Ok(())
    }

    /// Read the current remote directory and populate this file list with the new remote entries.
    pub fn fetch_remote_files(
        &mut self,
        sftp: &ssh2::Sftp,
        keep_hidden_files: bool,
    ) -> io::Result<()> {
        self.remote_entries = self
            .remote_directory
            .parent()
            .map(|parent| RemoteFileEntry::Parent(parent.to_path_buf()))
            .into_iter()
            .chain(RemoteFileEntry::read_dir(&self.remote_directory, sftp)?)
            .collect();

        if !keep_hidden_files {
            self.remote_entries.retain(|entry| !entry.is_hidden());
        }
        self.remote_entries.sort_unstable();
        Ok(())
    }

    /// Set the current local directory and then fetch its local files.
    pub fn set_local_working_path(
        &mut self,
        path: impl AsRef<Path>,
        keep_hidden_files: bool,
    ) -> io::Result<()> {
        self.local_directory = canonicalize(path)?;
        self.fetch_local_files(keep_hidden_files)?;
        // Make sure we have a valid entry selected.
        self.apply_op_to_selected(|i| i);
        Ok(())
    }

    /// Set the current remote directory and then fetch its remote files.
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
        self.apply_op_to_selected(|i| i);
        Ok(())
    }

    /// Return the current local directory.
    pub fn get_local_working_path(&self) -> &Path {
        &self.local_directory
    }

    /// Return the current remote directory.
    pub fn get_remote_working_path(&self) -> &Path {
        &self.remote_directory
    }

    /// Return the currently selected file entry.
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

    /// Return the index of the currently selected file entry if a local file entry is selected.
    pub fn get_local_selected_index(&self) -> ListState {
        let index = match self.selected {
            SelectedFileEntryIndex::Local(i) => Some(i),
            _ => None,
        };
        let mut state = ListState::default();
        state.select(index);
        state
    }

    /// Return the index of the currently selected file entry if a remote file entry is selected.
    pub fn get_remote_selected_index(&self) -> ListState {
        let index = match self.selected {
            SelectedFileEntryIndex::Remote(i) => Some(i),
            _ => None,
        };
        let mut state = ListState::default();
        state.select(index);
        state
    }

    /// Apply `f` to the index of the currently selected file entry.
    fn apply_op_to_selected<F>(&mut self, f: F)
    where
        F: Fn(isize) -> isize,
    {
        self.selected = match self.selected {
            SelectedFileEntryIndex::Local(i) => {
                if self.local_entries.is_empty() {
                    SelectedFileEntryIndex::None
                } else {
                    let n = self.local_entries.len();
                    SelectedFileEntryIndex::Local(f(i as isize).rem_euclid(n as isize) as usize)
                }
            }
            SelectedFileEntryIndex::Remote(i) => {
                if self.remote_entries.is_empty() {
                    SelectedFileEntryIndex::None
                } else {
                    let n = self.remote_entries.len();
                    SelectedFileEntryIndex::Remote(f(i as isize).rem_euclid(n as isize) as usize)
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
        }
    }

    /// Set the currently selected file entry to the next file entry.
    pub fn next_selected(&mut self) {
        self.apply_op_to_selected(|i| i + 1)
    }

    /// Set the currently selected file entry to the previous file entry.
    pub fn prev_selected(&mut self) {
        self.apply_op_to_selected(|i| i - 1)
    }

    /// Toggle the currently selected file entry between the local and remote file entries.
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

    /// Draw the current file list.
    pub fn draw<B>(&self, frame: &mut tui::terminal::Frame<B>, rect: tui::layout::Rect)
    where
        B: tui::backend::Backend,
    {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Ratio(1, 2), Constraint::Ratio(1, 2)].as_ref())
            .split(rect);
        let (local_rect, remote_rect) = (chunks[0], chunks[1]);

        let generate_list = |title, items| {
            List::new(items)
                .block(Block::default().title(title).borders(Borders::ALL))
                .highlight_style(Style::default().bg(FILELIST_HIGHLIGHT_COLOR))
                .highlight_symbol(">>")
        };

        let title = format!("Local: {:?}", self.get_local_working_path());
        let width = (local_rect.width - 4) as usize;
        let items: Vec<_> = self
            .local_entries
            .iter()
            .map(|entry| entry.to_text(width))
            .collect();
        let mut state = self.get_local_selected_index();
        let list = generate_list(&title, items.into_iter());
        frame.render_stateful_widget(list, local_rect, &mut state);

        let title = format!("Remote: {:?}", self.get_remote_working_path());
        let width = (remote_rect.width - 4) as usize;
        let items: Vec<_> = self
            .remote_entries
            .iter()
            .map(|entry| entry.to_text(width))
            .collect();
        let mut state = self.get_remote_selected_index();
        let list = generate_list(&title, items.into_iter());
        frame.render_stateful_widget(list, remote_rect, &mut state);
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
                LocalFileEntry::File(PathBuf::from("/a/b/c/myfile.txt"), 30_000),
                LocalFileEntry::File(PathBuf::from("/a/b/c/myotherfile.dat"), 128),
                LocalFileEntry::Directory(PathBuf::from("/a/b/c/important")),
            ],
            remote_entries: vec![
                RemoteFileEntry::Parent(PathBuf::from("home/files/..")),
                RemoteFileEntry::File(PathBuf::from("home/files/pic.png"), 55_000),
                RemoteFileEntry::File(PathBuf::from("home/files/movie.mkv"), 123_000_000),
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
                "│⬅                      ││  ⬅                    │",
                "│myfile.txt    30.0 KB  ││  pic.png       55.0 KB│",
                "│myotherfile.dat 128 B  ││>>movie.mkv    123.0 MB│",
                "│important/             ││  games/               │",
                "│                       ││  trash/               │",
                "│                       ││                       │",
                "└───────────────────────┘└───────────────────────┘",
            ])
        );
    }
}
