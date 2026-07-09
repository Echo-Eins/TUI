use std::ffi::OsStr;
use std::io::{self, Read};
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::os::unix::process::CommandExt;

const POLL_INTERVAL: Duration = Duration::from_millis(10);

pub fn run_command_with_timeout<I, S>(
    program: impl AsRef<OsStr>,
    args: I,
    timeout: Duration,
) -> io::Result<Output>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(unix)]
    unsafe {
        command.pre_exec(|| {
            if libc::setpgid(0, 0) == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }

    let mut child = command.spawn()?;
    let pid = child.id();
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("failed to capture child stdout"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| io::Error::other("failed to capture child stderr"))?;

    let stdout_reader = thread::spawn(move || read_all(stdout));
    let stderr_reader = thread::spawn(move || read_all(stderr));
    let deadline = Instant::now() + timeout.max(POLL_INTERVAL);

    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            terminate_process_tree(pid);
            let _ = child.kill();
            let _ = child.wait();
            let _ = stdout_reader.join();
            let _ = stderr_reader.join();
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!(
                    "command timed out after {:.1}s",
                    timeout.max(POLL_INTERVAL).as_secs_f64()
                ),
            ));
        }
        thread::sleep(POLL_INTERVAL);
    };

    let stdout = join_reader(stdout_reader)?;
    let stderr = join_reader(stderr_reader)?;
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

fn read_all(mut reader: impl Read) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn join_reader(handle: thread::JoinHandle<io::Result<Vec<u8>>>) -> io::Result<Vec<u8>> {
    handle
        .join()
        .map_err(|_| io::Error::other("command output reader panicked"))?
}

#[cfg(unix)]
fn terminate_process_tree(pid: u32) {
    unsafe {
        libc::kill(-(pid as i32), libc::SIGKILL);
    }
}

#[cfg(not(unix))]
fn terminate_process_tree(_pid: u32) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn returns_output_for_successful_command() {
        let output = run_command_with_timeout("sh", ["-c", "printf hello"], Duration::from_secs(1))
            .expect("command succeeds");
        assert!(output.status.success());
        assert_eq!(output.stdout, b"hello");
    }

    #[cfg(unix)]
    #[test]
    fn terminates_command_after_timeout() {
        let started = Instant::now();
        let error = run_command_with_timeout("sh", ["-c", "sleep 5"], Duration::from_millis(50))
            .expect_err("command should time out");
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}
