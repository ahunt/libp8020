use std::sync::mpsc::{SendError, Sender};

use crate::multidev::{DeviceSynchroniser, StepDirective};
use crate::protocol::{Command, Indicator, Message};
use crate::test_config::{StageCounts, TestConfig, TestStage};
use crate::ValveState;

#[repr(C)]
pub enum TestState {
    Pending,
    StartedExercise { exercise: usize },
    Finished,
}

#[repr(C)]
pub enum SampleType {
    AmbientPurge,
    AmbientSample,
    SpecimenPurge,
    SpecimenSample,
}

#[repr(C)]
pub struct SampleData {
    device_id: usize,
    exercise: usize,
    value: f64,
    sample_type: SampleType,
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

    pub fn is_ambient_sample(&self) -> bool {
        matches!(self, StageResults::AmbientSample { .. })
    }

    pub fn is_exercise(&self) -> bool {
        matches!(self, StageResults::Exercise { .. })
    }

    fn append(&mut self, value: f64) -> SampleType {
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
                    if self.is_ambient_sample() {
                        SampleType::AmbientPurge
                    } else {
                        SampleType::SpecimenPurge
                    }
                } else {
                    samples.push(value);
                    if self.is_ambient_sample() {
                        SampleType::AmbientSample
                    } else {
                        SampleType::SpecimenSample
                    }
                }
            }
        }
    }

    fn is_complete(&self) -> bool {
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

    fn has_samples(&self) -> bool {
        match self {
            StageResults::AmbientSample { samples, .. }
            | StageResults::Exercise { samples, .. } => !samples.is_empty(),
        }
    }

    pub fn avg(&self) -> f64 {
        match self {
            StageResults::AmbientSample { samples, .. }
            | StageResults::Exercise { samples, .. } => {
                let avg = samples.iter().sum::<f64>() / samples.len() as f64;
                // In theory, we might measure 0 particles throughout an exercise,
                // which would lead to an infinite fit factor. The minimum measurable
                // number of particles/cm3 is 1/n/1.67 (see Appendix D of the 8020
                // Operations and Service Manual - p57(digital)/p51(paper) of
                // https://tsi.com/getmedia/9b578bab-ace5-4820-a414-fb0a78712c67/Model_8020_8028_1980092?ext=.pdf
                // Using this as a minimum means we would calculate the highest
                // *measurable* fit-factor (with a lot of handwaving) as opposed
                // to true fit-factor in this scenario, which is probably the most
                // reasonable result.
                // Note: of course all of this is bogus for machines whose
                // flow-rates are off, or that have other issues.
                avg.max(60.0 / 100.0 / (samples.len() as f64))
            }
        }
    }

    pub fn err(&self) -> f64 {
        let avg = self.avg();
        match self {
            StageResults::AmbientSample { samples, .. }
            | StageResults::Exercise { samples, .. } => {
                // 8020 flow rate = 100cm3/min
                1.0 / f64::sqrt(avg * (samples.len() as f64) * 100.0 / 60.0)
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
    StateChange { test_state: TestState },
    /// ExerciseResult indicates the final FF for a given exercise.
    ExerciseResult {
        device_id: usize,
        exercise: usize,
        fit_factor: f64,
        error: f64,
    },
    /// Sample indicates a fresh sample from the 8020. This differs from
    /// RawSample in that it contains metadata about how this reading is being
    /// used and where it came from (ambient vs specimen, sample vs purge).
    /// moreover, this data is only available during a test.
    Sample { data: SampleData },
    LiveFF {
        device_id: usize,
        exercise: usize,
        index: usize,
        fit_factor: f64,
    },
    /// InterimFF is the average FF at this moment in time calculated based on
    /// all data collected so far, namely average specimen particles calculated
    /// from all specimen samples during the current Exercise, divided by
    /// average ambient particles from the last AmbientSample stage.
    InterimFF {
        device_id: usize,
        exercise: usize,
        fit_factor: f64,
    },
}

pub enum StepOutcome {
    TestComplete,
    None,
}

pub type TestCallback = Option<Box<dyn Fn(&TestNotification) + 'static + std::marker::Send>>;

pub struct Test<'a> {
    config: TestConfig,
    device_synchroniser: Option<DeviceSynchroniser>,
    device_id: usize,
    test_callback: TestCallback,
    // TODO: figure out a better way of representing all of this, it's a little confusing.
    current_stage: usize,
    results: Vec<StageResults>,
    // Final FFs for each exercise. Caution: for non-periodic protocols, a given
    // exercise's FF might not be calculated until several intermediate
    // exerciseshave completed.
    pub exercise_ffs: Vec<f64>,
    // This is NOT the same as exercise_ffs.len(), see above.
    exercises_completed: usize,
    tx_command: &'a Sender<Command>,
    // Series of initial commands to be sent after the first incoming sample.
    // See full explanation in Test::create_and_start().
    initial_commands: Option<Vec<Command>>,
}

// This implementation is extremely specific to the 8020. However, it's not hard
// to imagine converting this into something device-agnostic with a little spot
// of tweaking (in conjunction with a CPC-abstraction-layer).
impl Test<'_> {
    fn create(
        config: TestConfig,
        device_synchroniser: Option<DeviceSynchroniser>,
        tx_command: &Sender<Command>,
        test_callback: TestCallback,
        initial_commands: Vec<Command>,
    ) -> Test {
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
        let device_id = match &device_synchroniser {
            Some(ds) => ds.device_id,
            None => 0,
        };
        Test {
            config,
            device_synchroniser,
            device_id,
            test_callback,
            current_stage: 0,
            results,
            exercise_ffs: Vec::with_capacity(stage_count),
            exercises_completed: 0,
            tx_command,
            initial_commands: Some(initial_commands),
        }
    }

    pub fn create_and_start<'a>(
        config: TestConfig,
        device_synchroniser: Option<DeviceSynchroniser>,
        tx_command: &'a Sender<Command>,
        valve_state: &mut ValveState,
        test_callback: TestCallback,
    ) -> Result<Test<'a>, SendError<Command>> {
        // The 8020(A) can sometimes swallow incoming commands. This appears to
        // happen if it is already sending (or just about to send?) its own
        // message. This is extremely common as the device is sending a sample
        // every second. Once a test is running, this is generally not a major
        // issue as we only send commands in direct response to an incoming sample
        // (e.g. last sample for an exercise -> switch valve to next state, update
        // display, etc.). But this is a common issue when starting a test -
        // therefore we avoid immediately sending the initial series of commands
        // as they may be swallowed, and instead save them to be sent after the
        // next sample. I've never seen more than the first two commands be
        // swallowed, but if the ValveAmbient command is missed then the test
        // gets stuck because the test can't progress until it receives a first
        // ambient sample.
        let mut initial_commands = Vec::new();

        match valve_state {
            ValveState::Ambient | ValveState::AwaitingAmbient => (),
            ValveState::Specimen | ValveState::AwaitingSpecimen => {
                initial_commands.push(Command::ValveAmbient);
                *valve_state = ValveState::AwaitingAmbient;
            }
        };
        initial_commands.push(Command::ClearDisplay);
        initial_commands.push(Command::Indicator(Indicator {
            in_progress: true,
            ..Indicator::empty()
        }));
        initial_commands.push(Command::DisplayExercise(1));
        initial_commands.push(Command::Beep {
            duration_deciseconds: 40,
        });

        let test = Self::create(
            config,
            device_synchroniser,
            tx_command,
            test_callback,
            initial_commands,
        );
        test.send_notification(&TestNotification::StateChange {
            test_state: TestState::StartedExercise { exercise: 0 },
        });
        Ok(test)
    }

    fn send_notification(&self, notification: &TestNotification) {
        if let Some(callback) = &self.test_callback {
            callback(notification);
        }
    }

    fn last_ambient(&self) -> &StageResults {
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
    fn store_sample(
        &mut self,
        value: f64,
        valve_state: &mut ValveState,
    ) -> Result<Option<SampleType>, SendError<Command>> {
        let stage_results = self.results.last_mut().unwrap();
        match valve_state {
            ValveState::AwaitingAmbient => {
                // Send the appropriate command to be safe. See comment on
                // initial_commands in Test::create_and_start for context.
                // The initial_commands approach is expected to be sufficient,
                // but sending the command repeatedly makes us more robust
                // against surprises (and doing so is not expensive/harmful).
                // Failing to switch the valve is disastrous as the test will
                // never proceed. Other commands are less critical - display
                // bugs are annoying but NBD.
                self.tx_command.send(Command::ValveAmbient)?;
                eprintln!("discarded a sample while awaiting ambient valve switch");
                return Ok(None);
            }
            ValveState::AwaitingSpecimen => {
                // See command in AwaitingAmbient case above.
                self.tx_command.send(Command::ValveSpecimen)?;
                eprintln!("discarded a sample while awaiting specimen valve switch");
            }
            ValveState::Ambient => {
                assert!(
                    stage_results.is_ambient_sample(),
                    "valve state (ambient) does not match test stage (should be AmbientSample)"
                );
            }
            ValveState::Specimen => {
                assert!(
                    stage_results.is_exercise(),
                    "valve state (specimen) does not match test stage (should be Exercise)"
                );
            }
        }
        Ok(Some(stage_results.append(value)))
    }

    fn calculate_ffs(&mut self) {
        let mut iter = self.results.iter().rev();
        let ambient_samples = loop {
            match iter.next() {
                Some(StageResults::AmbientSample { samples, .. }) => {
                    break samples.iter().copied();
                }
                Some(_) => (),
                None => panic!(
                    "must not call calculate_ffs without at least two ambient stages (found 0)"
                ),
            }
        };
        let ambient_samples = ambient_samples.chain(loop {
            match iter.next() {
                Some(StageResults::AmbientSample { samples, .. }) => {
                    break samples.iter().copied();
                }
                Some(_) => (),
                None => panic!(
                    "must not call calculate_ffs without at least two ambient stages (found 0)"
                ),
            }
        });

        let mut exercise_averages_stack = Vec::new();
        for stage in self.results.iter().rev().skip(1) {
            if !matches!(stage, StageResults::Exercise { .. }) {
                break;
            }
            exercise_averages_stack.push((stage.avg(), stage.err()));
        }

        let ambients: Vec<f64> = ambient_samples.collect();
        let ambient_sum = ambients.iter().sum::<f64>();
        let ambient_avg = ambient_sum / (ambients.len() as f64);
        let ambient_err = 1.0 / f64::sqrt(ambient_sum * 100.0 / 60.0);

        while let Some((exercise_avg, exercise_err)) = exercise_averages_stack.pop() {
            let ff = ambient_avg / exercise_avg;
            eprintln!(
                "Exercise {}: FF={}±{}",
                self.exercise_ffs.len(),
                ff,
                ff * f64::sqrt(exercise_err.powi(2) + ambient_err.powi(2)),
            );
            self.send_notification(&TestNotification::ExerciseResult {
                device_id: self.device_id,
                exercise: self.exercise_ffs.len(),
                fit_factor: ff,
                // This will be completely off in the vicinity of 0 specimen particles (or max
                // FF).
                error: ff * f64::sqrt(exercise_err.powi(2) + ambient_err.powi(2)),
            });
            self.exercise_ffs.push(ff);
        }
    }

    fn process_sample(
        &mut self,
        value: f64,
        valve_state: &mut ValveState,
    ) -> Result<StepOutcome, SendError<Command>> {
        assert!(
            (!(self.current_stage == self.config.stages.len()
                && self.results.last().unwrap().is_complete())),
            "process_sample must not be called after test completion"
        );

        if let Some(initial_commands) = Option::take(&mut self.initial_commands) {
            for command in initial_commands {
                self.tx_command.send(command)?;
            }
        }

        let Some(stored_sample_type) = self.store_sample(value, valve_state)? else {
            return Ok(StepOutcome::None);
        };
        self.send_notification(&TestNotification::Sample {
            data: SampleData {
                device_id: self.device_id,
                exercise: self.exercises_completed,
                value,
                sample_type: stored_sample_type,
            },
        });

        let stage_results = self.results.last().unwrap().clone();
        if let StageResults::Exercise { samples, .. } = &stage_results {
            assert!(self.last_ambient().has_samples(), "should not be executing exercise without at least one completed ambient sample stage");
            if stage_results.has_samples() {
                let ambient_avg = self.last_ambient().avg();
                let live_ff = ambient_avg / value.max(100.0 / 60.0);
                self.send_notification(&TestNotification::LiveFF {
                    device_id: self.device_id,
                    exercise: self.exercises_completed,
                    index: samples.len(),
                    fit_factor: live_ff,
                });
                let interim_ff = ambient_avg / stage_results.avg();
                self.send_notification(&TestNotification::InterimFF {
                    device_id: self.device_id,
                    exercise: self.exercises_completed,
                    fit_factor: interim_ff,
                });
            }
        }
        if stage_results.is_complete() {
            if self.exercises_completed > 0 && stage_results.is_ambient_sample() {
                self.calculate_ffs();
            }

            if self.current_stage == self.config.stages.len() - 1 {
                self.tx_command.send(Command::ValveSpecimen)?;
                *valve_state = ValveState::AwaitingSpecimen;
                self.tx_command.send(Command::ClearDisplay)?;
                self.tx_command.send(Command::Beep {
                    duration_deciseconds: 50,
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
                if self.results.len() != self.config.stages.len() {
                    self.send_notification(&TestNotification::StateChange {
                        test_state: TestState::StartedExercise {
                            exercise: self.exercises_completed,
                        },
                    });
                    let device_exercise = ((self.exercises_completed + 1) % 20) as u8;
                    self.tx_command
                        .send(Command::DisplayExercise(device_exercise))?;
                    self.tx_command.send(Command::Beep {
                        duration_deciseconds: 10,
                    })?;
                }
            }
        }
        Ok(StepOutcome::None)
    }

    pub fn step(
        &mut self,
        message: Message,
        valve_state: &mut ValveState,
    ) -> Result<StepOutcome, SendError<Command>> {
        match message {
            Message::Sample(value) => match self
                .device_synchroniser
                .as_mut()
                .map_or(StepDirective::Proceed, |s| s.try_step())
            {
                StepDirective::Proceed => {
                    return self.process_sample(value, valve_state);
                }
                StepDirective::Skip => (),
            },
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
            // These are already handled by the device_thread. They're irrelevant for a test.
            Message::Setting(_) => (),
        }
        Ok(StepOutcome::None)
    }
}
