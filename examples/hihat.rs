use gooey::instruments::{HiHat, HiHatConfig};

#[cfg(feature = "native")]
fn main() {
    let _ = HiHat::with_config(44100.0, HiHatConfig::short());
    println!("The legacy hihat example has been replaced. Use: cargo run --example hihat2 --features \"native,crossterm\"");
}

#[cfg(not(feature = "native"))]
fn main() {
    println!("This example is only available with the 'native' feature enabled.");
}
