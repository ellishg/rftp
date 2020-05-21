use crate::connect::create_session;
use crate::file::*;
use crate::progress::Progress;
use crate::user_message::UserMessage;
use crate::utils::Result;

use clap;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ssh2;
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
    progress_bars: Vec<Arc<Progress>>,
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
            progress_bars: vec![],
            show_hidden_files: Arc::new(AtomicBool::new(show_hidden_files)),
            user_message: Arc::new(user_message),
        })
    }

    /// Work that is done on every "tick".
    pub fn tick(&mut self) -> Result<()> {
        self.progress_bars.retain(|p| !p.is_finished());
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
                if self.progress_bars.is_empty() {
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
                        if source.is_file() {
                            let dest = files
                                .get_remote_working_path()
                                .join(source.path().file_name().unwrap());
                            drop(files);
                            self.spawn_upload(source, dest);
                        } else {
                            drop(files);
                            self.user_message.report(&format!(
                                "Error: Cannot upload \"{}\" because it is a directory!",
                                source.file_name_lossy().unwrap()
                            ));
                        }
                    }
                    SelectedFileEntry::Remote(source) => {
                        if source.is_file() {
                            let dest = files
                                .get_local_working_path()
                                .join(source.path().file_name().unwrap());
                            drop(files);
                            self.spawn_download(source, dest);
                        } else {
                            drop(files);
                            self.user_message.report(&format!(
                                "Error: Cannot download \"{}\" because it is a directory!",
                                source.file_name_lossy().unwrap()
                            ));
                        }
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
                    h/j/k/l         Navigate the files.
                    Enter           Enter the selected directory.
                    Spacebar        Download/Upload the selected file.
                    z               Show/hide hidden files.
                    q               Quit.
                    Q               Force quit.
                    ?               Print this help message.",
                    clap::crate_version!()
                ));
            }
            _ => {}
        };
        Ok(())
    }

    /// Spawn a task to upload `source` to `dest`, then fetch the remote files again.
    fn spawn_upload(&mut self, source: LocalFileEntry, dest: PathBuf) {
        let source_filename = source.file_name_lossy().unwrap().to_string();
        let progress = {
            let title = format!("Uploading \"{}\"", source_filename);
            let len = source.len().unwrap();
            Arc::new(Progress::new(&title, len))
        };
        self.progress_bars.push(Arc::clone(&progress));

        let sftp = Arc::clone(&self.sftp);
        let user_message = Arc::clone(&self.user_message);
        let show_hidden_files = Arc::clone(&self.show_hidden_files);
        let files = Arc::clone(&self.files);

        thread::spawn(move || {
            let result = upload(source, dest, &sftp, &progress).and({
                let mut files = files.lock().unwrap();
                files.fetch_remote_files(&sftp, show_hidden_files.load(Ordering::Relaxed))
            });
            if let Err(err) = result {
                progress.finish();
                user_message.report(&format!(
                    "Error: Unable to upload \"{}\". {}",
                    source_filename,
                    err.to_string()
                ));
            } else {
                user_message.report(&format!("Finished uploading \"{}\".", source_filename));
            }
        });
    }

    /// Spawn a task to download `source` to `dest`, then fetch the local files again.
    fn spawn_download(&mut self, source: RemoteFileEntry, dest: PathBuf) {
        let source_filename = source.file_name_lossy().unwrap().to_string();
        let progress = {
            let title = format!("Downloading \"{}\"", source_filename);
            let len = source.len().unwrap();
            Arc::new(Progress::new(&title, len))
        };
        self.progress_bars.push(Arc::clone(&progress));

        let sftp = Arc::clone(&self.sftp);
        let user_message = Arc::clone(&self.user_message);
        let show_hidden_files = Arc::clone(&self.show_hidden_files);
        let files = Arc::clone(&self.files);

        thread::spawn(move || {
            let result = download(source, dest, &sftp, &progress).and({
                let mut files = files.lock().unwrap();
                files.fetch_local_files(show_hidden_files.load(Ordering::Relaxed))
            });
            if let Err(err) = result {
                progress.finish();
                user_message.report(&format!(
                    "Error: Unable to download \"{}\". {}",
                    source_filename,
                    err.to_string()
                ));
            } else {
                user_message.report(&format!("Finished downloading \"{}\".", source_filename));
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

        let progress_bars = self.progress_bars.iter().map(|p| p.as_ref()).collect();
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
