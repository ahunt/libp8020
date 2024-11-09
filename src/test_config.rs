mod builtin;

use std::str::FromStr;

#[derive(Clone, Debug, PartialEq)]
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

#[derive(Clone, Debug, PartialEq)]
struct TestConfig {
    name: String,
    short_name: String,
    stages: Vec<TestStage>,
}

#[derive(Debug, PartialEq, Eq)]
enum ValidationError {
    InvalidConfig,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ParseError<'a> {
    IoError(String),
    InvalidExerciseStage(&'a str),
    InvalidAmbientStage(&'a str),
    InvalidTestHeader(&'a str),
    Other(String),
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

    pub fn parse_from_csv(csv: &mut dyn std::io::BufRead) -> Result<TestConfig, ParseError> {
        // This could be implemented using a csv parser. But... aside from NIH,
        // I'm averse to including more deps just to save 5 lines.
        // Ooops... looks like it's actually about 20 lines (modulo
        // application-specific logic).

        let mut stages = Vec::new();
        let mut test_header: Option<(String, String)> = None;

        let mut line = String::with_capacity(64);
        loop {
            line.clear();
            let len = match csv.read_line(&mut line) {
                // EOF
                Ok(0) => {
                    break;
                }
                Ok(i) => i,
                Err(e) => return Err(ParseError::IoError(e.to_string())),
            };

            let data = line.trim();
            if data.len() == 0 || data.chars().nth(0).unwrap() == '#' {
                continue;
            }

            // Note: any additional columns are ignored for reasons of forward
            // compatibility. However, we do not allow comments in any column.
            let cols: Vec<&str> = match data
                .split(',')
                .map(|col| {
                    if col.trim().starts_with("#") {
                        Err(())
                    } else {
                        Ok(col)
                    }
                })
                .collect()
            {
                Err(_) => {
                    return Err(ParseError::Other(
                        "inline comments are prohibited".to_string(),
                    ))
                }
                Ok(res) => res,
            };

            // TODO: support field quoting.
            match cols[0] {
                "TEST" => {
                    if cols.len() < 3 {
                        return Err(ParseError::InvalidTestHeader(
                            "test header (TEST line) must contain >= 3 fields",
                        ));
                    }
                    test_header = Some((String::from(cols[1]), String::from(cols[2])));
                }
                "AMBIENT" => {
                    if cols.len() < 3 {
                        return Err(ParseError::InvalidAmbientStage(
                            "ambient stage must contain >= 3 fields",
                        ));
                    }
                    let purge_count = if let Ok(i) = u8::from_str(cols[1]) {
                        i
                    } else {
                        return Err(ParseError::InvalidAmbientStage(
                            "ambient stage purge count must be an integer between 0 and 255",
                        ));
                    };
                    // There is no need to validate counts here - that's the validator's
                    // responsibility.
                    let sample_count = if let Ok(i) = u16::from_str(cols[2]) {
                        i
                    } else {
                        return Err(ParseError::InvalidAmbientStage(
                            "ambient stage purge count must be an integer between 0 and {u16::MAX}",
                        ));
                    };
                    stages.push(TestStage::AmbientSample {
                        purge_count: purge_count,
                        sample_count: sample_count,
                    });
                }
                "EXERCISE" => {
                    if cols.len() < 4 {
                        return Err(ParseError::InvalidExerciseStage(
                            "exercise stage must contain >= 4 fields",
                        ));
                    }
                    let purge_count = if let Ok(i) = u8::from_str(cols[1]) {
                        i
                    } else {
                        return Err(ParseError::InvalidExerciseStage(
                            "exercise stage purge count must be an integer between 0 and 255",
                        ));
                    };
                    let sample_count = if let Ok(i) = u16::from_str(cols[2]) {
                        i
                    } else {
                        return Err(ParseError::InvalidExerciseStage("exercise stage purge count must be an integer between 0 and {u16::MAX}"));
                    };
                    stages.push(TestStage::Exercise {
                        name: if cols[3].len() > 0 {
                            cols[3].to_string()
                        } else {
                            "<no name>".to_string()
                        },
                        purge_count: purge_count,
                        sample_count: sample_count,
                    });
                }
                // We must fail on lines that we do not understand. This means we won't be
                // forward-compatible against new stages/commands/whatever - but we have no
                // choice because skipping commands could result in a test that doesn't match
                // the user's expectation.
                // (This differs from above, where we ignore additional fields, because we
                // assume that additional fields won't functionally alter the test. I
                // apologise in advance if my assumptions end up being incorrect.)
                cmd => {
                    let mut msg = String::from("unsupported stage/command: ");
                    msg.push_str(cmd);
                    return Err(ParseError::Other(msg));
                }
            }
        }
        if test_header.is_none() {
            return Err(ParseError::InvalidTestHeader(
                "test header (TEST line) not found",
            ));
        }

        let (name, short_name) = test_header.unwrap();
        Ok(TestConfig {
            name: name,
            short_name: short_name,
            stages: stages,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_osha_fast_ffp() {
        let mut cursor = std::io::Cursor::new(builtin::OSHA_FAST_FFP.as_bytes());
        let result = TestConfig::parse_from_csv(&mut cursor);
        assert_eq!(
            result,
            Ok(TestConfig {
                name: "\"OSHA Fast FFP (Modified Filtering Facepiece protocol)\"".to_string(),
                short_name: "osha_fast_ffp".to_string(),
                stages: vec![
                    TestStage::AmbientSample {
                        purge_count: 4,
                        sample_count: 5,
                    },
                    TestStage::Exercise {
                        purge_count: 11,
                        sample_count: 30,
                        name: "\"Bending Over\"".to_string(),
                    },
                    TestStage::Exercise {
                        purge_count: 0,
                        sample_count: 30,
                        name: "\"Talking\"".to_string(),
                    },
                    TestStage::Exercise {
                        purge_count: 0,
                        sample_count: 30,
                        name: "\"Head Side-to-Side\"".to_string(),
                    },
                    TestStage::Exercise {
                        purge_count: 0,
                        sample_count: 30,
                        name: "\"Head Up-and-Down\"".to_string(),
                    },
                    TestStage::AmbientSample {
                        purge_count: 4,
                        sample_count: 5,
                    },
                ],
            })
        );
    }

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
