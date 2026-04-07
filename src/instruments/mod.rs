pub mod bass;
pub mod fm_snap;
pub mod hihat2;
pub mod kick;
pub mod poly_synth;
pub mod snare;
pub mod tom;
pub mod tom2;

pub use self::bass::*;
pub use self::fm_snap::*;
pub use self::hihat2::*;
pub use self::kick::*;
pub use self::poly_synth::*;
pub use self::snare::*;
pub use self::tom::*;
pub use self::tom2::*;

pub type HiHat = HiHat2;
pub type HiHatConfig = HiHat2Config;
