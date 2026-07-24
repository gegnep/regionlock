//! Writes to a piped child's stdin.
//!
//! A piped child can exit before it drains stdin. The applier checks
//! privilege before reading stdin; nft can reject input early. Either
//! way the writer can hit BrokenPipe. That is not a failure: the
//! child's exit status and output are the authoritative outcome.

use std::io::{self, Write};
use std::process::Child;

/// Write `bytes` to `child`'s piped stdin, tolerating BrokenPipe.
/// Takes the stdin handle and drops it on return, so the child sees
/// EOF before the caller waits. Panics if stdin is not piped.
pub fn write_stdin_tolerating_broken_pipe(child: &mut Child, bytes: &[u8]) -> io::Result<()> {
    let mut stdin = child.stdin.take().expect("stdin piped");
    if let Err(e) = stdin.write_all(bytes)
        && e.kind() != io::ErrorKind::BrokenPipe
    {
        return Err(e);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};

    #[test]
    fn broken_pipe_reads_as_success() {
        // The child closes stdin and exits without reading. The payload
        // exceeds the 64 KiB pipe buffer, so the write cannot complete
        // into the buffer and BrokenPipe is deterministic.
        let mut child = Command::new("sh")
            .args(["-c", "exec 0<&-; exit 0"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("sh spawns");
        let payload = vec![b'x'; 1 << 20];

        let result = write_stdin_tolerating_broken_pipe(&mut child, &payload);

        assert!(result.is_ok(), "BrokenPipe must read as Ok: {result:?}");
        child.wait().expect("child exits");
    }

    #[test]
    #[should_panic(expected = "stdin piped")]
    fn unpiped_stdin_panics() {
        let mut child = Command::new("sh")
            .args(["-c", ":"])
            .stdin(Stdio::null())
            .spawn()
            .expect("sh spawns");
        let _ = write_stdin_tolerating_broken_pipe(&mut child, b"payload");
    }
}
