use anyhow::{Context, Result, bail};
use std::{
    io::{Read, Write},
    process::{Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

/// Drain both pipes while waiting, so large Git output cannot deadlock the child.
/// Commands are always invoked with argument arrays, never through a shell.
pub fn capture(command: &mut Command, input: Option<Vec<u8>>, timeout: Duration) -> Result<Output> {
    command
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().context("could not start command")?;
    let stdin = child.stdin.take();
    let writer = thread::spawn(move || {
        if let (Some(mut pipe), Some(input)) = (stdin, input) {
            let _ = pipe.write_all(&input);
        }
    });
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let out = thread::spawn(move || read_pipe(stdout));
    let err = thread::spawn(move || read_pipe(stderr));
    let deadline = Instant::now() + timeout;
    let (status, timed_out) = loop {
        if let Some(status) = child.try_wait()? {
            break (status, false);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            break (child.wait()?, true);
        }
        thread::sleep(Duration::from_millis(5));
    };
    let _ = writer.join();
    let stdout = out
        .join()
        .map_err(|_| anyhow::anyhow!("stdout reader failed"))??;
    let stderr = err
        .join()
        .map_err(|_| anyhow::anyhow!("stderr reader failed"))??;
    if timed_out {
        bail!("command exceeded its time budget");
    }
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

fn read_pipe(reader: impl Read) -> Result<Vec<u8>> {
    const LIMIT: u64 = 64 * 1024 * 1024;
    let mut bytes = Vec::new();
    reader.take(LIMIT + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > LIMIT {
        bail!("command output exceeded 64 MiB");
    }
    Ok(bytes)
}
