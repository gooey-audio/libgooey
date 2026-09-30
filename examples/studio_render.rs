//! Device-free session export and stress runner.
use anyhow::{bail, Context, Result};
use gooey::studio::{Control, Session, Studio};
fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let mut session = Session::demo();
    let mut export = "studio-mix.wav".to_string();
    let mut bars = 4;
    let mut stress = 0;
    let mut save = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--export" => export = args.next().context("--export needs a path")?,
            "--load" => session = Session::load(args.next().context("--load needs a path")?)?,
            "--save" => save = Some(args.next().context("--save needs a path")?),
            "--bars" => bars = args.next().context("--bars needs a count")?.parse()?,
            "--stress" => {
                stress = args
                    .next()
                    .context("--stress needs a block count")?
                    .parse::<usize>()?
            }
            "--help" => {
                println!("studio_render [--load session.json] [--save session.json] [--export mix.wav] [--bars 4] [--stress 10000]");
                return Ok(());
            }
            _ => bail!("unknown argument: {arg}"),
        }
    }
    if let Some(path) = save {
        session.save(path)?;
    }
    if stress > 0 {
        let mut engine = Studio::new(session.clone(), 48000)?;
        engine.play(true);
        let mut buf = [0.0; 1024];
        let start = std::time::Instant::now();
        for i in 0..stress {
            engine.set_control(Control::Gain(i % 4), (i % 100) as f32 / 100.0)?;
            engine.set_mute((i / 4) % 4, i % 17 == 0)?;
            engine.render(&mut buf)?;
            anyhow::ensure!(buf.iter().all(|s| s.is_finite()), "invalid stress audio");
        }
        println!(
            "stress: {stress} blocks, {:.2}s elapsed, finite audio",
            start.elapsed().as_secs_f64()
        );
    }
    let report = session.export_wav(&export, bars, 2.0)?;
    println!(
        "exported {export}: {} stereo frames at 48000 Hz, peak {:.4}, RMS {:.4}",
        report.frames, report.peak, report.rms
    );
    Ok(())
}
