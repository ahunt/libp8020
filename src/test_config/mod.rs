pub mod builtin;

use std::str::FromStr;

#[derive(Clone, Debug, PartialEq)]
pub struct StageCounts {
    pub purge_count: usize,
    pub sample_count: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TestStage {
    AmbientSample { counts: StageCounts },
    Exercise { name: String, counts: StageCounts },
}

impl TestStage {
    pub fn is_ambient_sample(self: &Self) -> bool {
        matches!(self, TestStage::AmbientSample { .. })
    }

    pub fn is_exercise(self: &Self) -> bool {
        matches!(self, TestStage::Exercise { .. })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TestConfig {
    pub name: String,
    pub short_name: String,
    pub stages: Vec<TestStage>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ValidationError {
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

const PARSE_ERROR_MESSAGE_BAD_LEADING_QUOTATION: &str = r#"Quotation marks must occur immediately after token separator ('foo,"bar"' is OK, 'foo, "bar"' and 'foo,b"bar" are not)."#;
const PARSE_ERROR_MESSAGE_BAD_TRAILING_QUOTATION: &str = r#"Separator must occur immediately after close of quotation marks ('"foo",...' is OK, '"foo" ,...' and '"foo"bar,' are not)"#;
const PARSE_ERROR_MESSAGE_UNCLOSED_QUOTATION: &str = "All quotations must be closed";
const PARSE_ERROR_MESSAGE_UNQUOTED_HASH: &str = r##"Raw hash symbols (#) are not allowed inline, enclose the token (cell) in quotes if necessary, e.g. "#ok" or "also #ok""##;

// Reusing an existing CSV parser would be the sensible approach, but... full
// CSV support simply isn't necessary. (It's not hard to change this decision in
// future if necessary anyway.)
fn tokenise_line<'a, 'b>(line: &'a str) -> Result<Vec<String>, ParseError<'b>> {
    enum LineState {
        Normal,
        InQuote,
    }

    let mut iter = line.chars().peekable();
    let mut out = Vec::new();
    out.push(String::new());
    let mut current_token = out.first_mut().unwrap();
    let mut state = LineState::Normal;
    if Some(&'#') == iter.peek() {
        return Ok(vec![line.to_string()]);
    }
    loop {
        match iter.next() {
            Some(',') => match state {
                LineState::Normal => {
                    out.push(String::new());
                    current_token = out.last_mut().unwrap();
                }
                LineState::InQuote => {
                    current_token.push(',');
                }
            },
            Some('"') => match state {
                LineState::Normal => {
                    if current_token.len() > 0 {
                        return Err(ParseError::Other(
                            PARSE_ERROR_MESSAGE_BAD_LEADING_QUOTATION.to_string(),
                        ));
                    }
                    state = LineState::InQuote;
                }
                LineState::InQuote => {
                    let Some(next) = iter.peek() else {
                        state = LineState::Normal;
                        break;
                    };
                    if *next == '"' {
                        current_token.push(*next);
                        iter.next();
                    } else if *next == ',' {
                        state = LineState::Normal
                    } else {
                        return Err(ParseError::Other(
                            PARSE_ERROR_MESSAGE_BAD_TRAILING_QUOTATION.to_string(),
                        ));
                    }
                }
            },
            Some('#') => match state {
                LineState::Normal => {
                    return Err(ParseError::Other(
                        PARSE_ERROR_MESSAGE_UNQUOTED_HASH.to_string(),
                    ));
                }
                LineState::InQuote => {
                    current_token.push('#');
                }
            },
            Some(c) => {
                current_token.push(c);
            }
            None => {
                break;
            }
        }
    }
    if !matches!(state, LineState::Normal) {
        return Err(ParseError::Other(
            PARSE_ERROR_MESSAGE_UNCLOSED_QUOTATION.to_string(),
        ));
    }
    Ok(out)
}

impl TestConfig {
    // TODO: add Option<Vec<ConfigWarning>>, and implement warning generation.
    // TODO: make ValidationError more useful.
    pub fn validate(self: &Self) -> Result<(), ValidationError> {
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
                    TestStage::AmbientSample { counts, .. }
                    | TestStage::Exercise { counts, .. } => counts.sample_count,
                };
                if sample_count < 1 {
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
            match csv.read_line(&mut line) {
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
            let tokens = tokenise_line(data)?;
            let cols: Vec<&str> = tokens.iter().map(|col| col.as_str()).collect();

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
                        counts: StageCounts {
                            purge_count: purge_count as usize,
                            sample_count: sample_count as usize,
                        },
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
                        counts: StageCounts {
                            purge_count: purge_count as usize,
                            sample_count: sample_count as usize,
                        },
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

    pub fn exercise_count(self: &Self) -> usize {
        self.stages
            .iter()
            .filter(|stage| stage.is_exercise())
            .count()
    }

    pub fn exercise_names(self: &Self) -> Vec<String> {
        self.stages
            .iter()
            .filter(|stage| stage.is_exercise())
            .map(|stage| {
                let TestStage::Exercise { name, .. } = stage else {
                    panic!("exercises should've been filtered out already");
                };
                name
            })
            .cloned()
            .collect()
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
                name: "OSHA Fast FFP (Modified Filtering Facepiece protocol)".to_string(),
                short_name: "osha_fast_ffp".to_string(),
                stages: vec![
                    TestStage::AmbientSample {
                        counts: StageCounts {
                            purge_count: 4,
                            sample_count: 5,
                        },
                    },
                    TestStage::Exercise {
                        counts: StageCounts {
                            purge_count: 11,
                            sample_count: 30,
                        },
                        name: "Bending Over".to_string(),
                    },
                    TestStage::Exercise {
                        counts: StageCounts {
                            purge_count: 0,
                            sample_count: 30,
                        },
                        name: "Talking".to_string(),
                    },
                    TestStage::Exercise {
                        counts: StageCounts {
                            purge_count: 0,
                            sample_count: 30,
                        },
                        name: "Head Side-to-Side".to_string(),
                    },
                    TestStage::Exercise {
                        counts: StageCounts {
                            purge_count: 0,
                            sample_count: 30,
                        },
                        name: "Head Up-and-Down".to_string(),
                    },
                    TestStage::AmbientSample {
                        counts: StageCounts {
                            purge_count: 4,
                            sample_count: 5,
                        },
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
                        counts: StageCounts {
                            purge_count: 0,
                            sample_count: 1,
                        },
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
                            counts: StageCounts {
                                purge_count: 0,
                                sample_count: 1,
                            },
                        },
                        TestStage::AmbientSample {
                            counts: StageCounts {
                                purge_count: 0,
                                sample_count: 1,
                            },
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
                            counts: StageCounts {
                                purge_count: 0,
                                sample_count: 1,
                            },
                        },
                        TestStage::AmbientSample {
                            counts: StageCounts {
                                purge_count: 0,
                                sample_count: 1,
                            },
                        },
                        TestStage::AmbientSample {
                            counts: StageCounts {
                                purge_count: 0,
                                sample_count: 1,
                            },
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
                            counts: StageCounts {
                                purge_count: 0,
                                sample_count: 1,
                            },
                        },
                        TestStage::Exercise {
                            name: "foo".to_string(),
                            counts: StageCounts {
                                purge_count: 0,
                                sample_count: 1,
                            },
                        },
                        TestStage::AmbientSample {
                            counts: StageCounts {
                                purge_count: 0,
                                sample_count: 1,
                            },
                        },
                        TestStage::AmbientSample {
                            counts: StageCounts {
                                purge_count: 0,
                                sample_count: 1,
                            },
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
                            counts: StageCounts {
                                purge_count: 0,
                                sample_count: 1,
                            },
                        },
                        TestStage::Exercise {
                            name: "foo".to_string(),
                            counts: StageCounts {
                                purge_count: 0,
                                sample_count: 1,
                            },
                        },
                        TestStage::AmbientSample {
                            counts: StageCounts {
                                purge_count: 0,
                                sample_count: 1,
                            },
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
                            counts: StageCounts {
                                purge_count: 0,
                                sample_count: 0,
                            },
                        },
                        TestStage::Exercise {
                            name: "foo".to_string(),
                            counts: StageCounts {
                                purge_count: 0,
                                sample_count: 1,
                            },
                        },
                        TestStage::AmbientSample {
                            counts: StageCounts {
                                purge_count: 0,
                                sample_count: 1,
                            },
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
                            counts: StageCounts {
                                purge_count: 0,
                                sample_count: 1,
                            },
                        },
                        TestStage::Exercise {
                            name: "foo".to_string(),
                            counts: StageCounts {
                                purge_count: 0,
                                sample_count: 1,
                            },
                        },
                        TestStage::Exercise {
                            name: "foo".to_string(),
                            counts: StageCounts {
                                purge_count: 0,
                                sample_count: 1,
                            },
                        },
                        TestStage::AmbientSample {
                            counts: StageCounts {
                                purge_count: 0,
                                sample_count: 1,
                            },
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

    #[test]
    fn test_tokenise_line() {
        struct TestCase<'a> {
            name: &'a str,
            input: &'a str,
            expected_result: Result<Vec<String>, ParseError<'a>>,
        }
        let tests = [
            &TestCase {
                name: "single_item",
                input: "abcdef",
                expected_result: Ok(vec!["abcdef".to_string()]),
            },
            &TestCase {
                name: "two_items",
                input: "abcdef,hijkl",
                expected_result: Ok(vec!["abcdef".to_string(), "hijkl".to_string()]),
            },
            &TestCase {
                // This doesn't match RFC4180, but we never promised to adhere
                // to RFC4180 anyway.
                name: "two_items_spaces_are_retained",
                input: " abcdef , hijkl ",
                expected_result: Ok(vec![" abcdef ".to_string(), " hijkl ".to_string()]),
            },
            &TestCase {
                name: "one_quoted_item",
                input: r#""abcdef""#,
                expected_result: Ok(vec!["abcdef".to_string()]),
            },
            &TestCase {
                name: "one_quoted_item_with_separator",
                input: r#""abc,def""#,
                expected_result: Ok(vec!["abc,def".to_string()]),
            },
            &TestCase {
                name: "two_quoted_items",
                input: r#""abcdef","foo""#,
                expected_result: Ok(vec!["abcdef".to_string(), "foo".to_string()]),
            },
            &TestCase {
                name: "quote_in_one_quoted_item",
                input: r#""abc""def""#,
                expected_result: Ok(vec![r#"abc"def"#.to_string()]),
            },
            &TestCase {
                name: "hash_in_one_quoted_item",
                input: r##""abc#def""##,
                expected_result: Ok(vec![r##"abc#def"##.to_string()]),
            },
            &TestCase {
                name: "unquoted_hash",
                input: r##"abc#def"##,
                expected_result: Err(ParseError::Other(
                    PARSE_ERROR_MESSAGE_UNQUOTED_HASH.to_string(),
                )),
            },
            &TestCase {
                name: "unquoted_comment",
                input: r##"abc,#def"##,
                expected_result: Err(ParseError::Other(
                    PARSE_ERROR_MESSAGE_UNQUOTED_HASH.to_string(),
                )),
            },
            &TestCase {
                name: "many_quotes_in_two_quoted_items",
                input: r#""abc""de""""f","foo""bar""#,
                expected_result: Ok(vec![r#"abc"de""f"#.to_string(), r#"foo"bar"#.to_string()]),
            },
            &TestCase {
                name: "bad_leading_quotation",
                input: r#""abc", "def""#,
                expected_result: Err(ParseError::Other(
                    PARSE_ERROR_MESSAGE_BAD_LEADING_QUOTATION.to_string(),
                )),
            },
            &TestCase {
                name: "bad_leading_quotation_single_item",
                input: r#" "abc""#,
                expected_result: Err(ParseError::Other(
                    PARSE_ERROR_MESSAGE_BAD_LEADING_QUOTATION.to_string(),
                )),
            },
            &TestCase {
                name: "bad_trailing_quotation",
                input: r#""abc" ,"def""#,
                expected_result: Err(ParseError::Other(
                    PARSE_ERROR_MESSAGE_BAD_TRAILING_QUOTATION.to_string(),
                )),
            },
            &TestCase {
                name: "bad_trailing_quotation_single_item",
                input: r#""abc" "#,
                expected_result: Err(ParseError::Other(
                    PARSE_ERROR_MESSAGE_BAD_TRAILING_QUOTATION.to_string(),
                )),
            },
            &TestCase {
                name: "unclosed_quotation",
                input: r#""abc "#,
                expected_result: Err(ParseError::Other(
                    PARSE_ERROR_MESSAGE_UNCLOSED_QUOTATION.to_string(),
                )),
            },
            &TestCase {
                name: "second_item_unclosed_quotation",
                input: r#""abc","def"#,
                expected_result: Err(ParseError::Other(
                    PARSE_ERROR_MESSAGE_UNCLOSED_QUOTATION.to_string(),
                )),
            },
            &TestCase {
                name: "comment_line_not_tokenised",
                input: "#abc,def",
                expected_result: Ok(vec!["#abc,def".to_string()]),
            },
        ];
        for case in tests {
            let input = case.input.to_string();
            let got = tokenise_line(&input);
            assert_eq!(
                got, case.expected_result,
                "{}: got={got:?}, want={:?}",
                case.name, case.expected_result
            );
        }
    }
}
