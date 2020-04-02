use crate::file::*;
use crate::progress::Progress;

use clap;
use ssh2;
use std::env;
use std::error::Error;
use std::iter::Iterator;
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use termion::event::Key;

pub struct Rftp {
    session: ssh2::Session,
    sftp: Arc<ssh2::Sftp>,
    local_files: Arc<Mutex<LocalFileList>>,
    remote_files: Arc<Mutex<RemoteFileList>>,
    is_alive: bool,
    progress_bars: Vec<Arc<Progress>>,
    selected_file: SelectedFileIndex,
    show_hidden_files: Arc<AtomicBool>,
    user_message: Arc<Mutex<Option<String>>>,
}

pub enum SelectedFileIndex {
    LocalFileIndex(usize),
    RemoteFileIndex(usize),
    None,
}

impl Rftp {
    pub fn new() -> Result<Self, Box<dyn Error>> {
        let matches = clap::clap_app!(
            rftp =>
                (@arg destination: +required)
                (@arg port: -p --port +takes_value)
                (@arg username: -u --user +takes_value +required)
        )
        .get_matches();

        let destination = matches.value_of("destination").unwrap();
        let username = matches.value_of("username").unwrap();
        let port = matches.value_of("port");
        let session = create_session(destination, username, port)?;
        let sftp = session.sftp()?;

        let show_hidden_files = false;
        let selected_file = SelectedFileIndex::None;

        let local_files = {
            let path = std::fs::canonicalize(env::current_dir()?)?;
            let mut list = LocalFileList::new(path)?;
            if !show_hidden_files {
                list.remove_hidden();
            }
            Arc::new(Mutex::new(list))
        };
        let remote_files = {
            let path = PathBuf::from("./");
            let mut list = RemoteFileList::new(path, &sftp)?;
            if !show_hidden_files {
                list.remove_hidden();
            }
            Arc::new(Mutex::new(list))
        };

        Ok(Rftp {
            session,
            sftp: Arc::new(sftp),
            local_files,
            remote_files,
            is_alive: true,
            progress_bars: vec![],
            selected_file,
            show_hidden_files: Arc::new(AtomicBool::new(show_hidden_files)),
            user_message: Arc::new(Mutex::new(None)),
        })
    }

    pub fn tick(&mut self) -> Result<(), Box<dyn Error>> {
        self.progress_bars.retain(|p| !p.is_finished());
        Ok(())
    }

    // TODO: Make async.
    pub fn on_event(&mut self, key: Key) -> Result<(), Box<dyn Error>> {
        match key {
            Key::Esc => {
                self.is_alive = false;
            }
            Key::Char('\n') => {
                match self.selected_file {
                    SelectedFileIndex::LocalFileIndex(i) => {
                        let entry = { self.local_files.lock().unwrap().get_entries()[i].clone() };
                        if entry.is_dir() {
                            {
                                self.local_files
                                    .lock()
                                    .unwrap()
                                    .set_working_path(entry.path())?;
                            }
                            Rftp::fetch_local_files(&self.local_files, &self.show_hidden_files)?;
                        } else {
                            *self.user_message.lock().unwrap() = Some(format!(
                                "Error: Cannot enter {:?} because it is not a directory!",
                                entry.path()
                            ));
                        }
                    }
                    SelectedFileIndex::RemoteFileIndex(i) => {
                        let entry = { self.remote_files.lock().unwrap().get_entries()[i].clone() };
                        if entry.is_dir() {
                            {
                                self.remote_files
                                    .lock()
                                    .unwrap()
                                    .set_working_path(entry.path());
                            }
                            Rftp::fetch_remote_files(
                                &self.remote_files,
                                &self.sftp,
                                &self.show_hidden_files,
                            )?;
                        } else {
                            *self.user_message.lock().unwrap() = Some(format!(
                                "Error: Cannot enter {:?} because it is not a directory!",
                                entry.path()
                            ));
                        }
                    }
                    SelectedFileIndex::None => {}
                };
            }
            Key::Char(' ') => {
                match self.selected_file {
                    SelectedFileIndex::LocalFileIndex(i) => {
                        let source = { self.local_files.lock().unwrap().get_entries()[i].clone() };
                        if source.is_file() {
                            let sftp = Arc::clone(&self.sftp);
                            let dest = {
                                let mut path = {
                                    self.remote_files
                                        .lock()
                                        .unwrap()
                                        .get_working_path()
                                        .to_path_buf()
                                };
                                path.push(source.file_name().unwrap());
                                path
                            };
                            let source_len = source.len()?.unwrap();
                            let source_filename = source.path().to_str().unwrap().to_string();
                            let progress =
                                Arc::new(Progress::new(source_filename.clone(), source_len));
                            self.progress_bars.push(Arc::clone(&progress));
                            let user_message = Arc::clone(&self.user_message);
                            let remote_files = Arc::clone(&self.remote_files);
                            let show_hidden_files = Arc::clone(&self.show_hidden_files);
                            tokio::spawn(async move {
                                upload(source, dest, &sftp, &progress)
                                    .await
                                    .and_then(|()| {
                                        Rftp::fetch_remote_files(
                                            remote_files.as_ref(),
                                            &sftp,
                                            &show_hidden_files,
                                        )
                                    })
                                    .and_then(|()| {
                                        *user_message.lock().unwrap() =
                                            Some(format!("Finished uploading {}", source_filename));
                                        Ok(())
                                    })
                                    .unwrap_or_else(|err| {
                                        *user_message.lock().unwrap() = Some(err.to_string());
                                    });
                            });
                        } else {
                            *self.user_message.lock().unwrap() = Some(format!(
                                "Error: Cannot upload {:?} because it is a directory",
                                source.path()
                            ));
                        }
                    }
                    SelectedFileIndex::RemoteFileIndex(i) => {
                        let source = { self.remote_files.lock().unwrap().get_entries()[i].clone() };
                        if source.is_file() {
                            let sftp = Arc::clone(&self.sftp);
                            let dest = {
                                let mut path = {
                                    self.local_files
                                        .lock()
                                        .unwrap()
                                        .get_working_path()
                                        .to_path_buf()
                                };
                                path.push(source.file_name().unwrap());
                                path
                            };
                            let source_len = source.len().unwrap();
                            let source_filename = source.path().to_str().unwrap().to_string();
                            let progress =
                                Arc::new(Progress::new(source_filename.clone(), source_len));
                            self.progress_bars.push(Arc::clone(&progress));
                            let user_message = Arc::clone(&self.user_message);
                            let local_files = Arc::clone(&self.local_files);
                            let show_hidden_files = Arc::clone(&self.show_hidden_files);
                            tokio::spawn(async move {
                                download(source, dest, &sftp, &progress)
                                    .await
                                    .and_then(|()| {
                                        Rftp::fetch_local_files(&local_files, &show_hidden_files)
                                    })
                                    .and_then(|()| {
                                        *user_message.lock().unwrap() = Some(format!(
                                            "Finished downloading {}",
                                            source_filename
                                        ));
                                        Ok(())
                                    })
                                    .unwrap_or_else(|err| {
                                        *user_message.lock().unwrap() = Some(err.to_string());
                                    });
                            });
                        } else {
                            *self.user_message.lock().unwrap() = Some(format!(
                                "Error: Cannot upload {:?} because it is a directory",
                                source.path()
                            ));
                        }
                    }
                    SelectedFileIndex::None => {}
                };
            }
            Key::Down | Key::Char('j') => {
                self.selected_file = self.selected_file.next(&|i| i + 1, &self);
            }
            Key::Up | Key::Char('k') => {
                self.selected_file = self.selected_file.next(&|i| i - 1, &self);
            }
            Key::Left | Key::Right | Key::Char('h') | Key::Char('l') => {
                self.selected_file = self.selected_file.toggle(&self);
            }
            _ => {}
        };
        Ok(())
    }

    pub fn get_progress_bars(&self) -> Vec<&Progress> {
        self.progress_bars.iter().map(|p| p.as_ref()).collect()
    }

    pub fn is_alive(&self) -> bool {
        self.is_alive
    }

    // TODO: Make async.
    pub fn fetch_local_files(
        local_files: &Mutex<LocalFileList>,
        show_hidden_files: &AtomicBool,
    ) -> std::io::Result<()> {
        if show_hidden_files.load(Ordering::Relaxed) {
            let mut local_files = local_files.lock().unwrap();
            local_files.fetch()?;
        } else {
            let mut local_files = local_files.lock().unwrap();
            local_files.fetch()?;
            local_files.remove_hidden();
        }
        Ok(())
    }

    pub fn fetch_remote_files(
        remote_files: &Mutex<RemoteFileList>,
        sftp: &ssh2::Sftp,
        show_hidden_files: &AtomicBool,
    ) -> std::io::Result<()> {
        if show_hidden_files.load(Ordering::Relaxed) {
            let mut remote_files = remote_files.lock().unwrap();
            remote_files.fetch(sftp)?;
        } else {
            let mut remote_files = remote_files.lock().unwrap();
            remote_files.fetch(sftp)?;
            remote_files.remove_hidden();
        }
        Ok(())
    }

    pub fn get_local_files(&self) -> Vec<String> {
        self.local_files
            .lock()
            .unwrap()
            .get_entries()
            .iter()
            .map(|entry| entry.to_text())
            .collect()
    }

    pub fn get_remote_filenames(&self) -> Vec<String> {
        self.remote_files
            .lock()
            .unwrap()
            .get_entries()
            .iter()
            .map(|entry| entry.to_text())
            .collect()
    }

    pub fn get_selected_file(&self) -> &SelectedFileIndex {
        &self.selected_file
    }

    pub fn get_local_working_path(&self) -> String {
        self.local_files
            .lock()
            .unwrap()
            .get_working_path()
            .to_str()
            .unwrap()
            .to_string()
    }

    pub fn get_remote_working_path(&self) -> String {
        self.remote_files
            .lock()
            .unwrap()
            .get_working_path()
            .to_str()
            .unwrap()
            .to_string()
    }

    pub fn get_user_message(&self) -> Option<String> {
        self.user_message.lock().unwrap().clone()
    }
}

impl SelectedFileIndex {
    pub fn get_local_file_index(&self) -> Option<usize> {
        match self {
            SelectedFileIndex::LocalFileIndex(i) => Some(*i),
            _ => None,
        }
    }

    pub fn get_remote_file_index(&self) -> Option<usize> {
        match self {
            SelectedFileIndex::RemoteFileIndex(i) => Some(*i),
            _ => None,
        }
    }

    fn next(&self, f: &dyn Fn(usize) -> usize, rftp: &Rftp) -> Self {
        match self {
            SelectedFileIndex::RemoteFileIndex(i) => {
                let num_files = { rftp.remote_files.lock().unwrap().len() };
                if num_files == 0 {
                    SelectedFileIndex::None.next(f, rftp)
                } else {
                    SelectedFileIndex::RemoteFileIndex(f(num_files + *i) % num_files)
                }
            }
            SelectedFileIndex::LocalFileIndex(i) => {
                let num_files = { rftp.local_files.lock().unwrap().len() };
                if num_files == 0 {
                    SelectedFileIndex::None.next(f, rftp)
                } else {
                    SelectedFileIndex::LocalFileIndex(f(num_files + *i) % num_files)
                }
            }
            SelectedFileIndex::None => {
                let num_local_files = { rftp.local_files.lock().unwrap().len() };
                let num_remote_files = { rftp.remote_files.lock().unwrap().len() };
                if num_remote_files > 0 {
                    SelectedFileIndex::RemoteFileIndex(0)
                } else if num_local_files > 0 {
                    SelectedFileIndex::LocalFileIndex(0)
                } else {
                    SelectedFileIndex::None
                }
            }
        }
    }

    fn toggle(&self, rftp: &Rftp) -> Self {
        match self {
            SelectedFileIndex::LocalFileIndex(_) => {
                let num_remote_files = { rftp.remote_files.lock().unwrap().len() };
                if num_remote_files > 0 {
                    SelectedFileIndex::RemoteFileIndex(0)
                } else {
                    SelectedFileIndex::None.toggle(rftp)
                }
            }
            SelectedFileIndex::RemoteFileIndex(_) => {
                let num_local_files = { rftp.local_files.lock().unwrap().len() };
                if num_local_files > 0 {
                    SelectedFileIndex::LocalFileIndex(0)
                } else {
                    SelectedFileIndex::None.toggle(rftp)
                }
            }
            SelectedFileIndex::None => {
                let num_local_files = { rftp.local_files.lock().unwrap().len() };
                let num_remote_files = { rftp.remote_files.lock().unwrap().len() };
                if num_remote_files > 0 {
                    SelectedFileIndex::RemoteFileIndex(0)
                } else if num_local_files > 0 {
                    SelectedFileIndex::LocalFileIndex(0)
                } else {
                    SelectedFileIndex::None
                }
            }
        }
    }
}

impl Drop for Rftp {
    fn drop(&mut self) {
        // TODO: Shutdown tcp stream.
        self.session
            .disconnect(Some(ssh2::DisconnectCode::ByApplication), "", None)
            .unwrap();
    }
}

fn create_session(
    destination: &str,
    username: &str,
    port: Option<&str>,
) -> Result<ssh2::Session, Box<dyn Error>> {
    let tcp = if let Some(port) = port {
        let port = port
            .parse::<u16>()
            .map_err(|_| "unable to parse port number")?;
        TcpStream::connect((destination, port))?
    } else {
        TcpStream::connect(destination).unwrap_or(TcpStream::connect((destination, 22))?)
    };

    let mut session = ssh2::Session::new()?;
    session.set_timeout(100000);
    session.set_compress(true);
    session.set_tcp_stream(tcp);
    session.handshake()?;

    let mut auth_methods = session.auth_methods(username)?.split(",");
    while !session.authenticated() {
        match auth_methods.next() {
            Some("password") => {
                // TODO: I can't get this to work yet.
                // struct Prompter;
                // impl ssh2::KeyboardInteractivePrompt for Prompter {
                //     fn prompt(
                //         &mut self,
                //         username: &str,
                //         instructions: &str,
                //         prompts: &[ssh2::Prompt],
                //     ) -> Vec<String> {
                //         println!("{}", username);
                //         println!("{}", instructions);
                //         prompts.iter().map(|p| p.text.to_string()).collect()
                //     }
                // }
                // let mut prompter = Prompter;
                // session.userauth_keyboard_interactive(username, &mut prompter)?;
            }
            Some("publickey") => {
                session.userauth_agent(username)?;
            }
            Some(auth_method) => {
                // TODO: Handle more authentication methods.
                eprintln!("Unknown authentication method \"{}\".", auth_method);
            }
            None => {
                session.userauth_agent(username)?;
                if !session.authenticated() {
                    return Err(Box::from("unable to authenticate session"));
                }
            }
        }
    }

    Ok(session)
}
