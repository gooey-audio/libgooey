use gooey::instruments::KickConfig;
use std::array;
use std::sync::atomic::{AtomicU32, Ordering};

pub const PARAM_COUNT: usize = 7;
pub const STATE_MAGIC: [u8; 4] = *b"GKST";
pub const STATE_VERSION: u32 = 1;
pub const STATE_SIZE: usize = 4 + 4 + PARAM_COUNT * 4;

#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParamId {
    Frequency = 0,
    Decay = 1,
    Punch = 2,
    Click = 3,
    PitchSweep = 4,
    Drive = 5,
    Output = 6,
}

impl ParamId {
    pub const ALL: [Self; PARAM_COUNT] = [
        Self::Frequency,
        Self::Decay,
        Self::Punch,
        Self::Click,
        Self::PitchSweep,
        Self::Drive,
        Self::Output,
    ];

    pub const fn index(self) -> usize {
        self as usize
    }

    pub const fn raw(self) -> u32 {
        self as u32
    }

    pub fn from_raw(raw: u32) -> Option<Self> {
        Self::ALL.get(raw as usize).copied()
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Frequency => "Frequency",
            Self::Decay => "Decay",
            Self::Punch => "Punch",
            Self::Click => "Click",
            Self::PitchSweep => "Pitch Sweep",
            Self::Drive => "Drive",
            Self::Output => "Output",
        }
    }

    pub const fn units(self) -> &'static str {
        match self {
            Self::Frequency => "Hz",
            Self::Decay => "s",
            _ => "%",
        }
    }

    pub fn normalized_to_plain(self, normalized: f64) -> f64 {
        let normalized = sanitize(normalized as f32) as f64;
        match self {
            Self::Frequency => 30.0 + normalized * 90.0,
            Self::Decay => 0.01 + normalized * 3.99,
            _ => normalized * 100.0,
        }
    }

    pub fn plain_to_normalized(self, plain: f64) -> f64 {
        if !plain.is_finite() {
            return 0.0;
        }
        match self {
            Self::Frequency => ((plain - 30.0) / 90.0).clamp(0.0, 1.0),
            Self::Decay => ((plain - 0.01) / 3.99).clamp(0.0, 1.0),
            _ => (plain / 100.0).clamp(0.0, 1.0),
        }
    }

    pub fn display(self, normalized: f64) -> String {
        let plain = self.normalized_to_plain(normalized);
        match self {
            Self::Frequency => format!("{plain:.1} Hz"),
            Self::Decay => format!("{plain:.2} s"),
            _ => format!("{plain:.0}%"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Parameters {
    values: [f32; PARAM_COUNT],
}

impl Default for Parameters {
    fn default() -> Self {
        let config = KickConfig::default();
        Self {
            values: [
                config.frequency,
                config.oscillator_decay,
                config.punch_amount,
                config.click_amount,
                config.pitch_envelope_amount,
                config.overdrive_amount,
                config.volume,
            ],
        }
    }
}

impl Parameters {
    pub fn new(values: [f32; PARAM_COUNT]) -> Self {
        Self {
            values: values.map(sanitize),
        }
    }

    pub fn values(self) -> [f32; PARAM_COUNT] {
        self.values
    }

    pub fn get(self, id: ParamId) -> f32 {
        self.values[id.index()]
    }

    pub fn set(&mut self, id: ParamId, value: f32) {
        self.values[id.index()] = sanitize(value);
    }

    pub fn encode(self) -> [u8; STATE_SIZE] {
        let mut bytes = [0_u8; STATE_SIZE];
        bytes[..4].copy_from_slice(&STATE_MAGIC);
        bytes[4..8].copy_from_slice(&STATE_VERSION.to_le_bytes());
        for (index, value) in self.values.iter().enumerate() {
            let start = 8 + index * 4;
            bytes[start..start + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, StateError> {
        if bytes.len() != STATE_SIZE {
            return Err(StateError::WrongSize);
        }
        if bytes[..4] != STATE_MAGIC {
            return Err(StateError::BadMagic);
        }
        let version = u32::from_le_bytes(bytes[4..8].try_into().expect("fixed version slice"));
        if version != STATE_VERSION {
            return Err(StateError::UnsupportedVersion);
        }

        let mut values = [0.0; PARAM_COUNT];
        for (index, value) in values.iter_mut().enumerate() {
            let start = 8 + index * 4;
            let decoded = f32::from_le_bytes(
                bytes[start..start + 4]
                    .try_into()
                    .expect("fixed parameter slice"),
            );
            if !decoded.is_finite() {
                return Err(StateError::NonFinite);
            }
            *value = decoded.clamp(0.0, 1.0);
        }
        Ok(Self { values })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StateError {
    WrongSize,
    BadMagic,
    UnsupportedVersion,
    NonFinite,
}

pub struct AtomicParameters {
    values: [AtomicU32; PARAM_COUNT],
}

impl Default for AtomicParameters {
    fn default() -> Self {
        Self::new(Parameters::default())
    }
}

impl AtomicParameters {
    pub fn new(parameters: Parameters) -> Self {
        let values = parameters.values();
        Self {
            values: array::from_fn(|index| AtomicU32::new(values[index].to_bits())),
        }
    }

    pub fn load(&self) -> Parameters {
        Parameters::new(array::from_fn(|index| {
            f32::from_bits(self.values[index].load(Ordering::Relaxed))
        }))
    }

    pub fn get(&self, id: ParamId) -> f32 {
        f32::from_bits(self.values[id.index()].load(Ordering::Relaxed))
    }

    pub fn set(&self, id: ParamId, value: f32) {
        self.values[id.index()].store(sanitize(value).to_bits(), Ordering::Relaxed);
    }

    pub fn replace(&self, parameters: Parameters) {
        for id in ParamId::ALL {
            self.set(id, parameters.get(id));
        }
    }
}

fn sanitize(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_stable_and_contiguous() {
        for (expected, id) in ParamId::ALL.into_iter().enumerate() {
            assert_eq!(id.raw(), expected as u32);
            assert_eq!(ParamId::from_raw(expected as u32), Some(id));
        }
        assert_eq!(ParamId::from_raw(7), None);
    }

    #[test]
    fn defaults_come_from_kick_config() {
        let config = KickConfig::default();
        assert_eq!(
            Parameters::default().values(),
            [
                config.frequency,
                config.oscillator_decay,
                config.punch_amount,
                config.click_amount,
                config.pitch_envelope_amount,
                config.overdrive_amount,
                config.volume,
            ]
        );
    }

    #[test]
    fn display_and_plain_conversions_match_contract() {
        assert_eq!(ParamId::Frequency.display(0.0), "30.0 Hz");
        assert_eq!(ParamId::Frequency.display(1.0), "120.0 Hz");
        assert_eq!(ParamId::Decay.display(0.0), "0.01 s");
        assert_eq!(ParamId::Decay.display(1.0), "4.00 s");
        assert_eq!(ParamId::Punch.display(0.375), "38%");
        assert!((ParamId::Frequency.plain_to_normalized(75.0) - 0.5).abs() < 1e-12);
        assert!((ParamId::Decay.plain_to_normalized(2.005) - 0.5).abs() < 1e-12);
    }

    #[test]
    fn external_values_are_finite_and_clamped() {
        let parameters = Parameters::new([-1.0, 2.0, f32::NAN, 0.2, 0.3, 0.4, 0.5]);
        assert_eq!(parameters.values()[..3], [0.0, 1.0, 0.0]);
    }

    #[test]
    fn state_round_trips_exactly() {
        let parameters = Parameters::new([0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7]);
        assert_eq!(Parameters::decode(&parameters.encode()), Ok(parameters));
    }

    #[test]
    fn malformed_state_is_rejected_without_partial_commit() {
        let atomics = AtomicParameters::default();
        let before = atomics.load();

        let mut bytes = Parameters::new([0.9; PARAM_COUNT]).encode();
        bytes[8 + 4 * 4..8 + 5 * 4].copy_from_slice(&f32::NAN.to_le_bytes());
        assert_eq!(Parameters::decode(&bytes), Err(StateError::NonFinite));
        assert_eq!(atomics.load(), before);

        assert_eq!(Parameters::decode(&bytes[..12]), Err(StateError::WrongSize));
        bytes = Parameters::default().encode();
        bytes[4..8].copy_from_slice(&2_u32.to_le_bytes());
        assert_eq!(
            Parameters::decode(&bytes),
            Err(StateError::UnsupportedVersion)
        );
    }
}
