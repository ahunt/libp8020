use std::sync::mpsc::{SendError, Sender};

use crate::protocol::{Command, Indicator, Message};
use crate::test_config::{StageCounts, TestConfig, TestStage};
use crate::ValveState;

#[repr(C)]
pub enum TestState {
    Pending,
    StartedExercise(usize),
    Finished,
}

// TODO: figure out if this is really the most FFI-friendly representation. A
// struct with index+value+enum indicating sample type might be nicer (albeit
// less rusty, I think).
#[repr(C)]
// TODO: make this wrap a common struct instead.
pub enum SampleNotification {
    AmbientPurge {
        exercise: usize,
        index: usize,
        value: f64,
    },
    AmbientSample {
        exercise: usize,
        index: usize,
        value: f64,
    },
    SpecimenPurge {
        exercise: usize,
        index: usize,
        value: f64,
    },
    SpecimenSample {
        exercise: usize,
        index: usize,
        value: f64,
    },
}

#[derive(Clone)]
enum StageResults {
    AmbientSample {
        purges: Vec<f64>,
        samples: Vec<f64>,
        config: StageCounts,
    },
    Exercise {
        purges: Vec<f64>,
        samples: Vec<f64>,
        config: StageCounts,
    },
}

impl StageResults {
    pub fn from(stage: &TestStage) -> StageResults {
        match stage {
            TestStage::AmbientSample { counts } => StageResults::AmbientSample {
                purges: Vec::with_capacity(counts.purge_count),
                samples: Vec::with_capacity(counts.sample_count),
                config: counts.clone(),
            },
            TestStage::Exercise { counts, .. } => StageResults::Exercise {
                purges: Vec::with_capacity(counts.purge_count),
                samples: Vec::with_capacity(counts.sample_count),
                config: counts.clone(),
            },
        }
    }

    pub fn is_ambient_sample(self: &Self) -> bool {
        matches!(self, StageResults::AmbientSample { .. })
    }

    pub fn is_exercise(self: &Self) -> bool {
        matches!(self, StageResults::Exercise { .. })
    }

    fn append(self: &mut Self, value: f64) {
        match self {
            StageResults::AmbientSample {
                purges,
                samples,
                config,
            }
            | StageResults::Exercise {
                purges,
                samples,
                config,
            } => {
                assert!(purges.len() < config.purge_count || samples.len() < config.sample_count);
                if purges.len() < config.purge_count {
                    purges.push(value);
                } else {
                    samples.push(value);
                }
            }
        }
    }

    fn is_complete(self: &Self) -> bool {
        match self {
            StageResults::AmbientSample {
                purges,
                samples,
                config,
            }
            | StageResults::Exercise {
                purges,
                samples,
                config,
            } => purges.len() == config.purge_count && samples.len() == config.sample_count,
        }
    }

    fn has_samples(self: &Self) -> bool {
        match self {
            StageResults::AmbientSample { samples, .. }
            | StageResults::Exercise { samples, .. } => samples.len() > 0,
        }
    }

    pub fn avg(self: &Self) -> f64 {
        match self {
            StageResults::AmbientSample { samples, .. }
            | StageResults::Exercise { samples, .. } => {
                samples.iter().sum::<f64>() / samples.len() as f64
            }
        }
    }
}

#[repr(C)]
pub enum TestNotification {
    /// StateChange indicates that the test has changed state, e.g. a new
    /// exercise was started. Note that just because a given exercise (or
    /// the entire test) was completed, it is not safe to assume that all
    /// data for that exercise (or the entire test) is available yet.
    StateChange(TestState),
    /// ExerciseResult indicates that the FF for exercise N was M.
    ExerciseResult(usize, f64),
    /// RawSample indicates a fresh reading from the PC. It is safe to assume
    /// that it was delivered 1s (plus/minus the 8020's internal delays) after
    /// the previous RawReading. This is simply the latest sample, no more,
    /// no less - i.e. it might be part of the ambient or specimen purge,
    /// or from the actually sampling period.
    // TODO: check specs for what the actual allowed range is.
    // TODO: move this into a new Device-specific callback. Raw samples are
    // available as soon as we've connected, and I'd like to see raw samples
    // prior to starting a test, as it allows you to detect if particle levels
    // in the mask haven't settled yet (I'm not convinced that this is a real
    // issue, but being able to visualise this data will help verify).
    RawSample(f64),
    /// Sample indicates a fresh sample from the 8020. This differs from
    /// RawSample in that it contains metadata about how this reading is being
    /// used and where it came from (ambient vs specimen, sample vs purge).
    /// moreover, this data is only available during a test.
    Sample(SampleNotification),
    LiveFF {
        exercise: usize,
        index: usize,
        fit_factor: f64,
    },
    /// InterimFF is the average FF at this moment in time calculated based on
    /// all data collected so far, namely average specimen particles calculated
    /// from all specimen samples during the current Exercise, divided by
    /// average ambient particles from the last AmbientSample stage.
    InterimFF { exercise: usize, fit_factor: f64 },
}

pub enum StepOutcome {
    TestComplete,
    None,
}

pub struct Test<'a> {
    config: TestConfig,
    test_callback: Option<Box<dyn Fn(&TestNotification) + 'static + std::marker::Send>>,
    // TODO: figure out a better way of representing all of this, it's a little confusing.
    current_stage: usize,
    results: Vec<StageResults>,
    // Final FFs for each exercise. Caution: for non-periodic protocols, a given
    // exercise's FF might not be calculated until several intermediate
    // exerciseshave completed.
    exercise_ffs: Vec<f64>,
    // This is NOT the same as exercise_ffs.len(), see above.
    exercises_completed: usize,
    tx_command: &'a Sender<Command>,
}

// This implementation is extremely specific to the 8020. However, it's not hard
// to imagine converting this into something device-agnostic with a little spot
// of tweaking (in conjunction with a CPC-abstraction-layer).
impl Test<'_> {
    fn create<'a>(
        config: TestConfig,
        tx_command: &'a Sender<Command>,
        test_callback: Option<Box<dyn Fn(&TestNotification) + 'static + std::marker::Send>>,
    ) -> Test<'a> {
        let stage_count = config.stages.len();
        assert!(
            stage_count >= 3,
            "invalid test config - must have at least 3 stages"
        );
        assert!(
            config.stages[0].is_ambient_sample(),
            "invalid test config - must end with ambient"
        );
        let mut results = Vec::with_capacity(stage_count);
        results.push(StageResults::from(&config.stages[0]));
        Test {
            config: config,
            test_callback: test_callback,
            current_stage: 0,
            results: results,
            exercise_ffs: Vec::with_capacity(stage_count),
            exercises_completed: 0,
            tx_command: tx_command,
        }
    }

    pub fn create_and_start<'a>(
        config: TestConfig,
        tx_command: &'a Sender<Command>,
        valve_state: &mut ValveState,
        test_callback: Option<Box<dyn Fn(&TestNotification) + 'static + std::marker::Send>>,
    ) -> Result<Test<'a>, SendError<Command>> {
        let test = Self::create(config, tx_command, test_callback);
        match valve_state {
            ValveState::Ambient | ValveState::AwaitingAmbient => (),
            ValveState::Specimen | ValveState::AwaitingSpecimen => {
                tx_command.send(Command::ValveAmbient)?;
                *valve_state = ValveState::AwaitingAmbient;
            }
        };
        tx_command.send(Command::Indicator(Indicator {
            in_progress: true,
            ..Indicator::empty()
        }))?;
        tx_command.send(Command::DisplayExercise(1))?;
        test.send_notification(&TestNotification::StateChange(TestState::StartedExercise(
            0,
        )));
        tx_command.send(Command::Beep {
            duration_deciseconds: 40,
        })?;
        Ok(test)
    }

    fn send_notification(self: &Self, notification: &TestNotification) {
        if let Some(callback) = &self.test_callback {
            callback(notification);
        }
    }

    fn last_ambient(self: &Self) -> &StageResults {
        for stage_results in self.results.iter().rev() {
            if let StageResults::AmbientSample { .. } = stage_results {
                return stage_results;
            }
        }
        panic!("encountered invalid test config with no ambient stage results")
    }

    // store_sample stores the sample without doing any further work - callers
    // must ensure to perform any followup changes to the test (e.g. by moving
    // to the next stage).
    fn store_sample(self: &mut Self, value: f64, valve_state: &mut ValveState) -> bool {
        let stage_results = self.results.last_mut().unwrap();
        match valve_state {
            ValveState::AwaitingAmbient | ValveState::AwaitingSpecimen => {
                eprintln!("discarded a sample while awaiting valve switch");
                false
            }
            ValveState::Ambient => {
                assert!(
                    stage_results.is_ambient_sample(),
                    "valve state (ambient) does not match test stage (should be AmbientSample)"
                );
                stage_results.append(value);
                true
            }
            ValveState::Specimen => {
                assert!(
                    stage_results.is_exercise(),
                    "valve state (specimen) does not match test stage (should be Exercise)"
                );
                stage_results.append(value);
                true
            }
        }
    }

    fn process_sample(
        self: &mut Self,
        value: f64,
        valve_state: &mut ValveState,
    ) -> Result<StepOutcome, SendError<Command>> {
        assert!(
            (!(self.current_stage == self.config.stages.len()
                && self.results.last().unwrap().is_complete())),
            "process_sample must not be called after test completion"
        );
        // TODO: send SampleNotifications
        if !self.store_sample(value, valve_state) {
            return Ok(StepOutcome::None);
        }
        let stage_results = self.results.last().unwrap().clone();
        if let StageResults::Exercise { samples, .. } = &stage_results {
            assert!(self.last_ambient().has_samples(), "should not be executing exercise without at least one completed ambient sample stage");
            if stage_results.has_samples() {
                let ambient_avg = self.last_ambient().avg();
                let live_ff = ambient_avg / value.max(0.01);
                self.send_notification(&TestNotification::LiveFF {
                    exercise: self.exercises_completed,
                    index: samples.len(),
                    fit_factor: live_ff,
                });
                let interim_ff = ambient_avg / stage_results.avg().max(0.01);
                self.send_notification(&TestNotification::InterimFF {
                    exercise: self.exercises_completed,
                    fit_factor: interim_ff,
                });
            }
        }
        if stage_results.is_complete() {
            if stage_results.is_ambient_sample() {
                // TODO: Calculate final per-exercise FFs.
            }

            if self.current_stage == self.config.stages.len() - 1 {
                self.tx_command.send(Command::ValveSpecimen)?;
                *valve_state = ValveState::AwaitingSpecimen;
                self.tx_command.send(Command::ClearDisplay)?;
                self.tx_command.send(Command::Beep {
                    duration_deciseconds: 99,
                })?;
                return Ok(StepOutcome::TestComplete);
            }

            self.current_stage += 1;
            self.results
                .push(StageResults::from(&self.config.stages[self.current_stage]));

            match self.results.last().unwrap() {
                StageResults::AmbientSample { .. } => {
                    eprintln!("starting ambient sample stage");
                    // We can always assume that valve_state=Sample.
                    self.tx_command.send(Command::ValveAmbient)?;
                    *valve_state = ValveState::AwaitingAmbient;
                }
                StageResults::Exercise { .. } => {
                    eprintln!("starting exercise stage");
                    if !matches!(valve_state, ValveState::Specimen) {
                        self.tx_command.send(Command::ValveSpecimen)?;
                        *valve_state = ValveState::AwaitingSpecimen;
                    }
                }
            }

            if let StageResults::Exercise { .. } = stage_results {
                self.exercises_completed += 1;
                self.send_notification(&TestNotification::StateChange(TestState::StartedExercise(
                    self.exercises_completed,
                )));
                let device_exercise = ((self.exercises_completed + 1) % 20) as u8;
                self.tx_command
                    .send(Command::DisplayExercise(device_exercise))?;
                self.tx_command.send(Command::Beep {
                    duration_deciseconds: 10,
                })?;
            }
        }
        Ok(StepOutcome::None)
    }

    pub fn step(
        self: &mut Self,
        message: Message,
        valve_state: &mut ValveState,
    ) -> Result<StepOutcome, SendError<Command>> {
        match message {
            Message::Sample(value) => {
                return self.process_sample(value, valve_state);
            }
            Message::Response(command) => match command {
                // These are already handled by the device_thread. Nevertheless, the
                // test implementation should be usable independent of the
                // 3-thread model.
                Command::ValveAmbient => {
                    *valve_state = ValveState::Ambient;
                }
                Command::ValveSpecimen => {
                    *valve_state = ValveState::Specimen;
                }
                any => {
                    eprintln!("ignoring command response: {any:?}");
                }
            },
            Message::ErrorResponse(response) => {
                eprintln!("ignoring command error response: {response:?}");
            }
            Message::UnknownError(response) => {
                eprintln!("ignoring unknown error: {response}");
            }
        }
        Ok(StepOutcome::None)
    }
}
