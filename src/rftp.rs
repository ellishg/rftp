use crate::connect::create_session;
use crate::file::*;
use crate::progress::Progress;
use crate::user_message::UserMessage;
use crate::utils::{ErrorKind, Result};

use clap;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ssh2;
use std::collections::VecDeque;
use std::iter::Iterator;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

pub struct Rftp {
    session: ssh2::Session,
    sftp: Arc<ssh2::Sftp>,
    files: Arc<Mutex<FileList>>,
    is_alive: bool,
    progress_bars: Arc<Mutex<Vec<Arc<Progress>>>>,
    show_hidden_files: Arc<AtomicBool>,
    user_message: Arc<UserMessage>,
}

impl Rftp {
    pub fn new() -> Result<Self> {
        let matches = clap::clap_app!(
            rftp =>
                (version: clap::crate_version!())
                (author: clap::crate_authors!())
                (about: clap::crate_description!())
                (@arg destination: +required)
                (@arg port: -p --port +takes_value)
                (@arg username: -u --user +takes_value)
                (@arg verbose: -v --verbose)
        )
        .get_matches();

        let destination = matches.value_of("destination").unwrap();
        let username = {
            if let Some(username) = matches.value_of("username") {
                username.to_string()
            } else if cfg!(unix) {
                std::env::var("USER")?
            } else if cfg!(windows) {
                std::env::var("USERNAME")?
            } else {
                unimplemented!()
            }
        };
        let port = matches.value_of("port");
        let verbose = matches.is_present("verbose");
        let session = create_session(destination, &username, port, verbose)?;
        let sftp = session.sftp()?;

        let show_hidden_files = false;

        let files = Arc::new(Mutex::new(FileList::new(
            &session,
            &sftp,
            show_hidden_files,
        )?));

        let user_message = UserMessage::new();
        user_message.report("Press \"?\" for help.");

        Ok(Rftp {
            session,
            sftp: Arc::new(sftp),
            files,
            is_alive: true,
            progress_bars: Arc::new(Mutex::new(vec![])),
            show_hidden_files: Arc::new(AtomicBool::new(show_hidden_files)),
            user_message: Arc::new(user_message),
        })
    }

    /// Work that is done on every "tick".
    pub fn tick(&mut self) -> Result<()> {
        self.progress_bars
            .lock()
            .unwrap()
            .retain(|p| !p.is_finished());
        Ok(())
    }

    /// Work that is done on every key press.
    pub fn on_event(&mut self, key: KeyEvent) -> Result<()> {
        match key {
            KeyEvent {
                code: KeyCode::Char('Q'),
                modifiers: KeyModifiers::NONE,
            } => {
                self.is_alive = false;
            }
            KeyEvent {
                code: KeyCode::Char('q'),
                modifiers: KeyModifiers::NONE,
            } => {
                if self.progress_bars.lock().unwrap().is_empty() {
                    self.is_alive = false;
                } else {
                    self.user_message.report(
                        "There are still downloads/uploads in progress. Press Q to force quit.",
                    );
                }
            }
            KeyEvent {
                code: KeyCode::Enter,
                modifiers: KeyModifiers::NONE,
            } => {
                let mut files = self.files.lock().unwrap();
                match files.get_selected_entry() {
                    SelectedFileEntry::Local(entry) => {
                        if entry.is_dir() {
                            files.set_local_working_path(
                                entry.path(),
                                self.show_hidden_files.load(Ordering::Relaxed),
                            )?;
                        } else {
                            drop(files);
                            self.user_message.report(&format!(
                                "Error: Cannot enter \"{}\" because it is not a directory!",
                                entry.file_name_lossy().unwrap()
                            ));
                        }
                    }
                    SelectedFileEntry::Remote(entry) => {
                        if entry.is_dir() {
                            files.set_remote_working_path(
                                entry.path(),
                                &self.sftp,
                                self.show_hidden_files.load(Ordering::Relaxed),
                            )?;
                        } else {
                            drop(files);
                            self.user_message.report(&format!(
                                "Error: Cannot enter \"{}\" because it is not a directory!",
                                entry.file_name_lossy().unwrap()
                            ));
                        }
                    }
                    SelectedFileEntry::None => {
                        drop(files);
                        self.user_message.report("No directory selected.");
                    }
                }
            }
            KeyEvent {
                code: KeyCode::Char(' '),
                modifiers: KeyModifiers::NONE,
            } => {
                let files = self.files.lock().unwrap();
                match files.get_selected_entry() {
                    SelectedFileEntry::Local(source) => {
                        let dest = files.get_remote_working_path().to_path_buf();
                        drop(files);
                        self.spawn_upload(source, dest);
                    }
                    SelectedFileEntry::Remote(source) => {
                        let dest = files.get_local_working_path().to_path_buf();
                        drop(files);
                        self.spawn_download(source, dest);
                    }
                    SelectedFileEntry::None => {
                        drop(files);
                        self.user_message.report("No file selected.");
                    }
                }
            }
            KeyEvent {
                code: KeyCode::Char('j'),
                modifiers: KeyModifiers::NONE,
            }
            | KeyEvent {
                code: KeyCode::Down,
                modifiers: KeyModifiers::NONE,
            } => {
                self.files.lock().unwrap().next_selected();
            }
            KeyEvent {
                code: KeyCode::Char('k'),
                modifiers: KeyModifiers::NONE,
            }
            | KeyEvent {
                code: KeyCode::Up,
                modifiers: KeyModifiers::NONE,
            } => {
                self.files.lock().unwrap().prev_selected();
            }
            KeyEvent {
                code: KeyCode::Char('h'),
                modifiers: KeyModifiers::NONE,
            }
            | KeyEvent {
                code: KeyCode::Char('l'),
                modifiers: KeyModifiers::NONE,
            }
            | KeyEvent {
                code: KeyCode::Left,
                modifiers: KeyModifiers::NONE,
            }
            | KeyEvent {
                code: KeyCode::Right,
                modifiers: KeyModifiers::NONE,
            } => {
                self.files.lock().unwrap().toggle_selected();
            }
            KeyEvent {
                code: KeyCode::Char('z'),
                modifiers: KeyModifiers::NONE,
            } => {
                let show_hidden_files = !self.show_hidden_files.fetch_xor(true, Ordering::Relaxed);
                self.user_message.report(&format!(
                    "{} hidden files.",
                    if show_hidden_files { "Show" } else { "Hide" }
                ));
                let mut files = self.files.lock().unwrap();
                files.fetch_local_files(show_hidden_files)?;
                files.fetch_remote_files(&self.sftp, show_hidden_files)?;
            }
            KeyEvent {
                code: KeyCode::Char('?'),
                modifiers: KeyModifiers::NONE,
            } => {
                self.user_message.report(&format!(
                    "Controls for rftp version {}.
                    ------------------------------------------------------------
                    h/j/k/l       Navigate the files.
                    Enter         Enter the selected directory.
                    Spacebar      Download/Upload the selected file/directory.
                    z             Show/hide hidden files.
                    q             Quit.
                    Q             Force quit.
                    ?             Print this help message.",
                    clap::crate_version!()
                ));
            }
            _ => {}
        };
        Ok(())
    }

    /// Spawn a task to upload `source` into the directory `dest`, then fetch the
    /// remote files again.
    fn spawn_upload(&mut self, source: LocalFileEntry, dest: PathBuf) {
        assert!(dest.is_dir());
        let sftp = Arc::clone(&self.sftp);
        let user_message = Arc::clone(&self.user_message);
        let show_hidden_files = Arc::clone(&self.show_hidden_files);
        let files = Arc::clone(&self.files);
        let source_filename = source.file_name_lossy().unwrap().to_string();
        let progress_bars = Arc::clone(&self.progress_bars);

        let directory_progress = if source.is_dir() {
            // TODO: Add a special progress type to use here.
            let progress = {
                let title = format!("Uploading \"{}\"", source.path().display());
                Arc::new(Progress::new(&title, 0))
            };
            progress_bars.lock().unwrap().push(Arc::clone(&progress));
            Some(progress)
        } else {
            None
        };

        // Traverse the local directory in depth-first order and upload each file.
        let task = move |sftp: &ssh2::Sftp, user_message: &UserMessage| -> Result<()> {
            let mut job_queue = VecDeque::from(vec![(source, dest)]);

            while !job_queue.is_empty() {
                let (source, dest) = job_queue.pop_front().unwrap();

                match &source {
                    LocalFileEntry::File(source_path, len) => {
                        let new_remote_file_path = dest.join(source_path.file_name().unwrap());
                        if RemoteFileEntry::exists(&new_remote_file_path, sftp)? {
                            return Err(ErrorKind::RemoteFileExists(
                                new_remote_file_path.to_string_lossy().to_string(),
                            ));
                        } else {
                            let progress = {
                                let title = format!(
                                    "Uploading \"{}\"",
                                    source.file_name_lossy().unwrap().to_string()
                                );
                                Arc::new(Progress::new(&title, *len))
                            };
                            progress_bars.lock().unwrap().push(Arc::clone(&progress));

                            upload(source, new_remote_file_path, &sftp, &progress)?;
                        }
                    }
                    LocalFileEntry::Directory(source_path) => {
                        let new_remote_directory_path = dest.join(source_path.file_name().unwrap());
                        if RemoteFileEntry::exists(&new_remote_directory_path, sftp)? {
                            return Err(ErrorKind::RemoteFileExists(
                                new_remote_directory_path.to_string_lossy().to_string(),
                            ));
                        } else {
                            job_queue.extend(
                                LocalFileEntry::read_dir(&source_path)?.into_iter().map(
                                    |source_child| {
                                        (source_child, new_remote_directory_path.clone())
                                    },
                                ),
                            );
                            sftp.mkdir(&new_remote_directory_path, 0o0755)?;
                        }
                    }
                    LocalFileEntry::Symlink(path) => user_message.report(&format!(
                        "Warning: Skipping local file {} because it might be a symlink.",
                        path.display()
                    )),
                    LocalFileEntry::Parent(path) => {
                        return Err(ErrorKind::CannotUploadParent(
                            path.to_string_lossy().to_string(),
                        ))
                    }
                }
            }

            Ok(())
        };

        thread::spawn(move || {
            match task(&sftp, &user_message) {
                Ok(()) => {
                    user_message.report(&format!("Finished uploading \"{}\".", source_filename));
                }
                Err(error) => {
                    user_message.report(&format!("Error: {}.", error.to_string()));
                }
            };

            directory_progress.map(|p| p.finish());

            if let Err(error) = files
                .lock()
                .unwrap()
                .fetch_remote_files(&sftp, show_hidden_files.load(Ordering::Relaxed))
            {
                user_message.report(&format!("Error: {}.", error.to_string()));
            }
        });
    }

    /// Spawn a task to download `source` into the directory `dest`, then fetch the
    /// local files again.
    fn spawn_download(&mut self, source: RemoteFileEntry, dest: PathBuf) {
        assert!(dest.is_dir());
        let sftp = Arc::clone(&self.sftp);
        let user_message = Arc::clone(&self.user_message);
        let show_hidden_files = Arc::clone(&self.show_hidden_files);
        let files = Arc::clone(&self.files);
        let source_filename = source.file_name_lossy().unwrap().to_string();
        let progress_bars = Arc::clone(&self.progress_bars);

        let directory_progress = if source.is_dir() {
            // TODO: Add a special progress type to use here.
            let progress = {
                let title = format!("Downloading \"{}\"", source.path().display());
                Arc::new(Progress::new(&title, 0))
            };
            progress_bars.lock().unwrap().push(Arc::clone(&progress));
            Some(progress)
        } else {
            None
        };

        // Traverse the remote directory in depth-first order and download each file.
        let task = move |user_message: &UserMessage| -> Result<()> {
            let mut job_queue = VecDeque::from(vec![(source, dest)]);

            while !job_queue.is_empty() {
                let (source, dest) = job_queue.pop_front().unwrap();

                match &source {
                    RemoteFileEntry::File(source_path, len) => {
                        let new_local_file_path = dest.join(source_path.file_name().unwrap());
                        if new_local_file_path.exists() {
                            return Err(ErrorKind::LocalFileExists(
                                new_local_file_path.to_string_lossy().to_string(),
                            ));
                        } else {
                            let progress = {
                                let title = format!(
                                    "Downloading \"{}\"",
                                    source.file_name_lossy().unwrap().to_string()
                                );
                                Arc::new(Progress::new(&title, *len))
                            };
                            progress_bars.lock().unwrap().push(Arc::clone(&progress));

                            download(source, new_local_file_path, &sftp, &progress)?;
                        }
                    }
                    RemoteFileEntry::Directory(source_path) => {
                        let new_local_directory_path = dest.join(source_path.file_name().unwrap());
                        if new_local_directory_path.exists() {
                            return Err(ErrorKind::LocalFileExists(
                                new_local_directory_path.to_string_lossy().to_string(),
                            ));
                        } else {
                            job_queue.extend(
                                RemoteFileEntry::read_dir(&source_path, &sftp)?
                                    .into_iter()
                                    .map(|source_child| {
                                        (source_child, new_local_directory_path.clone())
                                    }),
                            );
                            std::fs::create_dir(new_local_directory_path)?;
                        }
                    }
                    RemoteFileEntry::Symlink(path) => user_message.report(&format!(
                        "Warning: Skipping remote file {} because it might be a symlink.",
                        path.display()
                    )),
                    RemoteFileEntry::Parent(path) => {
                        return Err(ErrorKind::CannotDownloadParent(
                            path.to_string_lossy().to_string(),
                        ))
                    }
                }
            }

            Ok(())
        };

        thread::spawn(move || {
            match task(&user_message) {
                Ok(()) => {
                    user_message.report(&format!("Finished downloading \"{}\".", source_filename));
                }
                Err(error) => {
                    user_message.report(&format!("Error: {}.", error.to_string()));
                }
            };

            directory_progress.map(|p| p.finish());

            if let Err(error) = files
                .lock()
                .unwrap()
                .fetch_local_files(show_hidden_files.load(Ordering::Relaxed))
            {
                user_message.report(&format!("Error: {}.", error.to_string()));
            }
        });
    }

    /// Return true if the user has not quit.
    pub fn is_alive(&self) -> bool {
        self.is_alive
    }

    /// Draw the current state.
    pub fn draw<B>(&self, mut frame: tui::terminal::Frame<B>)
    where
        B: tui::backend::Backend,
    {
        let rect = frame.size();
        let rect = self.user_message.draw(&mut frame, rect);

        let progress_bar_vec = self.progress_bars.lock().unwrap();
        let progress_bars = progress_bar_vec.iter().map(|p| p.as_ref()).collect();
        let rect = Progress::draw_progress_bars(progress_bars, &mut frame, rect);

        self.files.lock().unwrap().draw(&mut frame, rect);
    }
}

impl Drop for Rftp {
    fn drop(&mut self) {
        self.session
            .disconnect(Some(ssh2::DisconnectCode::ByApplication), "", None)
            .unwrap();
    }
}
