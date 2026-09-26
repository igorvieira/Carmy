//! `carmy console`: a terminal UI for people, over the `carmy-console/1` protocol that
//! agents speak directly.
pub mod app;
mod client;
pub mod schema;
pub mod ui;

use app::App;
use client::{Client, Incoming};
use ratatui::crossterm::event::{self, Event, KeyEventKind};
use std::{
    io::IsTerminal,
    path::Path,
    process::{Command, ExitCode},
    time::Duration,
};

/// Run the console for the application at `project`.
///
/// With `jsonl`, or when stdin or stdout is not a terminal, the application's protocol is
/// connected straight to this process's stdio, for agents and scripts.
pub fn run(project: &Path, jsonl: bool) -> ExitCode {
    if jsonl || !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        return passthrough(project);
    }
    let (mut client, incoming) = match Client::spawn(project) {
        Ok(spawned) => spawned,
        Err(e) => {
            eprintln!("error: could not start `cargo run -- console`: {e}");
            return ExitCode::FAILURE;
        }
    };
    let mut app = App::new();
    let outcome = ratatui::run(|terminal| -> std::io::Result<()> {
        while !app.quit {
            while let Ok(message) = incoming.try_recv() {
                match message {
                    Incoming::Line(line) => {
                        for request in app.apply(line) {
                            client.send(&request)?;
                        }
                    }
                    Incoming::Log(line) => app.log(line),
                    Incoming::Exited => app.exited(client.exit_code()),
                }
            }
            terminal.draw(|frame| ui::draw(frame, &app))?;
            if event::poll(Duration::from_millis(50))?
                && let Event::Key(key) = event::read()?
                && key.kind == KeyEventKind::Press
            {
                for request in app.handle_key(key) {
                    client.send(&request)?;
                }
            }
        }
        Ok(())
    });
    drop(client);
    match (outcome, &app.phase) {
        (Err(e), _) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
        (Ok(()), app::Phase::Failed(reason)) => {
            // Show the build or startup output that the TUI hid.
            for line in &app.logs {
                eprintln!("{line}");
            }
            eprintln!("error: {reason}");
            ExitCode::FAILURE
        }
        (Ok(()), _) => ExitCode::SUCCESS,
    }
}

/// `cargo run --quiet -- <args>` in `project`, sharing this process's stdio.
pub fn cargo_run(project: &Path, args: &[&str]) -> ExitCode {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let status = Command::new(cargo)
        .args(["run", "--quiet", "--"])
        .args(args)
        .current_dir(project)
        .status();
    match status {
        Ok(status) if status.success() => ExitCode::SUCCESS,
        Ok(status) => ExitCode::from(status.code().unwrap_or(1).clamp(1, 255) as u8),
        Err(e) => {
            eprintln!("error: could not run cargo: {e}");
            ExitCode::FAILURE
        }
    }
}

fn passthrough(project: &Path) -> ExitCode {
    cargo_run(project, &["console"])
}
