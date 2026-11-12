use std::{
    collections::VecDeque,
    fs,
    io::{self, BufRead, BufReader, Read},
    path::{Path, PathBuf},
    process::{Child, ExitStatus, Stdio},
    sync::{
        atomic::{AtomicU64, Ordering::Relaxed},
        mpsc::{Receiver, RecvTimeoutError, TryRecvError},
    },
    thread,
    time::{Duration, Instant},
};

use super::{ffmpeg, operation::Plan};

#[derive(Clone, Copy, Debug)]
pub struct Progress {
    pub fraction: Option<f32>,
    pub elapsed: Duration,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Exported(PathBuf),
    Cancelled,
}

pub fn export(
    ffmpeg: &Path,
    source: &Path,
    destination: PathBuf,
    plan: Plan,
    cancel: Receiver<()>,
    update: impl Fn(Progress),
) -> Result<Outcome, String> {
    validate(source, &destination)?;
    let parent = destination.parent().ok_or("Choose an output folder.")?;
    if cancelled(&cancel) {
        return Ok(Outcome::Cancelled);
    }

    let folder = tempfile::Builder::new()
        .prefix(".fflume-")
        .tempdir_in(parent)
        .map_err(|e| format!("Could not create a temporary export in that folder: {e}"))?;
    let output = tempfile::Builder::new()
        .suffix(&format!(".{}", plan.format.extension()))
        .tempfile_in(folder.path())
        .map_err(|e| format!("Could not reserve a temporary output: {e}"))?
        .into_temp_path();

    if !encode(ffmpeg, &plan, &output, &cancel, update)? {
        return Ok(Outcome::Cancelled);
    }
    publish(output, &destination)?;
    Ok(Outcome::Exported(destination))
}

pub fn encode(
    ffmpeg: &Path,
    plan: &Plan,
    output: &Path,
    cancel: &Receiver<()>,
    update: impl Fn(Progress),
) -> Result<bool, String> {
    encode_to(
        ffmpeg::command(ffmpeg),
        plan,
        output,
        output,
        cancel,
        update,
    )
}

pub fn export_frames(
    ffmpeg: &Path,
    destination: &Path,
    plan: &Plan,
    cancel: &Receiver<()>,
    update: impl Fn(Progress),
) -> Result<Option<(PathBuf, usize)>, String> {
    let parent = destination.parent().ok_or("Choose an output folder.")?;
    let staged = tempfile::Builder::new()
        .prefix(".fflume-frames-")
        .tempdir_in(parent)
        .map_err(|error| format!("Could not prepare frame export: {error}"))?;
    let extension = plan.format.extension();
    let pattern = format!("frame-%06d.{extension}");
    let mut command = ffmpeg::command(ffmpeg);
    command.current_dir(staged.path());
    let first = format!("frame-000001.{extension}");
    if !encode_to(
        command,
        plan,
        Path::new(&pattern),
        &staged.path().join(&first),
        cancel,
        update,
    )? {
        return Ok(None);
    }
    let files = fs::read_dir(staged.path())
        .and_then(|files| files.collect::<Result<Vec<_>, _>>())
        .map_err(|error| format!("Could not read exported frames: {error}"))?;
    if cancelled(cancel) {
        return Ok(None);
    }
    fs::create_dir(destination).map_err(|error| {
        if error.kind() == io::ErrorKind::AlreadyExists {
            CONFLICT.into()
        } else {
            format!("Could not create the frame folder: {error}")
        }
    })?;
    for (saved, file) in files.iter().enumerate() {
        let output = tempfile::TempPath::try_from_path(file.path()).map_err(|error| {
            format!("Saved {saved} frame(s). Could not prepare the next frame: {error}")
        })?;
        publish(output, &destination.join(file.file_name()))
            .map_err(|error| format!("Saved {saved} frame(s). {error}"))?;
    }
    Ok(Some((destination.join(first), files.len())))
}

fn encode_to(
    mut command: std::process::Command,
    plan: &Plan,
    output: &Path,
    verification: &Path,
    cancel: &Receiver<()>,
    update: impl Fn(Progress),
) -> Result<bool, String> {
    if cancelled(cancel) {
        return Ok(false);
    }
    let mut child = command
        .args([
            "-hide_banner",
            "-nostdin",
            "-v",
            "error",
            "-nostats",
            "-abort_on",
            "empty_output",
            "-stats_period",
            "0.25",
            "-progress",
            "pipe:1",
            "-y",
        ])
        .args(&plan.args)
        .arg(output)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Could not start FFmpeg: {e}"))?;

    let (Some(stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take()) else {
        stop(&mut child);
        return Err("Could not capture FFmpeg output.".into());
    };

    let position = AtomicU64::new(0);
    let (status, diagnostics) = thread::scope(|scope| {
        let spawn_readers = || -> io::Result<_> {
            let progress_reader = thread::Builder::new()
                .name("ffmpeg-progress".into())
                .spawn_scoped(scope, || read_progress(stdout, &position))?;
            let error_reader = thread::Builder::new()
                .name("ffmpeg-stderr".into())
                .spawn_scoped(scope, || read_tail(stderr))?;
            Ok((progress_reader, error_reader))
        };
        let (progress_reader, error_reader) = match spawn_readers() {
            Ok(readers) => readers,
            Err(error) => {
                stop(&mut child);
                return Err(format!("Could not start an FFmpeg output reader: {error}"));
            }
        };

        let result = wait(&mut child, cancel, &position, plan.duration, update);
        if result.is_err() {
            stop(&mut child);
        }
        let progress_result = progress_reader.join();
        let error_result = error_reader.join();
        let status = result.map_err(|e| format!("Could not wait for FFmpeg: {e}"))?;
        if status.is_none() {
            return Ok((None, Vec::new()));
        }
        progress_result
            .map_err(|_| "FFmpeg progress reader stopped unexpectedly.")?
            .map_err(|e| format!("Could not read FFmpeg progress: {e}"))?;
        let diagnostics = error_result
            .map_err(|_| "FFmpeg error reader stopped unexpectedly.")?
            .map_err(|e| format!("Could not read FFmpeg diagnostics: {e}"))?;
        Ok((status, diagnostics))
    })?;

    let Some(status) = status else {
        return Ok(false);
    };
    if cancelled(cancel) {
        return Ok(false);
    }
    if !status.success() {
        let diagnostics = String::from_utf8_lossy(&diagnostics);
        let detail = diagnostics.trim();
        return Err(if detail.is_empty() {
            format!("FFmpeg stopped with {status}.")
        } else {
            detail.to_owned()
        });
    }
    if fs::metadata(verification)
        .map_err(|e| format!("Could not verify the export: {e}"))?
        .len()
        == 0
    {
        return Err("FFmpeg did not produce a media file.".into());
    }
    Ok(true)
}

pub fn publish(output: tempfile::TempPath, destination: &Path) -> Result<(), String> {
    output.persist_noclobber(destination).map_err(|e| {
        if e.error.kind() == io::ErrorKind::AlreadyExists {
            CONFLICT.into()
        } else {
            format!("Could not save the export: {}", e.error)
        }
    })
}

pub fn validate(source: &Path, output: &Path) -> Result<(), String> {
    if !source.is_absolute() || !output.is_absolute() || output.file_name().is_none() {
        return Err("Choose an absolute path for the source and output file.".into());
    }
    if source == output {
        return Err(CONFLICT.into());
    }
    if !fs::metadata(source).is_ok_and(|meta| meta.is_file()) {
        return Err("The source file is no longer available.".into());
    }
    match fs::symlink_metadata(output) {
        Ok(_) => Err(CONFLICT.into()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("Could not check the destination: {error}")),
    }
}

pub const CONFLICT: &str = "A file with this name already exists";

pub fn cancelled(cancel: &Receiver<()>) -> bool {
    !matches!(cancel.try_recv(), Err(TryRecvError::Empty))
}

fn stop(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn wait(
    child: &mut Child,
    cancel: &Receiver<()>,
    position: &AtomicU64,
    duration: Option<f64>,
    update: impl Fn(Progress),
) -> io::Result<Option<ExitStatus>> {
    let started = Instant::now();
    loop {
        if cancelled(cancel) {
            stop(child);
            return Ok(None);
        }
        if let Some(status) = child.try_wait()? {
            return Ok(Some(status));
        }
        update(Progress {
            fraction: duration.map(|duration| {
                (position.load(Relaxed) as f64 / 1_000_000.0 / duration).clamp(0.0, 0.99) as f32
            }),
            elapsed: started.elapsed(),
        });
        match cancel.recv_timeout(Duration::from_millis(250)) {
            Err(RecvTimeoutError::Timeout) => {}
            Ok(()) | Err(RecvTimeoutError::Disconnected) => {
                stop(child);
                return Ok(None);
            }
        }
    }
}

fn read_progress(stdout: impl Read, position: &AtomicU64) -> io::Result<()> {
    for line in BufReader::new(stdout).split(b'\n') {
        if let Some(micros) = parse_progress(&line?) {
            position.fetch_max(micros, Relaxed);
        }
    }
    Ok(())
}

fn parse_progress(line: &[u8]) -> Option<u64> {
    let micros = std::str::from_utf8(line.strip_prefix(b"out_time_us=")?).ok()?;
    let micros = micros.trim().parse::<i64>().ok()?;
    Some(micros.max(0) as u64)
}

fn read_tail(mut stderr: impl Read) -> io::Result<Vec<u8>> {
    const LIMIT: usize = 8192;
    let mut tail = VecDeque::with_capacity(LIMIT);
    let mut buffer = [0; 4096];
    loop {
        let count = stderr.read(&mut buffer)?;
        if count == 0 {
            return Ok(tail.into());
        }
        let excess = (tail.len() + count).saturating_sub(LIMIT);
        tail.drain(..excess);
        tail.extend(&buffer[..count]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_uses_microseconds_and_ignores_unknown_values() {
        assert_eq!(parse_progress(b"out_time_us=1500000\r"), Some(1_500_000));
        assert_eq!(parse_progress(b"out_time_us=-100"), Some(0));
        assert_eq!(parse_progress(b"out_time_us=N/A"), None);
        assert_eq!(parse_progress(b"out_time_ms=1500000"), None);
        assert_eq!(parse_progress(b"progress=end"), None);
    }
}
