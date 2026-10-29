use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::config::{ClickLocation, ClickType, Config, RepeatMode};
use crate::error::InputError;
use crate::input::{InputBackend, MouseButton};

const DOUBLE_CLICK_GAP: Duration = Duration::from_millis(12);
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);
const IDLE_WAIT: Duration = Duration::from_secs(3600);

#[derive(Debug, Clone)]
pub struct ClickPlan {
    pub interval: Duration,
    pub button: MouseButton,
    pub click_type: ClickType,
    pub limit: Option<u64>,
    pub target: Option<(i32, i32)>,
}

impl ClickPlan {
    pub fn from_config(config: &Config) -> Self {
        Self {
            interval: config.interval(),
            button: config.button,
            click_type: config.click_type,
            limit: match config.repeat_mode {
                RepeatMode::Continuous => None,
                RepeatMode::Count => Some(u64::from(config.repeat_count)),
            },
            target: match config.location {
                ClickLocation::Cursor => None,
                ClickLocation::Fixed => Some((config.fixed_x, config.fixed_y)),
            },
        }
    }
}

#[derive(Debug)]
pub enum Command {
    Start(ClickPlan),
    Stop,
    Reconnect,
    Shutdown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    Requested,
    Completed,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Backend {
        name: &'static str,
        error: Option<String>,
    },
    Started,
    Progress(u64),
    Stopped {
        clicks: u64,
        reason: StopReason,
    },
}

pub struct Engine {
    commands: Sender<Command>,
    events: Receiver<Event>,
    worker: Option<JoinHandle<()>>,
}

impl Engine {
    pub fn spawn<B, F>(factory: F) -> Self
    where
        B: InputBackend + 'static,
        F: FnOnce() -> B + Send + 'static,
    {
        let (command_tx, command_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("taplet-engine".to_string())
            .spawn(move || run_worker(factory(), command_rx, event_tx))
            .expect("failed to spawn engine thread");
        Self {
            commands: command_tx,
            events: event_rx,
            worker: Some(worker),
        }
    }

    pub fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }

    pub fn drain_events(&self) -> Vec<Event> {
        self.events.try_iter().collect()
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

struct Run {
    plan: ClickPlan,
    next_due: Instant,
    clicks: u64,
    last_report: Instant,
}

fn report_backend<B: InputBackend>(
    backend: &B,
    result: &Result<(), InputError>,
    events: &Sender<Event>,
) {
    let _ = events.send(Event::Backend {
        name: backend.name(),
        error: result.as_ref().err().map(ToString::to_string),
    });
}

fn click<B: InputBackend>(backend: &mut B, plan: &ClickPlan) -> Result<(), InputError> {
    if let Some((x, y)) = plan.target {
        backend.move_to(x, y)?;
    }
    let presses = match plan.click_type {
        ClickType::Single => 1,
        ClickType::Double => 2,
    };
    for index in 0..presses {
        backend.press_button(plan.button)?;
        backend.release_button(plan.button)?;
        if index + 1 < presses {
            thread::sleep(DOUBLE_CLICK_GAP);
        }
    }
    Ok(())
}

fn finish<B: InputBackend>(backend: &mut B, run: Run, reason: StopReason, events: &Sender<Event>) {
    let _ = backend.release_all();
    let _ = events.send(Event::Stopped {
        clicks: run.clicks,
        reason,
    });
}

fn run_worker<B: InputBackend>(mut backend: B, commands: Receiver<Command>, events: Sender<Event>) {
    let connected = backend.connect();
    report_backend(&backend, &connected, &events);
    let mut backend_ok = connected.is_ok();
    let mut run: Option<Run> = None;

    loop {
        let wait = match &run {
            Some(active) => active.next_due.saturating_duration_since(Instant::now()),
            None => IDLE_WAIT,
        };

        match commands.recv_timeout(wait) {
            Ok(Command::Start(plan)) => {
                if run.is_some() {
                    continue;
                }
                if !backend_ok {
                    let retry = backend.connect();
                    report_backend(&backend, &retry, &events);
                    backend_ok = retry.is_ok();
                    if let Err(err) = retry {
                        let _ = events.send(Event::Stopped {
                            clicks: 0,
                            reason: StopReason::Failed(err.to_string()),
                        });
                        continue;
                    }
                }
                let now = Instant::now();
                run = Some(Run {
                    plan,
                    next_due: now,
                    clicks: 0,
                    last_report: now,
                });
                let _ = events.send(Event::Started);
            }
            Ok(Command::Stop) => {
                if let Some(active) = run.take() {
                    finish(&mut backend, active, StopReason::Requested, &events);
                }
            }
            Ok(Command::Reconnect) => {
                if let Some(active) = run.take() {
                    finish(&mut backend, active, StopReason::Requested, &events);
                }
                let _ = backend.disconnect();
                let result = backend.connect();
                backend_ok = result.is_ok();
                report_backend(&backend, &result, &events);
            }
            Ok(Command::Shutdown) | Err(RecvTimeoutError::Disconnected) => {
                if let Some(active) = run.take() {
                    finish(&mut backend, active, StopReason::Requested, &events);
                }
                let _ = backend.disconnect();
                break;
            }
            Err(RecvTimeoutError::Timeout) => {
                let Some(mut active) = run.take() else {
                    continue;
                };
                match click(&mut backend, &active.plan) {
                    Ok(()) => {
                        active.clicks += 1;
                        let now = Instant::now();
                        active.next_due += active.plan.interval;
                        if active.next_due + active.plan.interval < now {
                            active.next_due = now + active.plan.interval;
                        }
                        let done = active
                            .plan
                            .limit
                            .is_some_and(|limit| active.clicks >= limit);
                        if done {
                            finish(&mut backend, active, StopReason::Completed, &events);
                        } else {
                            if now.duration_since(active.last_report) >= PROGRESS_INTERVAL {
                                active.last_report = now;
                                let _ = events.send(Event::Progress(active.clicks));
                            }
                            run = Some(active);
                        }
                    }
                    Err(err) => {
                        backend_ok = false;
                        finish(
                            &mut backend,
                            active,
                            StopReason::Failed(err.to_string()),
                            &events,
                        );
                        let _ = backend.disconnect();
                        let failed: Result<(), InputError> = Err(err);
                        report_backend(&backend, &failed, &events);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct Log {
        presses: u32,
        releases: u32,
        held: bool,
    }

    struct Fake(Arc<Mutex<Log>>);

    impl InputBackend for Fake {
        fn name(&self) -> &'static str {
            "fake"
        }
        fn connect(&mut self) -> Result<(), InputError> {
            Ok(())
        }
        fn press_button(&mut self, _button: MouseButton) -> Result<(), InputError> {
            let mut log = self.0.lock().unwrap();
            log.presses += 1;
            log.held = true;
            Ok(())
        }
        fn release_button(&mut self, _button: MouseButton) -> Result<(), InputError> {
            let mut log = self.0.lock().unwrap();
            log.releases += 1;
            log.held = false;
            Ok(())
        }
        fn release_all(&mut self) -> Result<(), InputError> {
            self.0.lock().unwrap().held = false;
            Ok(())
        }
        fn disconnect(&mut self) -> Result<(), InputError> {
            Ok(())
        }
    }

    fn wait_for<F: Fn(&Event) -> bool>(engine: &Engine, matcher: F) -> Option<Event> {
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut seen = Vec::new();
        while Instant::now() < deadline {
            seen.extend(engine.drain_events());
            if let Some(found) = seen.iter().find(|event| matcher(event)) {
                return Some(found.clone());
            }
            thread::sleep(Duration::from_millis(5));
        }
        None
    }

    #[test]
    fn finite_run_completes_with_exact_count() {
        let log = Arc::new(Mutex::new(Log::default()));
        let shared = Arc::clone(&log);
        let engine = Engine::spawn(move || Fake(shared));
        engine.send(Command::Start(ClickPlan {
            interval: Duration::from_millis(5),
            button: MouseButton::Left,
            click_type: ClickType::Single,
            limit: Some(10),
        }));
        let stopped = wait_for(&engine, |event| matches!(event, Event::Stopped { .. }));
        assert_eq!(
            stopped,
            Some(Event::Stopped {
                clicks: 10,
                reason: StopReason::Completed
            })
        );
        let log = log.lock().unwrap();
        assert_eq!(log.presses, 10);
        assert_eq!(log.releases, 10);
        assert!(!log.held);
    }

    #[test]
    fn double_click_sends_two_pairs_per_cycle() {
        let log = Arc::new(Mutex::new(Log::default()));
        let shared = Arc::clone(&log);
        let engine = Engine::spawn(move || Fake(shared));
        engine.send(Command::Start(ClickPlan {
            interval: Duration::from_millis(30),
            button: MouseButton::Left,
            click_type: ClickType::Double,
            limit: Some(3),
        }));
        wait_for(&engine, |event| matches!(event, Event::Stopped { .. })).unwrap();
        assert_eq!(log.lock().unwrap().presses, 6);
    }

    #[test]
    fn stop_cancels_promptly_and_leaves_nothing_held() {
        let log = Arc::new(Mutex::new(Log::default()));
        let shared = Arc::clone(&log);
        let engine = Engine::spawn(move || Fake(shared));
        engine.send(Command::Start(ClickPlan {
            interval: Duration::from_millis(500),
            button: MouseButton::Left,
            click_type: ClickType::Single,
            limit: None,
        }));
        wait_for(&engine, |event| matches!(event, Event::Started)).unwrap();
        let asked = Instant::now();
        engine.send(Command::Stop);
        let stopped = wait_for(&engine, |event| matches!(event, Event::Stopped { .. }));
        assert!(matches!(
            stopped,
            Some(Event::Stopped {
                reason: StopReason::Requested,
                ..
            })
        ));
        assert!(asked.elapsed() < Duration::from_millis(200));
        assert!(!log.lock().unwrap().held);
    }
}
