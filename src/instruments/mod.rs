pub mod fm_snap;
pub mod hihat2;
pub mod kick;
pub mod snare;
pub mod tom;
pub mod tom2;

pub use self::fm_snap::*;
pub use self::hihat2::*;
pub use self::kick::*;
pub use self::snare::*;
pub use self::tom::*;
pub use self::tom2::*;

pub type HiHat = HiHat2;
pub type HiHatConfig = HiHat2Config;
