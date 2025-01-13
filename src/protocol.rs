use std::str::FromStr;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Indicator {
    pub in_progress: bool,
    pub fit_factor: bool,
    pub service: bool,
    pub low_particle: bool,
    pub low_battery: bool,
    pub fail: bool,
    pub pass: bool,
}

const EMPTY_INDICATOR: Indicator = Indicator {
    in_progress: false,
    fit_factor: false,
    service: false,
    low_particle: false,
    low_battery: false,
    fail: false,
    pass: false,
};

impl Indicator {
    pub fn empty() -> Indicator {
        EMPTY_INDICATOR
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    EnterExternalControl,
    ExitExternalControl,
    Beep {
        // Duration of the beep in tenths of seconds. Value must be within
        // 1..=60 when sending. (Note: the specs claim 99, but values above
        // 60 do not work with my 8020A - it returns an error.)
        duration_deciseconds: u8,
    },
    /// VN - sample through ambient tube, or valve ON.
    ValveAmbient,
    /// VF - sample through "sample" tube, or valve OFF.
    ValveSpecimen,
    // Display exercise number: value must be within 1..=19 when sending.
    DisplayExercise(u8),
    DisplayConcentration(f64),
    Indicator(Indicator),
    ClearDisplay,
    RequestSettings,
}

#[derive(Debug, PartialEq)]
pub enum InvalidCommandError {
    OutOfRange {
        command: Command,
        allowed_range: std::ops::Range<usize>,
    },
}

impl Command {
    pub fn to_wire(&self) -> Result<String, InvalidCommandError> {
        match self {
            Command::EnterExternalControl => Ok("J".to_string()),
            Command::ExitExternalControl => Ok("G".to_string()),
            Command::Beep {
                duration_deciseconds,
            } => match duration_deciseconds {
                1..=60 => Ok(format!("B{:02}", duration_deciseconds)),
                _ => Err(InvalidCommandError::OutOfRange {
                    command: self.clone(),
                    allowed_range: std::ops::Range { start: 1, end: 61 },
                }),
            },
            Command::ValveAmbient => Ok("VN".to_string()),
            Command::ValveSpecimen => Ok("VF".to_string()),
            Command::DisplayExercise(exercise) => match exercise {
                0..=19 => Ok(format!("N{:02}", exercise)),
                _ => Err(InvalidCommandError::OutOfRange {
                    command: self.clone(),
                    allowed_range: std::ops::Range { start: 0, end: 20 },
                }),
            },
            Command::DisplayConcentration(value) => {
                // I haven't figured out a way to control segments directly yet
                // (including 'A' or 'a' as part of this command does not work for example...).
                // Being able to do so would be nice for indicating the current exercise name.
                if *value < 100.0 {
                    Ok(format!("D{value:09.2}"))
                } else {
                    let value = value.round() as usize;
                    if value > 999_999_999 {
                        return Err(InvalidCommandError::OutOfRange {
                            command: self.clone(),
                            allowed_range: std::ops::Range {
                                start: 0,
                                end: 999_999_999,
                            },
                        });
                    }
                    Ok(format!("D{value:09.0}"))
                }
            }
            Command::Indicator(indicator) => {
                let mut out = String::with_capacity(9);
                out.push_str("I0");
                out.push(if indicator.in_progress { '1' } else { '0' });
                out.push(if indicator.fit_factor { '1' } else { '0' });
                out.push(if indicator.service { '1' } else { '0' });
                out.push(if indicator.low_particle { '1' } else { '0' });
                out.push(if indicator.low_battery { '1' } else { '0' });
                out.push(if indicator.fail { '1' } else { '0' });
                out.push(if indicator.pass { '1' } else { '0' });
                Ok(out)
            }
            Command::ClearDisplay => Ok("K".to_string()),
            Command::RequestSettings => Ok("S".to_string()),
        }
    }
}

/// Message represents any message sent by the device. This can be a response,
/// or a sample, or any other message the device might send.
/// Note: the PortaCount mirrors many, but not all, commands that it receives.
/// Callers therefore cannot rely on always receiving a mirrored command. See
/// the addendum for details (e.g. the Error message can be received in response
/// to any command that the PortaCount didn't understand; the Settings command
/// triggers a list of settings across multiple messages; etc.).
#[derive(Debug, PartialEq)]
pub enum Message {
    Response(Command),
    /// Error response. Note: UnknownError might be returned instead of the
    /// original command could not be parsed.
    ErrorResponse(Command),
    UnknownError(String),
    Sample(f64),
    Setting(SettingMessage),
}

#[derive(Debug)]
pub struct ParseError {
    pub received_message: String,
    pub reason: String,
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
        ref command if command.starts_with("D") => {
            // According to spec, the number will use 9 chars - but but I don't
            // think there's much harm in being more permissive.
            match f64::from_str(&command[1..]) {
                Ok(value) => Ok(Command::DisplayConcentration(value)),
                Err(_) => Err(ParseError {
                    received_message: command.to_string(),
                    reason: "unable to parse display-concentration command".to_string(),
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

/// Represents one of the responses to "Request Settings" Command ("S").
/// These settings pertain to tests run directly on the device, i.e. they're
/// orthogonal to the test configurations being used by libp8020.
/// Case names match those used in the Technical Addendum (i.e. they do not
/// follow libp8020 conventions - compare Mask/Specimen, Purge/SamplePurge,
/// etc.).
/// Note: the addendum specifies that each value will be within a specific
/// range. However libp8020 does not actually validate that the device returned
/// a setting within the specified range.
#[derive(Debug, PartialEq)]
pub enum SettingMessage {
    // Spec: 4..=25
    AmbientPurgeTime {
        seconds: usize,
    },
    // Spec: 5..=99
    AmbientSampleTime {
        seconds: usize,
    },
    // Spec: 11..=99
    MaskSamplePurgeTime {
        seconds: usize,
    },
    // Spec:
    //   ex: 1..=13 (13 == time when running with 0 exercises aka 8010 mode).
    //   seconds: 10..=99
    MaskSampleTime {
        ex: usize,
        seconds: usize,
    },
    // Spec:
    //   ex: 1..=12
    //   fit_factor <= 64_000.
    FitFactorPassLevel {
        ex: usize,
        fit_factor: usize,
    },
    /// Might not be a number.
    SerialNumber(String),
    RunTimeSinceService {
        decaminutes: usize,
    },
    /// year is modulo 100. We use this format over timestamp or date
    /// representations as then we have to start worrying about timezones and
    /// such like.
    DateLastServiced {
        month: u8,
        year: u8,
    },
}

fn parse_setting(setting: &str) -> Result<SettingMessage, ParseError> {
    // Each of these messages is specified to be 9 chars long, with empty spaces
    // in the middle to suit. And despite that, a lot of messages contain
    // hardcoded 0s as a prefix to the numeric value. That actually doesn't
    // matter whatsoever because there's no need to care about indexes when
    // parsing, but it's certainly rather weird.
    match setting {
        setting if setting.starts_with("STPA") => {
            match usize::from_str(setting.strip_prefix("STPA").unwrap().trim()) {
                Ok(seconds) => Ok(SettingMessage::AmbientPurgeTime { seconds }),
                Err(_) => Err(ParseError {
                    received_message: setting.to_string(),
                    reason: "unable to parse ambient purge time".to_string(),
                }),
            }
        }
        command if command.starts_with("STA") => {
            match usize::from_str(setting.strip_prefix("STA").unwrap().trim()) {
                Ok(seconds) => Ok(SettingMessage::AmbientSampleTime { seconds }),
                Err(_) => Err(ParseError {
                    received_message: setting.to_string(),
                    reason: "unable to parse ambient sample time".to_string(),
                }),
            }
        }
        command if command.starts_with("STPM") => {
            match usize::from_str(setting.strip_prefix("STPM").unwrap().trim()) {
                Ok(seconds) => Ok(SettingMessage::MaskSamplePurgeTime { seconds }),
                Err(_) => Err(ParseError {
                    received_message: setting.to_string(),
                    reason: "unable to parse mask sample purge time".to_string(),
                }),
            }
        }
        command if command.starts_with("STM") => {
            // They really do specify this as STMxx000vv !?
            // There's probably no point in even trying to handle this, who cares?
            let value = &setting.strip_prefix("STM").unwrap().trim();
            match if value.chars().count() > 2 {
                let split_at = value.char_indices().nth(2).unwrap().0;
                if let Ok(ex) = usize::from_str(&value[..split_at]) {
                    if let Ok(seconds) = usize::from_str(&value[split_at..]) {
                        Some(SettingMessage::MaskSampleTime { ex, seconds })
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            } {
                Some(mask_purge_time) => Ok(mask_purge_time),
                None => Err(ParseError {
                    received_message: setting.to_string(),
                    reason: "unable to parse mask sample time".to_string(),
                }),
            }
        }
        command if command.starts_with("SP") => {
            // Same nonsense as above - "SP xxvvvvv" !?
            let value = &setting.strip_prefix("SP").unwrap().trim();
            match if value.chars().count() > 2 {
                let split_at = value.char_indices().nth(2).unwrap().0;
                if let Ok(ex) = usize::from_str(&value[..split_at]) {
                    if let Ok(fit_factor) = usize::from_str(&value[split_at..]) {
                        Some(SettingMessage::FitFactorPassLevel { ex, fit_factor })
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            } {
                Some(ffpl) => Ok(ffpl),
                None => Err(ParseError {
                    received_message: setting.to_string(),
                    reason: "unable to parse fit factor pass level".to_string(),
                }),
            }
        }
        command if command.starts_with("SS") => Ok(SettingMessage::SerialNumber(
            // The Technical addendum claims that this is "SS   vvvvv" (3 spaces
            // is seemingly guaranteed). In reality, the serial number can be longer
            // than 5 chars - my 8020A returns 8024XXXX.
            setting.strip_prefix("SS").unwrap().trim().to_string(),
        )),
        command if command.starts_with("SR") => {
            match usize::from_str(setting.strip_prefix("SR").unwrap().trim()) {
                Ok(decaminutes) => Ok(SettingMessage::RunTimeSinceService { decaminutes }),
                Err(_) => Err(ParseError {
                    received_message: setting.to_string(),
                    reason: "unable to parse run time since last service".to_string(),
                }),
            }
        }
        command if command.starts_with("SD") => {
            match match usize::from_str(setting.strip_prefix("SD").unwrap().trim()) {
                Ok(n) => {
                    let year = (n % 100) as u8;
                    let month = n / 100;
                    if n > 9999 || month > 12 {
                        None
                    } else {
                        Some(SettingMessage::DateLastServiced {
                            month: month as u8,
                            year,
                        })
                    }
                }
                Err(_) => None,
            } {
                Some(dls) => Ok(dls),
                None => Err(ParseError {
                    received_message: setting.to_string(),
                    reason: "unable to parse date last serviced".to_string(),
                }),
            }
        }
        _ => Err(ParseError {
            received_message: setting.to_string(),
            reason: "unknown or unsupported command".to_string(),
        }),
    }
}

/// Parse a message received from the portacount.
/// Note: this function can return a ParseError for messages that were not
/// understood. This does not indicate any problem with the device, it merely
/// indicates that we don't know what the message was intended to mean, and/or
/// that support for this message is not yet implemented.
pub fn parse_message(message: &str) -> Result<Message, ParseError> {
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
        // Specs claim that this will always be 9 chars long (after unwrapping),
        // but there's no reason to be strict
        ref message if message.chars().next().unwrap_or('x').is_ascii_digit() => {
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
        ref message if message.starts_with("S") => match parse_setting(message) {
            Ok(setting_message) => Ok(Message::Setting(setting_message)),
            Err(err) => Err(ParseError {
                received_message: message.to_string(),
                ..err
            }),
        },
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
    fn test_command_to_wire() {
        struct TestCase<'a> {
            name: &'a str,
            input: Command,
            expected_result: Result<String, InvalidCommandError>,
        }
        let tests = [
            TestCase {
                name: "EnterExternalControl",
                input: Command::EnterExternalControl,
                expected_result: Ok("J".to_string()),
            },
            TestCase {
                name: "ExitExternalControl",
                input: Command::ExitExternalControl,
                expected_result: Ok("G".to_string()),
            },
            TestCase {
                name: "Beep0",
                input: Command::Beep {
                    duration_deciseconds: 0,
                },
                expected_result: Err(InvalidCommandError::OutOfRange {
                    command: Command::Beep {
                        duration_deciseconds: 0,
                    },
                    allowed_range: std::ops::Range { start: 1, end: 61 },
                }),
            },
            TestCase {
                name: "Beep1",
                input: Command::Beep {
                    duration_deciseconds: 1,
                },
                expected_result: Ok("B01".to_string()),
            },
            TestCase {
                name: "Beep9",
                input: Command::Beep {
                    duration_deciseconds: 9,
                },
                expected_result: Ok("B09".to_string()),
            },
            TestCase {
                name: "Beep10",
                input: Command::Beep {
                    duration_deciseconds: 10,
                },
                expected_result: Ok("B10".to_string()),
            },
            TestCase {
                name: "Beep60",
                input: Command::Beep {
                    duration_deciseconds: 60,
                },
                expected_result: Ok("B60".to_string()),
            },
            TestCase {
                name: "Beep61",
                input: Command::Beep {
                    duration_deciseconds: 61,
                },
                expected_result: Err(InvalidCommandError::OutOfRange {
                    command: Command::Beep {
                        duration_deciseconds: 61,
                    },
                    allowed_range: std::ops::Range { start: 1, end: 61 },
                }),
            },
            TestCase {
                name: "ValveAmbient",
                input: Command::ValveAmbient,
                expected_result: Ok("VN".to_string()),
            },
            TestCase {
                name: "ValveSpecimen",
                input: Command::ValveSpecimen,
                expected_result: Ok("VF".to_string()),
            },
            TestCase {
                name: "DisplayExercise0",
                input: Command::DisplayExercise(0),
                expected_result: Ok("N00".to_string()),
            },
            TestCase {
                name: "DisplayExercise1",
                input: Command::DisplayExercise(1),
                expected_result: Ok("N01".to_string()),
            },
            TestCase {
                name: "DisplayExercise9",
                input: Command::DisplayExercise(9),
                expected_result: Ok("N09".to_string()),
            },
            TestCase {
                name: "DisplayExercise10",
                input: Command::DisplayExercise(10),
                expected_result: Ok("N10".to_string()),
            },
            TestCase {
                name: "DisplayExercise19",
                input: Command::DisplayExercise(19),
                expected_result: Ok("N19".to_string()),
            },
            TestCase {
                name: "DisplayExercise20",
                input: Command::DisplayExercise(20),
                expected_result: Err(InvalidCommandError::OutOfRange {
                    command: Command::DisplayExercise(20),
                    allowed_range: std::ops::Range { start: 0, end: 20 },
                }),
            },
            TestCase {
                name: "DisplayConcentration 0.0",
                input: Command::DisplayConcentration(0.0),
                expected_result: Ok("D000000.00".to_string()),
            },
            TestCase {
                name: "DisplayConcentration 99.9",
                input: Command::DisplayConcentration(99.9),
                expected_result: Ok("D000099.90".to_string()),
            },
            TestCase {
                name: "DisplayConcentration 100.0",
                input: Command::DisplayConcentration(100.0),
                expected_result: Ok("D000000100".to_string()),
            },
            TestCase {
                name: "DisplayConcentration 100.4",
                input: Command::DisplayConcentration(100.4),
                expected_result: Ok("D000000100".to_string()),
            },
            TestCase {
                name: "DisplayConcentration 100.5",
                input: Command::DisplayConcentration(100.5),
                expected_result: Ok("D000000101".to_string()),
            },
            TestCase {
                name: "DisplayConcentration 999_999_999.0",
                input: Command::DisplayConcentration(999_999_999.0),
                expected_result: Ok("D999999999".to_string()),
            },
            TestCase {
                name: "DisplayConcentration 1_000_000_000.0",
                input: Command::DisplayConcentration(1_000_000_000.0),
                expected_result: Err(InvalidCommandError::OutOfRange {
                    command: Command::DisplayConcentration(1_000_000_000.0),
                    allowed_range: std::ops::Range {
                        start: 0,
                        end: 999_999_999,
                    },
                }),
            },
            TestCase {
                name: "IndicatorEmpty",
                input: Command::Indicator(EMPTY_INDICATOR),
                expected_result: Ok("I00000000".to_string()),
            },
            TestCase {
                name: "IndicatorInProgress",
                input: Command::Indicator(Indicator {
                    in_progress: true,
                    ..EMPTY_INDICATOR
                }),
                expected_result: Ok("I01000000".to_string()),
            },
            TestCase {
                name: "IndicatorFitFactor",
                input: Command::Indicator(Indicator {
                    fit_factor: true,
                    ..EMPTY_INDICATOR
                }),
                expected_result: Ok("I00100000".to_string()),
            },
            TestCase {
                name: "IndicatorService",
                input: Command::Indicator(Indicator {
                    service: true,
                    ..EMPTY_INDICATOR
                }),
                expected_result: Ok("I00010000".to_string()),
            },
            TestCase {
                name: "IndicatorLowParticle",
                input: Command::Indicator(Indicator {
                    low_particle: true,
                    ..EMPTY_INDICATOR
                }),
                expected_result: Ok("I00001000".to_string()),
            },
            TestCase {
                name: "IndicatorLowBattery",
                input: Command::Indicator(Indicator {
                    low_battery: true,
                    ..EMPTY_INDICATOR
                }),
                expected_result: Ok("I00000100".to_string()),
            },
            TestCase {
                name: "IndicatorFail",
                input: Command::Indicator(Indicator {
                    fail: true,
                    ..EMPTY_INDICATOR
                }),
                expected_result: Ok("I00000010".to_string()),
            },
            TestCase {
                name: "IndicatorPass",
                input: Command::Indicator(Indicator {
                    pass: true,
                    ..EMPTY_INDICATOR
                }),
                expected_result: Ok("I00000001".to_string()),
            },
            TestCase {
                name: "IndicatorMulti1",
                input: Command::Indicator(Indicator {
                    in_progress: true,
                    pass: true,
                    ..EMPTY_INDICATOR
                }),
                expected_result: Ok("I01000001".to_string()),
            },
            TestCase {
                name: "IndicatorMulti2",
                input: Command::Indicator(Indicator {
                    fit_factor: true,
                    service: true,
                    ..EMPTY_INDICATOR
                }),
                expected_result: Ok("I00110000".to_string()),
            },
            TestCase {
                name: "IndicatorAll",
                input: Command::Indicator(Indicator {
                    in_progress: true,
                    fit_factor: true,
                    service: true,
                    low_particle: true,
                    low_battery: true,
                    fail: true,
                    pass: true,
                }),
                expected_result: Ok("I01111111".to_string()),
            },
            TestCase {
                name: "ClearDisplay",
                input: Command::ClearDisplay,
                expected_result: Ok("K".to_string()),
            },
            TestCase {
                name: "RequestSettings",
                input: Command::RequestSettings,
                expected_result: Ok("S".to_string()),
            },
        ];
        for case in tests {
            let got = case.input.to_wire();
            assert_eq!(
                got, case.expected_result,
                "{}: got={got:?}, want={:?}",
                case.name, case.expected_result
            );
        }
    }

    #[test]
    fn test_parse_message() {
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
                name: "DisplayConcentration_0.",
                input: "D00000000.",
                expected_result: Ok(Message::Response(Command::DisplayConcentration(0.0))),
            },
            TestCase {
                name: "DisplayConcentration_0.0",
                input: "D0000000.0",
                expected_result: Ok(Message::Response(Command::DisplayConcentration(0.0))),
            },
            TestCase {
                name: "DisplayConcentration_.000000000",
                input: "D.00000000",
                expected_result: Ok(Message::Response(Command::DisplayConcentration(0.0))),
            },
            TestCase {
                name: "DisplayConcentration_1.0",
                input: "D1.0000000",
                expected_result: Ok(Message::Response(Command::DisplayConcentration(1.0))),
            },
            TestCase {
                name: "DisplayConcentration_999.99",
                input: "D000999.99",
                expected_result: Ok(Message::Response(Command::DisplayConcentration(999.99))),
            },
            TestCase {
                name: "DisplayConcentration_1_000_000_000",
                // Not part of the spec, but we should be able to handle it...
                input: "D1000000000",
                expected_result: Ok(Message::Response(Command::DisplayConcentration(
                    1_000_000_000.0,
                ))),
            },
            TestCase {
                name: "DisplayConcentrationGarbage",
                input: "DAA",
                expected_result: Err(ParseError {
                    received_message: "DAA".to_string(),
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
                expected_result: Ok(Message::Response(Command::Indicator(EMPTY_INDICATOR))),
            },
            TestCase {
                name: "IndicatorInProgress",
                input: "I01000000",
                expected_result: Ok(Message::Response(Command::Indicator(Indicator {
                    in_progress: true,
                    ..EMPTY_INDICATOR
                }))),
            },
            TestCase {
                name: "IndicatorFitFactor",
                input: "I00100000",
                expected_result: Ok(Message::Response(Command::Indicator(Indicator {
                    fit_factor: true,
                    ..EMPTY_INDICATOR
                }))),
            },
            TestCase {
                name: "IndicatorService",
                input: "I00010000",
                expected_result: Ok(Message::Response(Command::Indicator(Indicator {
                    service: true,
                    ..EMPTY_INDICATOR
                }))),
            },
            TestCase {
                name: "IndicatorLowParticle",
                input: "I00001000",
                expected_result: Ok(Message::Response(Command::Indicator(Indicator {
                    low_particle: true,
                    ..EMPTY_INDICATOR
                }))),
            },
            TestCase {
                name: "IndicatorLowBattery",
                input: "I00000100",
                expected_result: Ok(Message::Response(Command::Indicator(Indicator {
                    low_battery: true,
                    ..EMPTY_INDICATOR
                }))),
            },
            TestCase {
                name: "IndicatorFail",
                input: "I00000010",
                expected_result: Ok(Message::Response(Command::Indicator(Indicator {
                    fail: true,
                    ..EMPTY_INDICATOR
                }))),
            },
            TestCase {
                name: "IndicatorPass",
                input: "I00000001",
                expected_result: Ok(Message::Response(Command::Indicator(Indicator {
                    pass: true,
                    ..EMPTY_INDICATOR
                }))),
            },
            TestCase {
                name: "IndicatorInProgressFailPass",
                input: "I01000011",
                expected_result: Ok(Message::Response(Command::Indicator(Indicator {
                    in_progress: true,
                    fail: true,
                    pass: true,
                    ..EMPTY_INDICATOR
                }))),
            },
            TestCase {
                name: "SettingAmbientPurgeTime4",
                input: "STPA 00004",
                expected_result: Ok(Message::Setting(SettingMessage::AmbientPurgeTime {
                    seconds: 4,
                })),
            },
            TestCase {
                name: "SettingAmbientPurgeTime0",
                // Not compliant with spec
                input: "STPA 0",
                expected_result: Ok(Message::Setting(SettingMessage::AmbientPurgeTime {
                    seconds: 0,
                })),
            },
            TestCase {
                name: "SettingAmbientPurgeTime999",
                // Not compliant with spec
                input: "STPA 00999",
                expected_result: Ok(Message::Setting(SettingMessage::AmbientPurgeTime {
                    seconds: 999,
                })),
            },
            TestCase {
                name: "SettingAmbientPurgeTimeEmpty",
                input: "STPA",
                expected_result: Err(ParseError {
                    received_message: "STPA".to_string(),
                    reason: "".to_string(),
                }),
            },
            TestCase {
                name: "SettingAmbientSampleTime5",
                input: "STA  00005",
                expected_result: Ok(Message::Setting(SettingMessage::AmbientSampleTime {
                    seconds: 5,
                })),
            },
            TestCase {
                name: "SettingAmbientSampleTime0",
                // Not compliant with spec
                input: "STA 0",
                expected_result: Ok(Message::Setting(SettingMessage::AmbientSampleTime {
                    seconds: 0,
                })),
            },
            TestCase {
                name: "SettingAmbientSampleTime999",
                // Not compliant with spec
                input: "STA 0999",
                expected_result: Ok(Message::Setting(SettingMessage::AmbientSampleTime {
                    seconds: 999,
                })),
            },
            TestCase {
                name: "SettingAmbientSampleTimeEmpty",
                input: "STA",
                expected_result: Err(ParseError {
                    received_message: "STA".to_string(),
                    reason: "".to_string(),
                }),
            },
            TestCase {
                name: "SettingMaskSamplePurgeTime11",
                input: "STPM 00011",
                expected_result: Ok(Message::Setting(SettingMessage::MaskSamplePurgeTime {
                    seconds: 11,
                })),
            },
            TestCase {
                name: "SettingMaskSamplePurgeTime0",
                // Not compliant with spec
                input: "STPM 0",
                expected_result: Ok(Message::Setting(SettingMessage::MaskSamplePurgeTime {
                    seconds: 0,
                })),
            },
            TestCase {
                name: "SettingMaskSamplePurgeTime999",
                // Not compliant with spec
                input: "STPM 00999",
                expected_result: Ok(Message::Setting(SettingMessage::MaskSamplePurgeTime {
                    seconds: 999,
                })),
            },
            TestCase {
                name: "SettingMaskSamplePurgeTimeEmpty",
                input: "STPM",
                expected_result: Err(ParseError {
                    received_message: "STPM".to_string(),
                    reason: "".to_string(),
                }),
            },
            TestCase {
                name: "SettingMaskSampleTime1_10",
                input: "STM0100010",
                expected_result: Ok(Message::Setting(SettingMessage::MaskSampleTime {
                    ex: 1,
                    seconds: 10,
                })),
            },
            TestCase {
                name: "SettingMaskSampleTime0_0",
                // This is totally out of spec.
                input: "STM0000000",
                expected_result: Ok(Message::Setting(SettingMessage::MaskSampleTime {
                    ex: 0,
                    seconds: 0,
                })),
            },
            TestCase {
                name: "SettingMaskSampleTime99_999",
                // This is also way out of spec.
                input: "STM9900999",
                expected_result: Ok(Message::Setting(SettingMessage::MaskSampleTime {
                    ex: 99,
                    seconds: 999,
                })),
            },
            TestCase {
                name: "SettingMaskSampleTimeInvalid11",
                input: "STM 11",
                expected_result: Err(ParseError {
                    received_message: "STM 11".to_string(),
                    reason: "".to_string(),
                }),
            },
            TestCase {
                name: "SettingMaskSampleTimeEmpty",
                input: "STM",
                expected_result: Err(ParseError {
                    received_message: "STM".to_string(),
                    reason: "".to_string(),
                }),
            },
            TestCase {
                name: "SettingMaskSampleTimeMalformed",
                // Found via fuzzing.
                input: "STM_©",
                expected_result: Err(ParseError {
                    received_message: "STM_©".to_string(),
                    reason: "".to_string(),
                }),
            },
            TestCase {
                name: "SettingFitFactorPassLevel01_100",
                input: "SP 0100100",
                expected_result: Ok(Message::Setting(SettingMessage::FitFactorPassLevel {
                    ex: 1,
                    fit_factor: 100,
                })),
            },
            TestCase {
                name: "SettingFitFactorPassLevel12_64000",
                input: "SP 1264000",
                expected_result: Ok(Message::Setting(SettingMessage::FitFactorPassLevel {
                    ex: 12,
                    fit_factor: 64_000,
                })),
            },
            TestCase {
                name: "SettingFitFactorPassLevel000",
                // Out of spec
                input: "SP 000",
                expected_result: Ok(Message::Setting(SettingMessage::FitFactorPassLevel {
                    ex: 0,
                    fit_factor: 0,
                })),
            },
            TestCase {
                name: "SettingFitFactorPassLevelInvalid12",
                input: "SP 12",
                expected_result: Err(ParseError {
                    received_message: "SP 12".to_string(),
                    reason: "".to_string(),
                }),
            },
            TestCase {
                name: "SettingFitFactorPassLevelEmpty",
                input: "SP",
                expected_result: Err(ParseError {
                    received_message: "SP".to_string(),
                    reason: "".to_string(),
                }),
            },
            TestCase {
                name: "SettingFitFactorPassLevelMalformed",
                // Found via fuzzing.
                input: "SP_©",
                expected_result: Err(ParseError {
                    received_message: "SP_©".to_string(),
                    reason: "".to_string(),
                }),
            },
            TestCase {
                name: "SettingSerial00000",
                input: "SS   00000",
                expected_result: Ok(Message::Setting(SettingMessage::SerialNumber(
                    "00000".to_string(),
                ))),
            },
            TestCase {
                name: "SettingSerialFooBa",
                input: "SS   FooBa",
                expected_result: Ok(Message::Setting(SettingMessage::SerialNumber(
                    "FooBa".to_string(),
                ))),
            },
            TestCase {
                name: "SettingSerialFooBarBaz",
                // Out of spec
                input: "SSFooBarBaz",
                expected_result: Ok(Message::Setting(SettingMessage::SerialNumber(
                    "FooBarBaz".to_string(),
                ))),
            },
            TestCase {
                name: "SettingSerialEmpty",
                input: "SS",
                // Opinions may reasonably differ.
                expected_result: Ok(Message::Setting(SettingMessage::SerialNumber(
                    "".to_string(),
                ))),
            },
            TestCase {
                name: "SettingRunTimeSinceLastServiced0",
                input: "SR   00000",
                expected_result: Ok(Message::Setting(SettingMessage::RunTimeSinceService {
                    decaminutes: 0,
                })),
            },
            TestCase {
                name: "SettingRunTimeSinceLastServiced100",
                input: "SR   00100",
                expected_result: Ok(Message::Setting(SettingMessage::RunTimeSinceService {
                    decaminutes: 100,
                })),
            },
            TestCase {
                name: "SettingRunTimeSinceLastServiced987123",
                // Out of spec
                input: "SR987123",
                expected_result: Ok(Message::Setting(SettingMessage::RunTimeSinceService {
                    decaminutes: 987123,
                })),
            },
            TestCase {
                name: "SettingDateLastServiced_12_24",
                input: "SD   01224",
                expected_result: Ok(Message::Setting(SettingMessage::DateLastServiced {
                    month: 12,
                    year: 24,
                })),
            },
            TestCase {
                name: "SettingDateLastServiced_01_99",
                input: "SD   00199",
                expected_result: Ok(Message::Setting(SettingMessage::DateLastServiced {
                    month: 01,
                    year: 99,
                })),
            },
            TestCase {
                name: "SettingDateLastServiced99999",
                input: "SD   99999",
                expected_result: Err(ParseError {
                    received_message: "SD   99999".to_string(),
                    reason: "".to_string(),
                }),
            },
            TestCase {
                name: "SettingDateLastServicedEmpty",
                input: "SD",
                expected_result: Err(ParseError {
                    received_message: "SD".to_string(),
                    reason: "".to_string(),
                }),
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
