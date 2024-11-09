#[derive(Clone)]
enum TestStage {
    AmbientSample {
        purge_count: u8,
        sample_count: u16,
    },
    Exercise {
        name: String,
        purge_count: u8,
        sample_count: u16,
    },
}

#[derive(Clone)]
struct TestConfig {
    name: String,
    short_name: String,
    stages: Vec<TestStage>,
}

#[derive(Debug, PartialEq, Eq)]
enum ValidationError {
    InvalidConfig,
}

impl TestConfig {
    // TODO: add Option<Vec<ConfigWarning>>, and implement warning generation.
    // TODO: make ValidationError more useful.
    fn validate(self: &Self) -> Result<(), ValidationError> {
        if self.stages.len() < 3 {
            return Err(ValidationError::InvalidConfig);
        }
        if !matches!(
            self.stages.first().unwrap(),
            TestStage::AmbientSample { .. }
        ) || !matches!(self.stages.last().unwrap(), TestStage::AmbientSample { .. })
        {
            return Err(ValidationError::InvalidConfig);
        }

        {
            let mut previous_stage: Option<&TestStage> = None;
            for stage in self.stages.iter() {
                if previous_stage.is_some()
                    && matches!(stage, TestStage::AmbientSample { .. })
                    && matches!(
                        previous_stage.as_ref().unwrap(),
                        TestStage::AmbientSample { .. }
                    )
                {
                    return Err(ValidationError::InvalidConfig);
                }
                previous_stage = Some(stage);

                // Each stage must include at least one sample. Not only is an empty stage
                // nonsensical, it will make FF calculation harder to implement (robustly)
                // because we know there's always at least one sample available.
                // We don't enforce a minimum purge time - each Exercise can legitimately skip
                // purging (that's the case for the abbreviated protocols). Skipping the
                // ambient purge is probably a bad idea, but it doesn't break anything.
                let sample_count = match stage {
                    TestStage::AmbientSample { sample_count, .. }
                    | TestStage::Exercise { sample_count, .. } => sample_count,
                };
                if *sample_count < 1 {
                    return Err(ValidationError::InvalidConfig);
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate() {
        let base_config = TestConfig {
            name: "foo".to_string(),
            short_name: "bar".to_string(),
            stages: vec![],
        };

        struct TestCase<'a> {
            name: &'a str,
            input: &'a TestConfig,
            expected_result: Result<(), ValidationError>,
        }
        let tests = [
            &TestCase {
                name: "NoStages",
                input: &base_config,
                expected_result: Err(ValidationError::InvalidConfig),
            },
            &TestCase {
                name: "OneAmbientStage",
                input: &TestConfig {
                    stages: vec![TestStage::AmbientSample {
                        purge_count: 0,
                        sample_count: 1,
                    }],
                    ..base_config.clone()
                },
                expected_result: Err(ValidationError::InvalidConfig),
            },
            &TestCase {
                name: "TwoAmbientStages",
                input: &TestConfig {
                    stages: vec![
                        TestStage::AmbientSample {
                            purge_count: 0,
                            sample_count: 1,
                        },
                        TestStage::AmbientSample {
                            purge_count: 0,
                            sample_count: 1,
                        },
                    ],
                    ..base_config.clone()
                },
                expected_result: Err(ValidationError::InvalidConfig),
            },
            &TestCase {
                name: "ThreeAmbientStages",
                input: &TestConfig {
                    stages: vec![
                        TestStage::AmbientSample {
                            purge_count: 0,
                            sample_count: 1,
                        },
                        TestStage::AmbientSample {
                            purge_count: 0,
                            sample_count: 1,
                        },
                        TestStage::AmbientSample {
                            purge_count: 0,
                            sample_count: 1,
                        },
                    ],
                    ..base_config.clone()
                },
                expected_result: Err(ValidationError::InvalidConfig),
            },
            &TestCase {
                name: "TwoAmbientStagesinSequence",
                input: &TestConfig {
                    stages: vec![
                        TestStage::AmbientSample {
                            purge_count: 0,
                            sample_count: 1,
                        },
                        TestStage::Exercise {
                            name: "foo".to_string(),
                            purge_count: 0,
                            sample_count: 1,
                        },
                        TestStage::AmbientSample {
                            purge_count: 0,
                            sample_count: 1,
                        },
                        TestStage::AmbientSample {
                            purge_count: 0,
                            sample_count: 1,
                        },
                    ],
                    ..base_config.clone()
                },
                expected_result: Err(ValidationError::InvalidConfig),
            },
            &TestCase {
                name: "MinimumViableTest",
                input: &TestConfig {
                    stages: vec![
                        TestStage::AmbientSample {
                            purge_count: 0,
                            sample_count: 1,
                        },
                        TestStage::Exercise {
                            name: "foo".to_string(),
                            purge_count: 0,
                            sample_count: 1,
                        },
                        TestStage::AmbientSample {
                            purge_count: 0,
                            sample_count: 1,
                        },
                    ],
                    ..base_config.clone()
                },
                expected_result: Ok(()),
            },
            &TestCase {
                name: "SampleCountZero",
                input: &TestConfig {
                    stages: vec![
                        TestStage::AmbientSample {
                            purge_count: 0,
                            sample_count: 0,
                        },
                        TestStage::Exercise {
                            name: "foo".to_string(),
                            purge_count: 0,
                            sample_count: 1,
                        },
                        TestStage::AmbientSample {
                            purge_count: 0,
                            sample_count: 1,
                        },
                    ],
                    ..base_config.clone()
                },
                expected_result: Err(ValidationError::InvalidConfig),
            },
            &TestCase {
                name: "TwoExercisesFastTest",
                input: &TestConfig {
                    stages: vec![
                        TestStage::AmbientSample {
                            purge_count: 0,
                            sample_count: 1,
                        },
                        TestStage::Exercise {
                            name: "foo".to_string(),
                            purge_count: 0,
                            sample_count: 1,
                        },
                        TestStage::Exercise {
                            name: "foo".to_string(),
                            purge_count: 0,
                            sample_count: 1,
                        },
                        TestStage::AmbientSample {
                            purge_count: 0,
                            sample_count: 1,
                        },
                    ],
                    ..base_config.clone()
                },
                expected_result: Ok(()),
            },
        ];
        for case in tests {
            let got = case.input.validate();
            assert_eq!(
                got, case.expected_result,
                "{}: got={got:?}, want={:?}",
                case.name, case.expected_result
            );
        }
    }
}
