//! Executable effect protocol. The child reads UTF-8 input from stdin and
//! writes little-endian u32 length-prefixed UTF-8 frames to stdout.

use std::ffi::OsString;
use std::io::{IsTerminal, Read, Write};
use std::path::PathBuf;
use std::process::{Command, ExitCode, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

use crate::cli::Cli;
use crate::engine::terminal::{get_terminal_dimensions, Terminal};

const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;

fn read_frame(reader: &mut impl Read) -> Result<Option<String>, String> {
    let mut length = [0u8; 4];
    loop {
        match reader.read(&mut length[..1]) {
            Ok(0) => return Ok(None),
            Ok(_) => break,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.to_string()),
        }
    }
    reader
        .read_exact(&mut length[1..])
        .map_err(|e| e.to_string())?;
    let length = u32::from_le_bytes(length) as usize;
    if length > MAX_FRAME_BYTES {
        return Err(format!("plugin frame exceeds {MAX_FRAME_BYTES} bytes"));
    }
    let mut bytes = vec![0; length];
    reader.read_exact(&mut bytes).map_err(|e| e.to_string())?;
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|e| e.to_string())
}

fn plugin_path(name: &str) -> Option<PathBuf> {
    let executable = format!("ttfx-effect-{name}");
    std::env::var_os("TTFX_EFFECT_PATH")
        .into_iter()
        .chain(std::env::var_os("PATH"))
        .flat_map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .map(|dir| dir.join(&executable))
        .find(|path| path.is_file())
}

pub fn run(cli: &Cli, args: &[OsString], input: &str) -> ExitCode {
    let Some(name) = args.first().and_then(|s| s.to_str()) else {
        crate::errln!("error: invalid effect name");
        return ExitCode::from(2);
    };
    if name.is_empty()
        || !name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        crate::errln!("error: invalid effect name '{name}'");
        return ExitCode::from(2);
    }
    let Some(path) = plugin_path(name) else {
        crate::errln!("error: unrecognized effect '{name}' (no ttfx-effect-{name} in TTFX_EFFECT_PATH or PATH)");
        return ExitCode::from(2);
    };
    crate::install_sigint_handler();
    let tty_output = std::io::stdout().is_terminal() && !cli.parity_dump;
    if tty_output {
        crate::install_sigterm_handler();
        crate::install_sigwinch_handler();
    }
    let mut config = cli.terminal_config();
    loop {
        let (columns, lines) = get_terminal_dimensions();
        if config.canvas_width == -1 {
            config.canvas_width = columns;
        }
        if config.canvas_height == -1 {
            config.canvas_height = lines;
        }
        let mut terminal = match Terminal::new(input, config.clone()) {
            Ok(terminal) => terminal,
            Err(e) => {
                crate::errln!("Error: {e}");
                return ExitCode::from(1);
            }
        };
        let mut child = match Command::new(&path)
            .args(&args[1..])
            .env("TTFX_EFFECT_PROTOCOL", "1")
            .env("TTFX_CANVAS_WIDTH", terminal.canvas.width.to_string())
            .env("TTFX_CANVAS_HEIGHT", terminal.canvas.height.to_string())
            .env("TTFX_FRAME_RATE", cli.frame_rate.to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
        {
            Ok(child) => child,
            Err(e) => {
                crate::errln!("error: starting plugin '{name}': {e}");
                return ExitCode::from(1);
            }
        };
        if let Some(mut stdin) = child.stdin.take() {
            if let Err(e) = stdin.write_all(input.as_bytes()) {
                let _ = child.kill();
                let _ = child.wait();
                crate::errln!("error: sending input to plugin '{name}': {e}");
                return ExitCode::from(1);
            }
        }
        let mut stdout = child.stdout.take().expect("piped plugin stdout");
        let (sender, receiver) = mpsc::sync_channel(2);
        let reader = std::thread::spawn(move || loop {
            let frame = read_frame(&mut stdout);
            let done = !matches!(frame, Ok(Some(_)));
            if sender.send(frame).is_err() || done {
                break;
            }
        });
        let mut out = std::io::stdout().lock();
        let mut result = Ok(());
        let mut resized = false;
        let mut resize_seen_at = None;
        let mut ended = false;
        let mut frames = 0u64;
        if !cli.parity_dump {
            result = terminal.prep_canvas(&mut out).map_err(|e| e.to_string());
        }
        while result.is_ok() {
            if crate::interrupted() || crate::terminated() {
                break;
            }
            if tty_output && !cli.ignore_terminal_dimensions {
                if crate::take_terminal_resize() {
                    resize_seen_at = Some(Instant::now());
                }
                if resize_seen_at
                    .is_some_and(|seen: Instant| seen.elapsed() >= Duration::from_millis(50))
                {
                    resize_seen_at = None;
                    if get_terminal_dimensions() != (columns, lines) {
                        resized = true;
                        break;
                    }
                }
            }
            match receiver.recv_timeout(Duration::from_millis(20)) {
                Ok(Ok(Some(frame))) => {
                    let write_result = if cli.parity_dump {
                        writeln!(out, "{}", frame.len())
                            .and_then(|_| out.write_all(frame.as_bytes()))
                            .and_then(|_| out.write_all(b"\n"))
                    } else {
                        if cli.frame_rate > 0 {
                            terminal.enforce_framerate();
                        }
                        terminal.print_frame(&mut out, &frame)
                    };
                    result = write_result.map_err(|e| e.to_string());
                    frames += 1;
                    if cli.max_frames.is_some_and(|limit| frames >= limit) {
                        break;
                    }
                }
                Ok(Ok(None)) => {
                    ended = true;
                    break;
                }
                Ok(Err(e)) => {
                    result = Err(format!("plugin protocol: {e}"));
                    break;
                }
                Err(RecvTimeoutError::Disconnected) => {
                    result = Err("plugin reader stopped".into());
                    break;
                }
                Err(RecvTimeoutError::Timeout) => {}
            }
        }
        if !cli.parity_dump {
            let teardown = if resized {
                terminal.reset_canvas_area(&mut out)
            } else {
                terminal.restore_cursor(&mut out, "\n")
            };
            if result.is_ok() {
                result = teardown.map_err(|e| e.to_string());
            }
        }
        let _ = out.flush();
        if !ended {
            let _ = child.kill();
        }
        drop(receiver);
        let status = child.wait();
        let _ = reader.join();
        if let Err(e) = result {
            crate::errln!("error: {e}");
            return ExitCode::from(1);
        }
        if resized {
            config.reuse_canvas = false;
            config.canvas_width = cli.canvas_width;
            config.canvas_height = cli.canvas_height;
            continue;
        }
        if crate::terminated() {
            crate::die_from_sigterm();
        }
        if crate::interrupted() {
            return ExitCode::from(1);
        }
        if cli.max_frames.is_none_or(|limit| frames < limit) && !status.is_ok_and(|s| s.success()) {
            crate::errln!("error: plugin '{name}' failed");
            return ExitCode::from(1);
        }
        if cli.parity_dump {
            crate::errln!("frames={frames}");
        }
        return ExitCode::SUCCESS;
    }
}
