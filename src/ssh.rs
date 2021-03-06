use failure::Error;
use failure::ResultExt;
use slog;
use ssh2;
use std::net::{SocketAddr, TcpStream};
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

/// An established SSH session.
///
/// See [`ssh2::Session`](https://docs.rs/ssh2/0.3/ssh2/struct.Session.html) in general, and
/// [`ssh2::Session#channel_session`](https://docs.rs/ssh2/0.3/ssh2/struct.Session.html#method.channel_session)
/// specifically, for how to execute commands on the remote host.
///
/// To execute a command and get its `STDOUT` output, use
/// [`Session#cmd`](struct.Session.html#method.cmd).
pub struct Session {
    ssh: ssh2::Session,
}

impl Session {
    pub(crate) fn connect(
        log: &slog::Logger,
        username: &str,
        addr: SocketAddr,
        key: &Path,
        timeout: Option<Duration>,
    ) -> Result<Self, Error> {
        // TODO: instead of max time, keep trying as long as instance is still active
        let start = Instant::now();
        let tcp = loop {
            match TcpStream::connect_timeout(&addr, Duration::from_secs(3)) {
                Ok(s) => break s,
                Err(e) => {
                    if let Some(to) = timeout {
                        if start.elapsed() <= to {
                            thread::sleep(Duration::from_secs(1));
                        } else {
                            Err(Error::from(e).context("failed to connect to ssh port"))?;
                        }
                    } else {
                        if start.elapsed() > Duration::from_secs(30) {
                            warn!(log, "still can't ssh to {}: {:?}", addr, e);
                        }
                    }
                }
            }
        };

        let mut sess = ssh2::Session::new().context("libssh2 not available")?;
        sess.set_tcp_stream(tcp);
        sess.handshake()
            .context("failed to perform ssh handshake")?;
        sess.userauth_pubkey_file(username, None, key, None)
            .context("failed to authenticate ssh session")?;

        Ok(Session { ssh: sess })
    }

    /// Issue the given command and return the command's raw standard output.
    pub fn cmd_raw(&self, cmd: &str) -> Result<Vec<u8>, Error> {
        use std::io::Read;

        let mut channel = self
            .ssh
            .channel_session()
            .map_err(Error::from)
            .map_err(|e| {
                e.context(format!(
                    "failed to create ssh channel for command '{}'",
                    cmd
                ))
            })?;

        channel
            .exec(cmd)
            .map_err(Error::from)
            .map_err(|e| e.context(format!("failed to execute command '{}'", cmd)))?;

        channel
            .send_eof()
            .map_err(Error::from)
            .map_err(|e| e.context(format!("failed to finish command '{}'", cmd)))?;

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        // NOTE: the loop is needed because libssh2 can return reads of size 0 without EOF
        // https://www.libssh2.org/libssh2_channel_read_ex.html
        // NOTE: we must read from *both* stdout and stderr. EOF is only sent when they're both
        // drained.
        while !channel.eof() {
            channel
                .read_to_end(&mut stdout)
                .map_err(Error::from)
                .map_err(|e| e.context(format!("failed to read stdout of command '{}'", cmd)))?;
            channel
                .stderr()
                .read_to_end(&mut stderr)
                .map_err(Error::from)
                .map_err(|e| e.context(format!("failed to read stderr of command '{}'", cmd)))?;
        }

        channel
            .wait_close()
            .map_err(Error::from)
            .map_err(|e| e.context(format!("command '{}' never completed", cmd)))?;

        // TODO: check channel.exit_status()
        // TODO: return stderr as well?
        drop(stderr);
        Ok(stdout)
    }

    /// Issue the given command and return the command's standard output.
    pub fn cmd(&self, cmd: &str) -> Result<String, Error> {
        Ok(String::from_utf8(self.cmd_raw(cmd)?)?)
    }
}

use std::ops::{Deref, DerefMut};
impl Deref for Session {
    type Target = ssh2::Session;
    fn deref(&self) -> &Self::Target {
        &self.ssh
    }
}

impl DerefMut for Session {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.ssh
    }
}
