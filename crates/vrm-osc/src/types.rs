use std::{
    convert::TryFrom,
    error::Error,
    fmt,
    iter::FromIterator,
    string::FromUtf8Error,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

/// Result type used by the OSC codec.
pub type Result<T> = std::result::Result<T, OscError>;

/// Errors returned by OSC decoding and allocation-returning encoding helpers.
#[derive(Debug)]
pub enum OscError {
    StringError(FromUtf8Error),
    ReadError(std::io::ErrorKind),
    BadChar(char),
    BadPacket(&'static str),
    BadMessage(&'static str),
    BadString(&'static str),
    BadArg(String),
    BadBundle(String),
    BadAddressPattern(String),
    BadAddress(String),
    RegexError(String),
    Unimplemented,
}

impl From<FromUtf8Error> for OscError {
    fn from(value: FromUtf8Error) -> Self {
        Self::StringError(value)
    }
}

impl fmt::Display for OscError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StringError(error) => write!(f, "OSC string is not valid UTF-8: {error}"),
            Self::ReadError(kind) => write!(f, "OSC read error: {kind:?}"),
            Self::BadChar(ch) => write!(f, "bad OSC char: {ch:?}"),
            Self::BadPacket(message) => write!(f, "bad OSC packet: {message}"),
            Self::BadMessage(message) => write!(f, "bad OSC message: {message}"),
            Self::BadString(message) => write!(f, "bad OSC string: {message}"),
            Self::BadArg(message) => write!(f, "bad OSC argument: {message}"),
            Self::BadBundle(message) => write!(f, "bad OSC bundle: {message}"),
            Self::BadAddressPattern(message) => write!(f, "bad OSC address pattern: {message}"),
            Self::BadAddress(message) => write!(f, "bad OSC address: {message}"),
            Self::RegexError(message) => write!(f, "OSC regex error: {message}"),
            Self::Unimplemented => f.write_str("OSC operation is not implemented"),
        }
    }
}

impl Error for OscError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::StringError(error) => Some(error),
            _ => None,
        }
    }
}

/// A time tag in an OSC message.
///
/// OSC time tags are encoded as two big-endian 32-bit integers: seconds since
/// 1900-01-01 00:00:00 UTC, followed by fractional seconds in units of
/// 2^-32 seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OscTime {
    pub seconds: u32,
    pub fractional: u32,
}

impl OscTime {
    /// OSC timetag value that means "immediately".
    pub const IMMEDIATE: Self = Self {
        seconds: 0,
        fractional: 1,
    };

    const UNIX_OFFSET: u64 = 2_208_988_800;
    const NANOS_PER_SECOND: u128 = 1_000_000_000;
    const TWO_POW_32: u128 = 1u128 << 32;
}

impl Default for OscTime {
    fn default() -> Self {
        Self::IMMEDIATE
    }
}

impl From<(u32, u32)> for OscTime {
    fn from(time: (u32, u32)) -> Self {
        let (seconds, fractional) = time;
        Self {
            seconds,
            fractional,
        }
    }
}

impl From<OscTime> for (u32, u32) {
    fn from(time: OscTime) -> Self {
        (time.seconds, time.fractional)
    }
}

impl TryFrom<SystemTime> for OscTime {
    type Error = OscTimeError;

    fn try_from(time: SystemTime) -> std::result::Result<Self, Self::Error> {
        let unix_offset = Duration::new(Self::UNIX_OFFSET, 0);
        let duration_since_osc = match time.duration_since(UNIX_EPOCH) {
            Ok(duration_since_unix) => duration_since_unix
                .checked_add(unix_offset)
                .ok_or(OscTimeError(OscTimeErrorKind::Overflow))?,
            Err(error) => unix_offset
                .checked_sub(error.duration())
                .ok_or(OscTimeError(OscTimeErrorKind::BeforeEpoch))?,
        };
        let seconds = u32::try_from(duration_since_osc.as_secs())
            .map_err(|_| OscTimeError(OscTimeErrorKind::Overflow))?;
        let fractional =
            (((duration_since_osc.subsec_nanos() as u128) << 32) / Self::NANOS_PER_SECOND) as u32;
        Ok(Self {
            seconds,
            fractional,
        })
    }
}

impl From<OscTime> for SystemTime {
    fn from(time: OscTime) -> Self {
        let nanos =
            ((time.fractional as u128 * OscTime::NANOS_PER_SECOND) / OscTime::TWO_POW_32) as u32;
        let duration_since_osc_epoch = Duration::new(time.seconds as u64, nanos);
        let unix_offset = Duration::new(OscTime::UNIX_OFFSET, 0);
        if duration_since_osc_epoch >= unix_offset {
            UNIX_EPOCH + (duration_since_osc_epoch - unix_offset)
        } else {
            UNIX_EPOCH - (unix_offset - duration_since_osc_epoch)
        }
    }
}

impl fmt::Display for OscTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({}, {})", self.seconds, self.fractional)
    }
}

/// Error returned by conversions involving [`OscTime`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OscTimeError(OscTimeErrorKind);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OscTimeErrorKind {
    BeforeEpoch,
    Overflow,
}

impl fmt::Display for OscTimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            OscTimeErrorKind::BeforeEpoch => {
                f.write_str("time is before the OSC epoch and cannot be stored")
            }
            OscTimeErrorKind::Overflow => f.write_str("time overflows what OSC time can store"),
        }
    }
}

impl Error for OscTimeError {}

/// OSC argument value.
///
/// This mirrors the set of values exposed by `rosc` 0.11.
#[derive(Clone, Debug, PartialEq)]
pub enum OscType {
    Int(i32),
    Float(f32),
    String(String),
    Blob(Vec<u8>),
    Time(OscTime),
    Long(i64),
    Double(f64),
    Char(char),
    Color(OscColor),
    Midi(OscMidiMessage),
    Bool(bool),
    Array(OscArray),
    Nil,
    Inf,
}

macro_rules! value_impl {
    ($(($name:ident, $variant:ident, $ty:ty)),* $(,)?) => {
        $(
            impl OscType {
                pub fn $name(self) -> Option<$ty> {
                    match self {
                        Self::$variant(value) => Some(value),
                        _ => None,
                    }
                }
            }

            impl From<$ty> for OscType {
                fn from(value: $ty) -> Self {
                    Self::$variant(value)
                }
            }
        )*
    };
}

value_impl! {
    (int, Int, i32),
    (float, Float, f32),
    (string, String, String),
    (blob, Blob, Vec<u8>),
    (array, Array, OscArray),
    (long, Long, i64),
    (double, Double, f64),
    (char, Char, char),
    (color, Color, OscColor),
    (midi, Midi, OscMidiMessage),
    (bool, Bool, bool),
}

impl OscType {
    pub fn time(self) -> Option<OscTime> {
        match self {
            Self::Time(time) => Some(time),
            _ => None,
        }
    }
}

impl From<(u32, u32)> for OscType {
    fn from(time: (u32, u32)) -> Self {
        Self::Time(time.into())
    }
}

impl From<OscTime> for OscType {
    fn from(time: OscTime) -> Self {
        Self::Time(time)
    }
}

impl From<&str> for OscType {
    fn from(string: &str) -> Self {
        Self::String(string.to_owned())
    }
}

impl TryFrom<SystemTime> for OscType {
    type Error = OscTimeError;

    fn try_from(time: SystemTime) -> std::result::Result<Self, Self::Error> {
        time.try_into().map(Self::Time)
    }
}

impl fmt::Display for OscType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Int(value) => write!(f, "(i) {value}"),
            Self::Float(value) => write!(f, "(f) {value}"),
            Self::String(value) => write!(f, "(s) {value}"),
            Self::Blob(value) => {
                f.write_str("(b)")?;
                if value.is_empty() {
                    return Ok(());
                }
                f.write_str(" 0x")?;
                for octet in value {
                    write!(f, "{octet:02X}")?;
                }
                Ok(())
            }
            Self::Time(value) => write!(f, "(t) {value}"),
            Self::Long(value) => write!(f, "(h) {value}"),
            Self::Double(value) => write!(f, "(d) {value}"),
            Self::Char(value) => write!(f, "(c) {value}"),
            Self::Color(value) => write!(f, "(r) {value}"),
            Self::Midi(value) => write!(f, "(m) {value}"),
            Self::Bool(true) => f.write_str("(T)"),
            Self::Bool(false) => f.write_str("(F)"),
            Self::Array(value) => write!(f, "{value}"),
            Self::Nil => f.write_str("(N)"),
            Self::Inf => f.write_str("(I)"),
        }
    }
}

/// Represents the four bytes of an OSC MIDI argument.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OscMidiMessage {
    pub port: u8,
    pub status: u8,
    pub data1: u8,
    pub data2: u8,
}

impl fmt::Display for OscMidiMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{{port:{}, status:0x{:02X}, data:0x{:02X}{:02X}}}",
            self.port, self.status, self.data1, self.data2,
        )
    }
}

/// An OSC packet: either a message or a bundle.
#[derive(Clone, Debug, PartialEq)]
pub enum OscPacket {
    Message(OscMessage),
    Bundle(OscBundle),
}

impl fmt::Display for OscPacket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Message(message) => message.fmt(f),
            Self::Bundle(bundle) => bundle.fmt(f),
        }
    }
}

/// An OSC message with an address and zero or more arguments.
#[derive(Clone, Debug, PartialEq)]
pub struct OscMessage {
    pub addr: String,
    pub args: Vec<OscType>,
}

impl From<String> for OscMessage {
    fn from(addr: String) -> Self {
        Self { addr, args: vec![] }
    }
}

impl From<&str> for OscMessage {
    fn from(addr: &str) -> Self {
        Self {
            addr: addr.to_owned(),
            args: vec![],
        }
    }
}

impl fmt::Display for OscMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.addr)?;
        if !self.args.is_empty() {
            f.write_str(", ")?;
            for (index, arg) in self.args.iter().enumerate() {
                if index > 0 {
                    f.write_str(", ")?;
                }
                write!(f, "{arg}")?;
            }
        }
        Ok(())
    }
}

/// An OSC bundle containing nested packets and a time tag.
#[derive(Clone, Debug, PartialEq)]
pub struct OscBundle {
    pub timetag: OscTime,
    pub content: Vec<OscPacket>,
}

impl fmt::Display for OscBundle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#bundle {} {{ ", self.timetag)?;
        for (index, packet) in self.content.iter().enumerate() {
            if index > 0 {
                f.write_str("; ")?;
            }
            write!(f, "{packet}")?;
        }
        f.write_str(" }")
    }
}

/// An RGBA color argument.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OscColor {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
    pub alpha: u8,
}

impl fmt::Display for OscColor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{{{},{},{},{}}}",
            self.red, self.green, self.blue, self.alpha
        )
    }
}

/// An OSC array argument.
#[derive(Clone, Debug, PartialEq)]
pub struct OscArray {
    pub content: Vec<OscType>,
}

impl<T: Into<OscType>> FromIterator<T> for OscArray {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        Self {
            content: iter.into_iter().map(Into::into).collect(),
        }
    }
}

impl fmt::Display for OscArray {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[")?;
        for (index, item) in self.content.iter().enumerate() {
            if index > 0 {
                f.write_str(",")?;
            }
            write!(f, "{item}")?;
        }
        f.write_str("]")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn osc_time_converts_unix_epoch() {
        let time = OscTime::try_from(UNIX_EPOCH).unwrap();

        assert_eq!(
            time,
            OscTime {
                seconds: OscTime::UNIX_OFFSET as u32,
                fractional: 0,
            }
        );
        assert_eq!(SystemTime::from(time), UNIX_EPOCH);
    }

    #[test]
    fn osc_time_supports_representable_pre_unix_times() {
        let system_time = UNIX_EPOCH - Duration::new(1, 0);
        let osc_time = OscTime::try_from(system_time).unwrap();

        assert_eq!(osc_time.seconds, (OscTime::UNIX_OFFSET - 1) as u32);
        assert_eq!(osc_time.fractional, 0);
        assert_eq!(SystemTime::from(osc_time), system_time);
    }

    #[test]
    fn osc_time_converts_fractional_seconds() {
        let system_time = UNIX_EPOCH + Duration::new(0, 500_000_000);
        let osc_time = OscTime::try_from(system_time).unwrap();

        assert_eq!(osc_time.seconds, OscTime::UNIX_OFFSET as u32);
        assert_eq!(osc_time.fractional, 0x8000_0000);
        assert_eq!(SystemTime::from(osc_time), system_time);
    }
}
