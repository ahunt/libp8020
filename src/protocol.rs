use std::str::FromStr;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Indicator {
    in_progress: bool,
    fit_factor: bool,
    service: bool,
    low_particle: bool,
    low_battery: bool,
    fail: bool,
    pass: bool,
}

#[derive(Debug, PartialEq)]
enum Command {
    EnterExternalControl,
    ExitExternalControl,
    Beep {
        // Duration of the beep in tenths of seconds. Value must be within 1..=99 when sending.
        duration_deciseconds: u8,
    },
    /// VN - sample through ambient tube, or valve ON.
    ValveAmbient,
    /// VF - sample through "sample" tube, or valve OFF.
    ValveSpecimen,
    // Display exercise number: value must be within 1..=19 when sending.
    DisplayExercise(u8),
    Indicator(Indicator),
    ClearDisplay,
}

/// Message represents any message sent by the device. This can be a response,
/// or a sample, or any other message the device might send.
/// Note: the PortaCount mirrors many, but not all, commands that it receives.
/// Callers therefore cannot rely on always receiving a mirrored command. See
/// the addendum for details (e.g. the Error message can be received in response
/// to any command that the PortaCount didn't understand; the Settings command
/// triggers a list of settings across multiple messages; etc.).
#[derive(Debug, PartialEq)]
enum Message {
    Response(Command),
    /// Error response. Note: UnknownError might be returned instead of the
    /// original command could not be parsed.
    ErrorResponse(Command),
    UnknownError(String),
    Sample(f64),
}

#[derive(Debug)]
struct ParseError {
    received_message: String,
    reason: String,
}

impl PartialEq for ParseError {
    fn eq(&self, other: &Self) -> bool {
        self.received_message == other.received_message
    }
}

impl Eq for ParseError {}

fn parse_command(command: &str) -> Result<Command, ParseError> {
    match command {
        "VN" => Ok(Command::ValveAmbient),
        // The spec claims this is "VO", my 8020A returns "VF". Supporting both should
        // reduce the risk of surprises.
        "VF" | "VO" => Ok(Command::ValveSpecimen),
        // Note: the command to enter external control ("J") does not match the
        // response ("OK").
        "OK" => Ok(Command::EnterExternalControl),
        "G" => Ok(Command::ExitExternalControl),
        "K" => Ok(Command::ClearDisplay),
        ref command if command.starts_with("B") => {
            // According to spec, the range is 1..=99 (padded to two digits),
            // but I don't think there's much harm in being more permissive.
            match u8::from_str(&command[1..]) {
                Ok(duration) => Ok(Command::Beep {
                    duration_deciseconds: duration,
                }),
                Err(_) => Err(ParseError {
                    received_message: command.to_string(),
                    reason: "unable to parse beep duration".to_string(),
                }),
            }
        }
        ref command if command.starts_with("N") => {
            // According to spec, the range is 0..=19 (padded to two digits),
            // but I don't think there's much harm in being more permissive.
            match u8::from_str(&command[1..]) {
                Ok(exercise) => Ok(Command::DisplayExercise(exercise)),
                Err(_) => Err(ParseError {
                    received_message: command.to_string(),
                    reason: "unable to parse exercise number".to_string(),
                }),
            }
        }
        ref command if command.starts_with("I") => {
            if command.len() != 9 {
                return Err(ParseError {
                    received_message: command.to_string(),
                    reason: "unable to parse indicator with unexpected length".to_string(),
                });
            }
            let mut chars = command.chars();
            // I
            chars.next();
            // Unused (expected to be 0).;
            chars.next();
            // Parsing is deliberately permissive - I expect most clients to completely
            // ignore the result here anyway.
            Ok(Command::Indicator(Indicator {
                in_progress: chars.next() == Some('1'),
                fit_factor: chars.next() == Some('1'),
                service: chars.next() == Some('1'),
                low_particle: chars.next() == Some('1'),
                low_battery: chars.next() == Some('1'),
                fail: chars.next() == Some('1'),
                pass: chars.next() == Some('1'),
            }))
        }
        _ => Err(ParseError {
            received_message: command.to_string(),
            reason: "unknown or unsupported command".to_string(),
        }),
    }
}

/// Parse a message received from the portacount.
/// Note: this function can return a ParseError for messages that were not
/// understood. This does not indicate any problem with the device, it merely
/// indicates that we don't know what the message was intended to mean, and/or
/// that support for this message is not yet implemented.
fn parse_message(message: &str) -> Result<Message, ParseError> {
    if message.is_empty() {
        return Err(ParseError {
            received_message: message.to_string(),
            reason: "received empty message".to_string(),
        });
    }

    // There are many more "elegant" and/or efficient ways to do this (e.g. by
    // using some proper parser, or with a trie). However the approach below is
    // more than performant enough (if someone is going to be handling thousands
    // of PortAcounts, then they can probably afford a few extra cores...).
    // Moreover, instead of hardcoded strings here (which are duplicated in
    // wherever outgoing messages are constructed) it would probably be possible
    // to build prefix->Command and Command->prefix tables from a single
    // definition using e.g. macros, but this is more than good enough IMHO and
    // might not unnecessarily confuse readers.
    match message {
        // Samples (i.e. numeric messages) are most common, hence we always
        // check these first, instead of trying to parse a command first and falling
        // back here if command parsing fails.
        // TODO: consider checking length too - the specs claim this will always be 9
        // chars long.
        ref message
            if match message.chars().next().unwrap_or('x') {
                '0'..='9' => true,
                _ => false,
            } =>
        {
            match f64::from_str(message) {
                Ok(sample) => Ok(Message::Sample(sample)),
                Err(_) => Err(ParseError {
                    received_message: message.to_string(),
                    reason: "unable to parse sample".to_string(),
                }),
            }
        }

        ref message if message.starts_with("E") => {
            // TODO: try to parse command recursively.
            Ok(Message::UnknownError(format!(
                "Error parsing not yet implemented: {}",
                message
            )))
        }
        message => match parse_command(message) {
            Ok(command) => Ok(Message::Response(command)),
            Err(err) => Err(ParseError {
                received_message: message.to_string(),
                ..err
            }),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_message() {
        let empty_indicator = Indicator {
            in_progress: false,
            fit_factor: false,
            service: false,
            low_particle: false,
            low_battery: false,
            fail: false,
            pass: false,
        };

        struct TestCase<'a> {
            name: &'a str,
            input: &'a str,
            expected_result: Result<Message, ParseError>,
        }
        let tests = [
            TestCase {
                name: "Sample0",
                input: "000000.00",
                expected_result: Ok(Message::Sample(0.0)),
            },
            TestCase {
                name: "Sample1",
                input: "000001.00",
                expected_result: Ok(Message::Sample(1.0)),
            },
            TestCase {
                name: "Sample.03",
                input: "000000.03",
                expected_result: Ok(Message::Sample(0.03)),
            },
            TestCase {
                name: "SampleMax",
                input: "99999999.",
                expected_result: Ok(Message::Sample(99999999.0)),
            },
            TestCase {
                name: "EnterExternalControl",
                input: "OK",
                expected_result: Ok(Message::Response(Command::EnterExternalControl)),
            },
            TestCase {
                name: "ExitExternalControl",
                input: "G",
                expected_result: Ok(Message::Response(Command::ExitExternalControl)),
            },
            TestCase {
                name: "ValveAmbient",
                input: "VN",
                expected_result: Ok(Message::Response(Command::ValveAmbient)),
            },
            TestCase {
                name: "ValveSpecimenSpec",
                input: "VO",
                expected_result: Ok(Message::Response(Command::ValveSpecimen)),
            },
            TestCase {
                name: "ValveSpecimenDeFacto",
                input: "VF",
                expected_result: Ok(Message::Response(Command::ValveSpecimen)),
            },
            TestCase {
                name: "Beep11",
                input: "B11",
                expected_result: Ok(Message::Response(Command::Beep {
                    duration_deciseconds: 11,
                })),
            },
            TestCase {
                name: "BeepGarbage",
                input: "BAA",
                expected_result: Err(ParseError {
                    received_message: "BAA".to_string(),
                    reason: "".to_string(),
                }),
            },
            TestCase {
                name: "BeepTooLong",
                input: "B256",
                expected_result: Err(ParseError {
                    received_message: "B256".to_string(),
                    reason: "".to_string(),
                }),
            },
            TestCase {
                name: "DisplayExercise0",
                input: "N00",
                expected_result: Ok(Message::Response(Command::DisplayExercise(0))),
            },
            TestCase {
                name: "DisplayExercise1",
                input: "N01",
                expected_result: Ok(Message::Response(Command::DisplayExercise(1))),
            },
            TestCase {
                name: "DisplayExercise99",
                input: "N99",
                expected_result: Ok(Message::Response(Command::DisplayExercise(99))),
            },
            TestCase {
                name: "DisplayExercise100",
                // Not part of the spec, but we should be able to handle it...
                input: "N100",
                expected_result: Ok(Message::Response(Command::DisplayExercise(100))),
            },
            TestCase {
                name: "DisplayExerciseGarbage",
                input: "NAA",
                expected_result: Err(ParseError {
                    received_message: "NAA".to_string(),
                    reason: "".to_string(),
                }),
            },
            TestCase {
                name: "ClearDisplay",
                input: "K",
                expected_result: Ok(Message::Response(Command::ClearDisplay)),
            },
            TestCase {
                name: "IndicatorEmpty",
                input: "I00000000",
                expected_result: Ok(Message::Response(Command::Indicator(empty_indicator))),
            },
            TestCase {
                name: "IndicatorInProgress",
                input: "I01000000",
                expected_result: Ok(Message::Response(Command::Indicator(Indicator {
                    in_progress: true,
                    ..empty_indicator
                }))),
            },
            TestCase {
                name: "IndicatorFitFactor",
                input: "I00100000",
                expected_result: Ok(Message::Response(Command::Indicator(Indicator {
                    fit_factor: true,
                    ..empty_indicator
                }))),
            },
            TestCase {
                name: "IndicatorService",
                input: "I00010000",
                expected_result: Ok(Message::Response(Command::Indicator(Indicator {
                    service: true,
                    ..empty_indicator
                }))),
            },
            TestCase {
                name: "IndicatorLowParticle",
                input: "I00001000",
                expected_result: Ok(Message::Response(Command::Indicator(Indicator {
                    low_particle: true,
                    ..empty_indicator
                }))),
            },
            TestCase {
                name: "IndicatorLowBattery",
                input: "I00000100",
                expected_result: Ok(Message::Response(Command::Indicator(Indicator {
                    low_battery: true,
                    ..empty_indicator
                }))),
            },
            TestCase {
                name: "IndicatorFail",
                input: "I00000010",
                expected_result: Ok(Message::Response(Command::Indicator(Indicator {
                    fail: true,
                    ..empty_indicator
                }))),
            },
            TestCase {
                name: "IndicatorPass",
                input: "I00000001",
                expected_result: Ok(Message::Response(Command::Indicator(Indicator {
                    pass: true,
                    ..empty_indicator
                }))),
            },
            TestCase {
                name: "IndicatorInProgressFailPass",
                input: "I01000011",
                expected_result: Ok(Message::Response(Command::Indicator(Indicator {
                    in_progress: true,
                    fail: true,
                    pass: true,
                    ..empty_indicator
                }))),
            },
        ];
        for case in tests {
            let got = parse_message(case.input);
            assert_eq!(
                got, case.expected_result,
                "{}: got={got:?}, want={:?}",
                case.name, case.expected_result
            );
        }
    }
}
