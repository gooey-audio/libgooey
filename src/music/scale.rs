use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScaleType {
    Major,
    NaturalMinor,
}

impl ScaleType {
    /// Returns the semitone offsets from root for each scale degree (7 degrees)
    pub fn intervals(self) -> &'static [u8; 7] {
        match self {
            ScaleType::Major => &[0, 2, 4, 5, 7, 9, 11],
            ScaleType::NaturalMinor => &[0, 2, 3, 5, 7, 8, 10],
        }
    }
}

impl fmt::Display for ScaleType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScaleType::Major => write!(f, "Major"),
            ScaleType::NaturalMinor => write!(f, "Minor"),
        }
    }
}
