use std::io;
use std::sync::atomic::Ordering;
use std::time::Duration;

use muxy_app_core::{Direction, PaneId};
use muxy_protocol::{ChannelId, CursorShape};
use ratatui::crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute,
};

use crate::state::Result;
use crate::{
    input, render,
    terminal::{self, Host},
    worker::{Action, Worker, lock},
};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum Overlay {
    #[default]
    None,
    Projects(usize),
    Sessions(usize),
    Help,
    Confirm(PaneId),
}

pub(crate) fn run() -> Result {
    terminal::require_interactive().map_err(|error| error.to_string())?;
    let profile = muxy_core::dirs::muxy_dir().map_err(|error| error.to_string())?;
    let _lock = terminal::singleton(&profile).map_err(|error| error.to_string())?;
    let executable = std::env::current_exe()
        .map_err(|error| error.to_string())?
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let _lease = muxy_client::local::bundle::acquire_runtime(&executable)
        .map_err(|error| error.to_string())?;
    let mut host = Host::enter().map_err(|error| error.to_string())?;
    let viewport = host.terminal.size().map_err(|error| error.to_string())?;
    let worker = Worker::start(
        profile,
        executable,
        ratatui::layout::Rect::new(0, 0, viewport.width, viewport.height),
    )?;
    let result = events(&mut host, &worker);
    drop(host);
    drop(worker);
    result
}

fn events(host: &mut Host, worker: &Worker) -> Result {
    let mut prefix = false;
    let mut overlay = Overlay::None;
    let mut focus = None;
    let mut shape = None;
    loop {
        if host.stop.load(Ordering::Acquire) {
            return Ok(());
        }
        host.resume_if_needed().map_err(|error| error.to_string())?;
        let size = host.terminal.size().map_err(|error| error.to_string())?;
        *lock(&worker.viewport) = ratatui::layout::Rect::new(0, 0, size.width, size.height);
        {
            let shared = lock(&worker.shared);
            if shared.done {
                return if shared.failed {
                    Err(shared.message.clone())
                } else {
                    Ok(())
                };
            }
        }
        report_focus(worker, &mut focus);
        let next_shape = {
            let shared = lock(&worker.shared);
            shared
                .state
                .as_ref()
                .and_then(|state| state.tab())
                .and_then(|tab| shared.views.get(&tab.focus))
                .map(|view| view.grid.cursor.shape)
        };
        if shape != next_shape {
            shape = next_shape;
            let style = match shape {
                Some(CursorShape::Bar) => cursor::SetCursorStyle::SteadyBar,
                Some(CursorShape::Underline) => cursor::SetCursorStyle::SteadyUnderScore,
                _ => cursor::SetCursorStyle::SteadyBlock,
            };
            execute!(io::stdout(), style).map_err(|error| error.to_string())?;
        }
        host.terminal
            .draw(|frame| render::draw(frame, &lock(&worker.shared), prefix, &overlay))
            .map_err(|error| error.to_string())?;
        if !event::poll(Duration::from_millis(16)).map_err(|error| error.to_string())? {
            continue;
        }
        let result = match event::read().map_err(|error| error.to_string())? {
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                key_event(key, worker, &mut prefix, &mut overlay)
            }
            Event::Paste(text) => {
                prefix = false;
                if overlay == Overlay::None {
                    send_text(worker, |modes| input::paste(text, modes.bracketed_paste))
                } else {
                    Ok(())
                }
            }
            Event::FocusGained => host_focus(worker, true),
            Event::FocusLost => host_focus(worker, false),
            Event::Resize(_, _) | Event::Mouse(_) | Event::Key(_) => Ok(()),
        };
        if let Err(error) = result {
            lock(&worker.shared).message = error;
        }
    }
}

fn key_event(key: KeyEvent, worker: &Worker, prefix: &mut bool, overlay: &mut Overlay) -> Result {
    if *overlay != Overlay::None {
        return picker_key(key, worker, overlay);
    }
    let control_b = key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char('b' | 'B'));
    if *prefix {
        *prefix = false;
        if control_b {
            return send_text(worker, |_| vec![0x02]);
        }
        let action = match key.code {
            KeyCode::Char('c') => Action::New(None),
            KeyCode::Char('n') => Action::CycleTab(true),
            KeyCode::Char('p') => Action::CycleTab(false),
            KeyCode::Char(number @ '0'..='9') => Action::SelectTab(number as usize - '0' as usize),
            KeyCode::Char('%') => Action::New(Some(Direction::Right)),
            KeyCode::Char('"') => Action::New(Some(Direction::Down)),
            KeyCode::Left | KeyCode::Right | KeyCode::Up | KeyCode::Down => {
                let direction = match key.code {
                    KeyCode::Left => Direction::Left,
                    KeyCode::Right => Direction::Right,
                    KeyCode::Up => Direction::Up,
                    _ => Direction::Down,
                };
                if key.modifiers.contains(KeyModifiers::CONTROL) {
                    Action::Resize(direction)
                } else {
                    Action::Focus(direction)
                }
            }
            KeyCode::Char('z') => Action::Zoom,
            KeyCode::Char('d') => Action::Detach,
            KeyCode::Char('s') => {
                *overlay = Overlay::Projects(0);
                return Ok(());
            }
            KeyCode::Char('w') => {
                *overlay = Overlay::Sessions(0);
                lock(&worker.shared).sessions.clear();
                return worker.send(Action::ListSessions);
            }
            KeyCode::Char('?') => {
                *overlay = Overlay::Help;
                return Ok(());
            }
            KeyCode::Char('x') => {
                let shared = lock(&worker.shared);
                let id = shared
                    .state
                    .as_ref()
                    .and_then(|state| state.tab())
                    .map(|tab| tab.focus)
                    .ok_or("No pane to close")?;
                let confirm = shared.views.get(&id).is_some_and(|view| {
                    !view.ended
                        && view
                            .process
                            .as_ref()
                            .is_none_or(|process| !process.is_shell)
                });
                if confirm {
                    *overlay = Overlay::Confirm(id);
                    return Ok(());
                }
                Action::Close(id)
            }
            _ => return Ok(()),
        };
        worker.send(action)
    } else if control_b {
        *prefix = true;
        Ok(())
    } else {
        send_text(worker, |modes| {
            input::encode(key, modes).unwrap_or_default()
        })
    }
}

fn picker_key(key: KeyEvent, worker: &Worker, overlay: &mut Overlay) -> Result {
    if key.code == KeyCode::Esc {
        *overlay = Overlay::None;
        return Ok(());
    }
    match *overlay {
        Overlay::None => {}
        Overlay::Help => {
            if matches!(key.code, KeyCode::Enter | KeyCode::Char('q')) {
                *overlay = Overlay::None;
            }
        }
        Overlay::Confirm(id) => match key.code {
            KeyCode::Char('y' | 'Y') | KeyCode::Enter => {
                *overlay = Overlay::None;
                worker.send(Action::Close(id))?;
            }
            KeyCode::Char('n' | 'N') => *overlay = Overlay::None,
            _ => {}
        },
        Overlay::Projects(mut index) | Overlay::Sessions(mut index) => {
            let shared = lock(&worker.shared);
            let projects = matches!(*overlay, Overlay::Projects(_));
            let count = if projects {
                shared
                    .catalog
                    .as_ref()
                    .map_or(0, |catalog| catalog.projects.len())
            } else {
                shared.sessions.len()
            };
            index = index.min(count.saturating_sub(1));
            match key.code {
                KeyCode::Up | KeyCode::Char('k') => index = index.saturating_sub(1),
                KeyCode::Down | KeyCode::Char('j') => {
                    index = (index + 1).min(count.saturating_sub(1));
                }
                KeyCode::Enter => {
                    let action = if projects {
                        shared
                            .catalog
                            .as_ref()
                            .and_then(|catalog| catalog.projects.get(index))
                            .map(|project| Action::Open(project.id))
                    } else {
                        shared.sessions.get(index).cloned().map(Action::Existing)
                    };
                    drop(shared);
                    if let Some(action) = action {
                        worker.send(action)?;
                        *overlay = Overlay::None;
                    }
                    return Ok(());
                }
                _ => {}
            }
            *overlay = if projects {
                Overlay::Projects(index)
            } else {
                Overlay::Sessions(index)
            };
        }
    }
    Ok(())
}

fn send_text(worker: &Worker, bytes: impl FnOnce(muxy_protocol::Modes) -> Vec<u8>) -> Result {
    let shared = lock(&worker.shared);
    let Some(view) = shared
        .state
        .as_ref()
        .and_then(|state| state.tab())
        .and_then(|tab| shared.views.get(&tab.focus))
        .filter(|view| !view.ended)
    else {
        return Ok(());
    };
    let Some((client, channel)) = shared.client.clone().zip(view.channel) else {
        return Ok(());
    };
    let modes = view.grid.modes;
    drop(shared);
    let bytes = bytes(modes);
    if !bytes.is_empty() {
        worker.input.send(client, channel, bytes)?;
    }
    Ok(())
}

fn host_focus(worker: &Worker, gained: bool) -> Result {
    let shared = lock(&worker.shared);
    let Some(view) = shared
        .state
        .as_ref()
        .and_then(|state| state.tab())
        .and_then(|tab| shared.views.get(&tab.focus))
        .filter(|view| view.input.focus_events && !view.ended)
    else {
        return Ok(());
    };
    let Some((client, channel)) = shared.client.clone().zip(view.channel) else {
        return Ok(());
    };
    drop(shared);
    worker.input.send(
        client,
        channel,
        if gained {
            b"\x1b[I".to_vec()
        } else {
            b"\x1b[O".to_vec()
        },
    )
}

fn report_focus(worker: &Worker, previous: &mut Option<ChannelId>) {
    let shared = lock(&worker.shared);
    let next = shared
        .state
        .as_ref()
        .and_then(|state| state.tab())
        .and_then(|tab| shared.views.get(&tab.focus))
        .filter(|view| view.input.focus_events && !view.ended)
        .and_then(|view| view.channel);
    if *previous == next {
        return;
    }
    let Some(client) = shared.client.clone() else {
        *previous = None;
        return;
    };
    let old = previous.and_then(|channel| {
        shared
            .views
            .values()
            .any(|view| view.channel == Some(channel) && view.input.focus_events && !view.ended)
            .then_some(channel)
    });
    drop(shared);
    if let Some(channel) = old {
        let _ = worker
            .input
            .send(client.clone(), channel, b"\x1b[O".to_vec());
    }
    if let Some(channel) = next {
        let _ = worker.input.send(client, channel, b"\x1b[I".to_vec());
    }
    *previous = next;
}
