//! Scheduler entrypoint. Owns a `HashMap<ProgramId, RunningProgram>` and
//! advances one tick per `tick_interval_ms`.

use std::collections::HashMap;
use std::sync::Arc;

use crossbeam_channel::{Receiver, Sender};
use vcli_capture::Capture;
use vcli_core::state::ProgramState;
use vcli_core::{Event, EventData, ProgramId};
use vcli_input::InputSink;
use vcli_perception::Perception;

use crate::clock::RuntimeClock;
use crate::command::SchedulerCommand;
use crate::event::EventEmitter;
use crate::program::RunningProgram;

/// Tunable knobs.
#[derive(Debug, Clone, Copy)]
pub struct SchedulerConfig {
    /// Tick cadence.
    pub tick_interval_ms: u32,
    /// Soft budget per tick.
    pub tick_budget_ms: u32,
    /// Concurrent-program cap.
    pub max_inflight: usize,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            tick_interval_ms: 100,
            tick_budget_ms: 90,
            max_inflight: 256,
        }
    }
}

/// The scheduler.
///
/// Single clock storage: `Arc<dyn RuntimeClock>` covers both wall-clock reads
/// (via `unix_ms`) and tick pacing (via `sleep_ms`). `RuntimeClock` does not
/// extend `vcli_core::Clock` — that would require trait-object upcasting,
/// which is only stable from Rust 1.86 and our MSRV is 1.75.
pub struct Scheduler {
    config: SchedulerConfig,
    capture: Box<dyn Capture>,
    input: Arc<dyn InputSink>,
    perception: Perception,
    clock: Arc<dyn RuntimeClock>,
    cmd_rx: Receiver<SchedulerCommand>,
    event: EventEmitter,
    programs: HashMap<ProgramId, RunningProgram>,
}

impl Scheduler {
    /// Construct. The daemon will call `run_until_shutdown()` from a dedicated
    /// OS thread immediately after construction.
    #[must_use]
    pub fn new(
        config: SchedulerConfig,
        capture: Box<dyn Capture>,
        input: Arc<dyn InputSink>,
        perception: Perception,
        clock: Arc<dyn RuntimeClock>,
        cmd_rx: Receiver<SchedulerCommand>,
        event_tx: Sender<Event>,
    ) -> Self {
        let event = EventEmitter::new(event_tx, Arc::clone(&clock));
        Self {
            config,
            capture,
            input,
            perception,
            clock,
            cmd_rx,
            event,
            programs: HashMap::new(),
        }
    }

    /// Drive a single tick. Public so scenario tests can step deterministically
    /// without a live wall-clock loop.
    pub fn tick_once_pub(&mut self) {
        self.drain_commands();
        self.tick_once();
    }

    /// Main loop. Ticks on each `tick_interval_ms` deadline; returns when a
    /// `Shutdown` command is observed. Does NOT emit `daemon.stopped` — that
    /// is the daemon's responsibility.
    pub fn run_until_shutdown(mut self) {
        loop {
            if self.drain_commands() {
                break;
            }
            self.tick_once();
            self.clock.sleep_ms(self.config.tick_interval_ms);
        }
    }

    /// Drain pending commands. Returns `true` if a `Shutdown` was observed.
    fn drain_commands(&mut self) -> bool {
        while let Ok(cmd) = self.cmd_rx.try_recv() {
            match cmd {
                SchedulerCommand::Shutdown => return true,
                SchedulerCommand::SubmitValidated { program_id, program } => {
                    let mut rp = RunningProgram::pending(program_id, program);
                    rp.state = ProgramState::Waiting;
                    self.event.emit(EventData::ProgramSubmitted {
                        program_id,
                        name: rp.program.name.clone(),
                    });
                    self.event.emit(EventData::ProgramStateChanged {
                        program_id,
                        from: ProgramState::Pending,
                        to: ProgramState::Waiting,
                        reason: "submitted".into(),
                    });
                    self.programs.insert(program_id, rp);
                }
                SchedulerCommand::Cancel { program_id, reason } => {
                    if let Some(rp) = self.programs.get_mut(&program_id) {
                        let from = rp.state;
                        rp.state = ProgramState::Cancelled;
                        self.event.emit(EventData::ProgramStateChanged {
                            program_id,
                            from,
                            to: ProgramState::Cancelled,
                            reason,
                        });
                    }
                }
                SchedulerCommand::Start { program_id } => {
                    if let Some(rp) = self.programs.get_mut(&program_id) {
                        let from = rp.state;
                        rp.state = ProgramState::Running;
                        rp.running_since_ms = Some(self.clock.unix_ms());
                        self.event.emit(EventData::ProgramStateChanged {
                            program_id,
                            from,
                            to: ProgramState::Running,
                            reason: "start".into(),
                        });
                    }
                }
                SchedulerCommand::ResumeRunning {
                    program_id,
                    from_step,
                    program,
                } => {
                    let mut rp = RunningProgram::pending(program_id, program);
                    rp.state = ProgramState::Running;
                    rp.running_since_ms = Some(self.clock.unix_ms());
                    rp.body_cursor = Some(from_step);
                    rp.resumed_from = Some(from_step);
                    self.event.emit(EventData::ProgramStateChanged {
                        program_id,
                        from: ProgramState::Failed,
                        to: ProgramState::Running,
                        reason: "resume".into(),
                    });
                    self.event.emit(EventData::ProgramResumed { program_id, from_step });
                    self.programs.insert(program_id, rp);
                }
            }
        }
        false
    }

    /// Placeholder tick body. Real evaluation lands in Task 17.
    fn tick_once(&mut self) {
        let _ = &self.capture;
        let _ = &self.input;
        let _ = &self.perception;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::ManualClock;
    use crossbeam_channel::unbounded;
    use std::collections::BTreeMap;
    use vcli_capture::capture::{Capture, WindowDescriptor};
    use vcli_capture::error::CaptureError;
    use vcli_core::action::{Button, Modifier};
    use vcli_core::frame::{Frame, FrameFormat};
    use vcli_core::geom::{Point, Rect};
    use vcli_core::{program::DslVersion, trigger::Trigger, Program};
    use vcli_input::error::InputError;
    use vcli_input::sink::DragSegment;

    struct StaticCapture;
    impl Capture for StaticCapture {
        fn supported_formats(&self) -> &[FrameFormat] {
            &[FrameFormat::Rgba8]
        }
        fn enumerate_windows(&self) -> Result<Vec<WindowDescriptor>, CaptureError> {
            Ok(vec![])
        }
        fn grab_screen(&mut self) -> Result<Frame, CaptureError> {
            Ok(Frame::new(
                FrameFormat::Rgba8,
                Rect { x: 0, y: 0, w: 1, h: 1 },
                4,
                std::sync::Arc::from(vec![0u8, 0, 0, 0]),
                0,
            ))
        }
        fn grab_window(&mut self, _: &WindowDescriptor) -> Result<Frame, CaptureError> {
            self.grab_screen()
        }
    }

    struct NopInput;
    impl InputSink for NopInput {
        fn mouse_move(&self, _: Point) -> Result<(), InputError> {
            Ok(())
        }
        fn click(&self, _: Point, _: Button, _: &[Modifier], _: u32) -> Result<(), InputError> {
            Ok(())
        }
        fn double_click(&self, _: Point, _: Button) -> Result<(), InputError> {
            Ok(())
        }
        fn drag(&self, _: Point, _: &[DragSegment], _: Button) -> Result<(), InputError> {
            Ok(())
        }
        fn type_text(&self, _: &str) -> Result<(), InputError> {
            Ok(())
        }
        fn key_combo(&self, _: &[Modifier], _: &str) -> Result<(), InputError> {
            Ok(())
        }
    }

    fn empty_program() -> Program {
        Program {
            version: DslVersion(DslVersion::V0_1.to_string()),
            name: "x".into(),
            id: None,
            trigger: Trigger::OnSubmit,
            predicates: BTreeMap::new(),
            watches: vec![],
            body: vec![],
            on_complete: None,
            on_fail: None,
            timeout_ms: None,
            labels: BTreeMap::new(),
            priority: vcli_core::Priority::default(),
        }
    }

    #[test]
    fn shutdown_exits_cleanly() {
        let (cmd_tx, cmd_rx) = unbounded::<SchedulerCommand>();
        let (ev_tx, ev_rx) = unbounded::<Event>();
        let clock = std::sync::Arc::new(ManualClock::new(0));
        let perc = Perception::new();
        let sched = Scheduler::new(
            SchedulerConfig::default(),
            Box::new(StaticCapture),
            std::sync::Arc::new(NopInput),
            perc,
            clock,
            cmd_rx,
            ev_tx,
        );
        let id: ProgramId = "12345678-1234-4567-8910-111213141516".parse().unwrap();
        cmd_tx
            .send(SchedulerCommand::SubmitValidated {
                program_id: id,
                program: empty_program(),
            })
            .unwrap();
        cmd_tx.send(SchedulerCommand::Shutdown).unwrap();
        let handle = std::thread::spawn(move || sched.run_until_shutdown());
        handle.join().unwrap();
        let mut kinds: Vec<String> = Vec::new();
        while let Ok(ev) = ev_rx.try_recv() {
            kinds.push(
                serde_json::to_value(&ev).unwrap()["type"]
                    .as_str()
                    .unwrap()
                    .to_string(),
            );
        }
        assert!(kinds.iter().any(|k| k == "program.submitted"));
        assert!(kinds.iter().any(|k| k == "program.state_changed"));
    }
}
