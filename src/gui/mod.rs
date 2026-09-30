//! Unified graphical laboratories and public panel/audio extension contract.
mod audio;
mod experiments;
mod poly;
mod resonator;
#[cfg(feature = "studio-gui")]
pub mod studio;
pub use audio::*;
pub use eframe::egui;
use std::sync::{atomic::Ordering, Arc};

/// Mount an editor in the same shell. Panels never create streams, workers or
/// eframe applications. The shell stops audio BEFORE calling deactivate.
pub trait GuiPanel {
    fn name(&self) -> &str;
    fn ui(&mut self, ctx: &egui::Context);
    fn renderer(&mut self, sample_rate: f32, health: Arc<AudioHealth>) -> Box<dyn BlockRenderer>;
    fn deactivate(&mut self) {}
    /// Deterministic audition used by the device-free harness.
    fn exercise(&mut self) {}
    /// GUI/control-side stress progression, called once per simulated second.
    /// Extensions can vary presets/routes here; no audio worker is created.
    fn exercise_step(&mut self, _step: u64) {
        self.exercise();
    }
}
pub type PanelFactory = Box<dyn Fn(f32) -> Box<dyn GuiPanel>>;

/// Reusable control metadata. Engineering-unit controls can supply their own
/// bounds; legacy indexed instrument parameters use normalized descriptors.
#[derive(Clone, Copy, Debug)]
pub struct ParameterDescriptor {
    pub id: u32,
    pub name: &'static str,
    pub min: f32,
    pub max: f32,
}
impl ParameterDescriptor {
    pub const fn normalized(id: u32, name: &'static str) -> Self {
        Self {
            id,
            name,
            min: 0.0,
            max: 1.0,
        }
    }
    pub fn ui(&self, ui: &mut egui::Ui, value: &mut f32) -> bool {
        parameter(ui, self.name, value, self.min..=self.max)
    }
}
pub fn poly_parameters() -> impl Iterator<Item = ParameterDescriptor> {
    poly::PARAM_NAMES
        .iter()
        .enumerate()
        .map(|(id, name)| ParameterDescriptor::normalized(id as u32, name))
}

/// GUI-side lock recovery is observable. Audio adapters use try_lock instead.
pub fn control_lock<'a, T>(
    mutex: &'a std::sync::Mutex<T>,
    health: &AudioHealth,
) -> std::sync::MutexGuard<'a, T> {
    mutex.lock().unwrap_or_else(|poison| {
        health.control_errors.fetch_add(1, Ordering::Relaxed);
        poison.into_inner()
    })
}

pub fn parameter(
    ui: &mut egui::Ui,
    name: &str,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
) -> bool {
    ui.add(egui::Slider::new(value, range).text(name)).changed()
}

/// Engineering-unit frequency controls share the slider widget while retaining
/// a logarithmic gesture scale (the value and bounds stay in native units).
pub fn parameter_logarithmic(
    ui: &mut egui::Ui,
    name: &str,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
) -> bool {
    ui.add(egui::Slider::new(value, range).text(name).logarithmic(true))
        .changed()
}

pub struct Keyboard {
    held: [Option<u8>; 28],
    pub octave: i32,
}
impl Default for Keyboard {
    fn default() -> Self {
        Self {
            held: [None; 28],
            octave: 4,
        }
    }
}
impl Keyboard {
    pub fn release(&mut self) {
        self.held.fill(None);
    }
    /// Pure input transition state, shared by tests and the desktop keyboard.
    pub fn update(&mut self, enabled: bool, down: [bool; 25], chord: bool) -> Vec<(u8, bool)> {
        let mut old = [false; 128];
        for note in self.held.iter().flatten() {
            old[*note as usize] = true;
        }
        for i in 0..28 {
            let pressed = enabled && if i < 25 { down[i] } else { chord };
            let offset = if i < 25 { i } else { [0, 4, 7][i - 25] };
            let note = ((self.octave + 1) * 12 + offset as i32).clamp(0, 127) as u8;
            let next = pressed.then_some(note);
            self.held[i] = next;
        }
        let mut next = [false; 128];
        for note in self.held.iter().flatten() {
            next[*note as usize] = true;
        }
        (0..128)
            .filter(|i| old[*i] != next[*i])
            .map(|i| (i as u8, next[i]))
            .collect()
    }
    pub fn ui(&mut self, ui: &mut egui::Ui) -> Vec<(u8, bool)> {
        use egui::Key;
        let keys = [
            Key::Z,
            Key::S,
            Key::X,
            Key::D,
            Key::C,
            Key::V,
            Key::G,
            Key::B,
            Key::H,
            Key::N,
            Key::J,
            Key::M,
            Key::Q,
            Key::Num2,
            Key::W,
            Key::Num3,
            Key::E,
            Key::R,
            Key::Num5,
            Key::T,
            Key::Num6,
            Key::Y,
            Key::Num7,
            Key::U,
            Key::I,
        ];
        let enabled = ui
            .ctx()
            .input(|i| i.focused && !i.modifiers.ctrl && !i.modifiers.alt && !i.modifiers.mac_cmd)
            && !ui.ctx().wants_keyboard_input();
        let mut down = [false; 25];
        ui.horizontal_wrapped(|ui| {
            for (i, key) in keys.iter().enumerate() {
                let button = ui
                    .add(egui::Button::new(format!("{:?}", key)).selected(self.held[i].is_some()));
                down[i] =
                    ui.input(|input| input.key_down(*key)) || button.is_pointer_button_down_on();
            }
        });
        let chord = ui
            .add(egui::Button::new("C major chord / Space"))
            .is_pointer_button_down_on()
            || ui.input(|input| input.key_down(Key::Space));
        self.update(enabled, down, chord)
    }
}

pub fn builtin_factories() -> Vec<PanelFactory> {
    builtin_factories_with_demo(false)
}
fn builtin_factories_with_demo(_demo: bool) -> Vec<PanelFactory> {
    #[allow(unused_mut)]
    let mut factories: Vec<PanelFactory> = vec![
        Box::new(|rate| Box::new(poly::PolyPanel::new(rate))),
        Box::new(resonator::panel),
        Box::new(|rate| Box::new(experiments::Experiments::new(rate))),
    ];
    #[cfg(feature = "studio-gui")]
    factories.push(Box::new(move |rate| {
        Box::new(studio::StudioPanel::new(
            rate,
            if _demo {
                crate::studio::Session::demo()
            } else {
                crate::studio::Session::default()
            },
        ))
    }));
    factories
}

/// Shared shell. Extensions supply factories; construction occurs at the
/// negotiated device rate. Only the selected panel has an active renderer.
pub struct OmniApp {
    panels: Vec<Box<dyn GuiPanel>>,
    selected: usize,
    pub audio: GuiAudio,
    silent: bool,
    error: Option<String>,
    paused: bool,
}
impl OmniApp {
    pub fn new(
        factories: Vec<PanelFactory>,
        selected: Option<&str>,
        silent: bool,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(!factories.is_empty(), "At least one panel is required");
        let rate = GuiAudio::preferred_rate(silent);
        let panels: Vec<_> = factories.into_iter().map(|factory| factory(rate)).collect();
        let selected = match selected {
            Some(name) => panels
                .iter()
                .position(|panel| panel.name().eq_ignore_ascii_case(name))
                .ok_or_else(|| anyhow::anyhow!("Unknown panel {name}"))?,
            None => 0,
        };
        let mut app = Self {
            panels,
            selected,
            audio: GuiAudio::default(),
            silent,
            error: None,
            paused: false,
        };
        app.audio.sample_rate = rate;
        app.activate();
        Ok(app)
    }
    fn activate(&mut self) {
        let renderer = self.panels[self.selected].renderer(
            self.audio.sample_rate,
            Arc::clone(&self.audio.telemetry.health),
        );
        if let Err(error) = self.audio.start(renderer, self.silent) {
            self.error = Some(error.to_string());
            self.silent = true;
            let renderer = self.panels[self.selected].renderer(
                self.audio.sample_rate,
                Arc::clone(&self.audio.telemetry.health),
            );
            if let Err(error) = self.audio.start(renderer, true) {
                self.error = Some(error.to_string());
            }
        }
    }
    pub fn select(&mut self, index: usize) -> anyhow::Result<()> {
        anyhow::ensure!(index < self.panels.len(), "Panel index out of range");
        if index != self.selected {
            self.audio.stop();
            self.panels[self.selected].deactivate();
            self.selected = index;
            if !self.paused {
                self.activate();
            }
        }
        Ok(())
    }
    pub fn draw(&mut self, ctx: &egui::Context) {
        let mut selected = self.selected;
        egui::TopBottomPanel::top("omni-shell").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.heading("Gooey Omni Lab");
                for (i, panel) in self.panels.iter().enumerate() {
                    ui.selectable_value(&mut selected, i, panel.name());
                }
                if ui
                    .button(if self.paused {
                        "Resume audio"
                    } else {
                        "Stop audio"
                    })
                    .clicked()
                {
                    self.paused = !self.paused;
                    if self.paused {
                        self.audio.stop();
                        self.panels[self.selected].deactivate();
                    } else {
                        self.activate();
                    }
                }
                ui.label(&self.audio.status);
            });
            if let Some(error) = &self.error {
                ui.colored_label(egui::Color32::YELLOW, error);
            }
        });
        let _ = self.select(selected);
        egui::TopBottomPanel::bottom("omni-debug")
            .resizable(true)
            .default_height(230.0)
            .show(ctx, |ui| {
                diagnostics(ui, &self.audio);
            });
        self.panels[self.selected].ui(ctx);
        ctx.request_repaint_after(std::time::Duration::from_millis(16));
    }
}
impl Drop for OmniApp {
    fn drop(&mut self) {
        self.audio.stop();
        self.panels[self.selected].deactivate();
    }
}
impl eframe::App for OmniApp {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.draw(ctx);
    }
}

pub fn diagnostics(ui: &mut egui::Ui, audio: &GuiAudio) {
    let health = &audio.telemetry.health;
    ui.horizontal_wrapped(|ui| {
        for (name, counter) in [
            ("Blocks", &health.blocks),
            ("Non-finite", &health.non_finite),
            ("Lock misses", &health.contention),
            ("Over budget", &health.overruns),
            ("Max µs", &health.max_block_us),
            ("Device errors", &health.device_errors),
            ("Control errors", &health.control_errors),
            ("Render failures", &health.render_failures),
        ] {
            ui.label(format!("{name}: {}", counter.load(Ordering::Relaxed)));
        }
    });
    let samples = audio.telemetry.snapshot();
    let peak = samples.iter().fold(0.0_f32, |peak, frame| {
        peak.max(frame.l.abs()).max(frame.r.abs())
    });
    let rms = (samples
        .iter()
        .map(|f| (f.l * f.l + f.r * f.r) * 0.5)
        .sum::<f32>()
        / samples.len().max(1) as f32)
        .sqrt();
    ui.label(format!(
        "Stereo output · peak {peak:.3} · RMS {rms:.3} · bounded {SCOPE_SIZE}-frame history"
    ));
    ui.columns(2, |columns| {
        plot(
            &mut columns[0],
            "Stereo waveform L/R",
            &samples.iter().map(|f| f.l).collect::<Vec<_>>(),
            Some(&samples.iter().map(|f| f.r).collect::<Vec<_>>()),
            false,
        );
        let spectrum = stereo_spectrum(&samples);
        plot(
            &mut columns[1],
            &format!(
                "Stereo power spectrum · −100–0 dB · 0–{:.0} Hz",
                audio.sample_rate / 2.0
            ),
            &spectrum,
            None,
            true,
        );
    });
}

/// GUI-side Hann FFT, combining channel powers rather than mono summing so
/// anti-phase stereo signals remain visible. Values map −100..0 dB to 0..1.
pub fn stereo_spectrum(samples: &[crate::StereoFrame]) -> Vec<f32> {
    let n = samples.len().min(1024);
    if n < 2 {
        return Vec::new();
    }
    let channel = |right: bool| {
        samples
            .iter()
            .rev()
            .take(n)
            .enumerate()
            .map(|(i, frame)| {
                let window = 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / n as f32).cos();
                rustfft::num_complex::Complex::new(
                    if right {
                        frame.r * window
                    } else {
                        frame.l * window
                    },
                    0.0,
                )
            })
            .collect::<Vec<_>>()
    };
    let mut left = channel(false);
    let mut right = channel(true);
    let fft = rustfft::FftPlanner::new().plan_fft_forward(n);
    fft.process(&mut left);
    fft.process(&mut right);
    left.iter()
        .zip(&right)
        .take(n / 2)
        .map(|(l, r)| {
            let magnitude = ((l.norm_sqr() + r.norm_sqr()) * 0.5).sqrt() / n as f32;
            (magnitude.max(1e-6).log10() * 20.0 + 100.0) / 100.0
        })
        .collect()
}
fn plot(ui: &mut egui::Ui, label: &str, values: &[f32], second: Option<&[f32]>, positive: bool) {
    ui.label(label);
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), 120.0),
        egui::Sense::hover(),
    );
    ui.painter()
        .rect_filled(rect, 2.0, egui::Color32::from_gray(15));
    for (values, color) in [
        (values, egui::Color32::LIGHT_GREEN),
        (second.unwrap_or(&[]), egui::Color32::LIGHT_BLUE),
    ] {
        let points: Vec<_> = values
            .iter()
            .enumerate()
            .map(|(i, value)| {
                egui::pos2(
                    rect.left() + i as f32 / values.len().max(1) as f32 * rect.width(),
                    if positive {
                        rect.bottom() - value.clamp(0.0, 1.0) * rect.height()
                    } else {
                        rect.center().y - value.clamp(-1.0, 1.0) * rect.height() * 0.45
                    },
                )
            })
            .collect();
        if points.len() > 1 {
            ui.painter()
                .add(egui::Shape::line(points, egui::Stroke::new(1.0_f32, color)));
        }
    }
}

/// Single desktop entrypoint, also accepting --silent, --panel NAME, --list,
/// and --stress SECONDS (accelerated device-free audio plus headless layout).
pub fn run(default_panel: Option<&str>) -> anyhow::Result<()> {
    let demo = std::env::args().any(|arg| arg == "--demo");
    #[allow(unused_mut)]
    let mut factories = builtin_factories_with_demo(demo);
    #[cfg(feature = "studio-gui")]
    {
        let args: Vec<_> = std::env::args().collect();
        if let Some(index) = args.iter().position(|arg| arg == "--load") {
            let path = args
                .get(index + 1)
                .filter(|path| !path.starts_with("--"))
                .ok_or_else(|| anyhow::anyhow!("--load requires a session path"))?
                .clone();
            // Startup I/O precedes the audio host and stays off its callback.
            let song = std::thread::spawn(move || crate::studio::Session::load(path))
                .join()
                .map_err(|_| anyhow::anyhow!("Session load worker panicked"))??;
            *factories.last_mut().expect("studio factory") =
                Box::new(move |rate| Box::new(studio::StudioPanel::new(rate, song.clone())));
        }
    }
    run_with(factories, default_panel)
}
/// Compatibility graphical entry for formerly waveform-equipped terminal labs.
pub fn run_experiment(
    instrument: &'static str,
    effect: &'static str,
    sequence: bool,
    modulation: bool,
) -> anyhow::Result<()> {
    let mut factories = builtin_factories();
    factories[2] = Box::new(move |rate| {
        Box::new(experiments::Experiments::configured(
            rate, instrument, effect, sequence, modulation,
        ))
    });
    run_with(factories, Some("Experiments"))
}
pub fn run_with(factories: Vec<PanelFactory>, default_panel: Option<&str>) -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            #[cfg(feature = "studio-gui")]
            "--demo" => index += 1,
            #[cfg(feature = "studio-gui")]
            "--load" => {
                anyhow::ensure!(
                    args.get(index + 1)
                        .is_some_and(|value| !value.starts_with("--")),
                    "--load requires a session path"
                );
                index += 2;
            }
            "--silent" | "--list" | "--help" => index += 1,
            "--panel" | "--stress" | "--soak" => {
                anyhow::ensure!(
                    args.get(index + 1)
                        .is_some_and(|value| !value.starts_with("--")),
                    "{} requires a value",
                    args[index]
                );
                index += 2;
            }
            argument => anyhow::bail!("Unknown option {argument}; use --help"),
        }
    }
    if args.iter().any(|arg| arg == "--help") {
        println!("Gooey Omni Lab: --panel NAME --silent --list --stress AUDIO_SECONDS --soak WALL_SECONDS\nWith studio-gui: --demo or --load SESSION initializes Loop Studio (load takes precedence).\nStress renders all panels at 44100/48000 Hz without a device/window. Soak repeatedly switches the real silent worker with headless GUI frames.");
        return Ok(());
    }
    let option = |name: &str| {
        args.iter()
            .position(|arg| arg == name)
            .and_then(|i| args.get(i + 1))
    };
    if args.iter().any(|arg| arg == "--list") {
        for factory in factories {
            println!("{}", factory(44_100.0).name());
        }
        return Ok(());
    }
    if let Some(seconds) = option("--stress") {
        return stress(factories, seconds.parse()?);
    }
    if let Some(seconds) = option("--soak") {
        return soak(factories, seconds.parse()?);
    }
    let panel = option("--panel").map(String::as_str).or(default_panel);
    let app = OmniApp::new(factories, panel, args.iter().any(|arg| arg == "--silent"))?;
    eframe::run_native(
        "Gooey Omni Lab",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([1440.0, 1000.0])
                .with_min_inner_size([900.0, 700.0]),
            ..Default::default()
        },
        Box::new(move |_| Ok(Box::new(app))),
    )
    .map_err(|error| anyhow::anyhow!(error.to_string()))
}
pub fn stress(factories: Vec<PanelFactory>, seconds: u64) -> anyhow::Result<()> {
    anyhow::ensure!(
        (1..=86_400).contains(&seconds),
        "Stress seconds must be 1–86400"
    );
    for rate in [44_100.0, 48_000.0] {
        for factory in &factories {
            let mut panel = factory(rate);
            let telemetry = Arc::new(Telemetry::default());
            let mut renderer = panel.renderer(rate, Arc::clone(&telemetry.health));
            let mut scratch = [crate::StereoFrame::default(); BLOCK_SIZE];
            let mut last_step = u64::MAX;
            let blocks = (seconds as f64 * rate as f64 / BLOCK_SIZE as f64) as usize;
            let ctx = egui::Context::default();
            for block in 0..blocks {
                let step = (block * BLOCK_SIZE) as u64 / rate as u64;
                if step != last_step {
                    panel.exercise_step(step);
                    let _ = ctx.run(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(
                                egui::Pos2::ZERO,
                                egui::vec2(1440.0, 1000.0),
                            )),
                            ..Default::default()
                        },
                        |ctx| panel.ui(ctx),
                    );
                    last_step = step;
                }
                audio::render_block(renderer.as_mut(), &mut scratch, rate, &telemetry);
            }
            anyhow::ensure!(
                telemetry.health.non_finite.load(Ordering::Relaxed) == 0,
                "{} produced non-finite audio",
                panel.name()
            );
            anyhow::ensure!(
                telemetry.health.render_failures.load(Ordering::Relaxed) == 0,
                "{} renderer failed",
                panel.name()
            );
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1440.0, 1000.0),
                    )),
                    ..Default::default()
                },
                |ctx| panel.ui(ctx),
            );
            panel.deactivate();
            println!(
                "PASS {} · {rate} Hz · {seconds}s · {} blocks",
                panel.name(),
                telemetry.health.blocks.load(Ordering::Relaxed)
            );
        }
    }
    Ok(())
}

/// Wall-clock device-free lifecycle soak, unlike accelerated audio stress.
pub fn soak(factories: Vec<PanelFactory>, seconds: u64) -> anyhow::Result<()> {
    anyhow::ensure!(
        (1..=86_400).contains(&seconds),
        "Soak seconds must be 1–86400"
    );
    let mut app = OmniApp::new(factories, None, true)?;
    let context = egui::Context::default();
    let start = std::time::Instant::now();
    let mut last_second = u64::MAX;
    while start.elapsed().as_secs() < seconds {
        let second = start.elapsed().as_secs();
        if second != last_second {
            if last_second != u64::MAX {
                let health = &app.audio.telemetry.health;
                anyhow::ensure!(
                    health.non_finite.load(Ordering::Relaxed) == 0
                        && health.render_failures.load(Ordering::Relaxed) == 0,
                    "Soak renderer failed"
                );
                app.select((app.selected + 1) % app.panels.len())?;
            }
            app.panels[app.selected].exercise_step(second);
            last_second = second;
        }
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1440.0, 1000.0),
            )),
            focused: second % 4 != 3,
            ..Default::default()
        };
        let output = context.run(input, |ctx| app.draw(ctx));
        anyhow::ensure!(!output.shapes.is_empty(), "Empty shell layout");
        std::thread::sleep(std::time::Duration::from_millis(16));
    }
    app.audio.stop();
    let health = &app.audio.telemetry.health;
    anyhow::ensure!(
        health.non_finite.load(Ordering::Relaxed) == 0
            && health.render_failures.load(Ordering::Relaxed) == 0,
        "Soak renderer failed"
    );
    let total_blocks = health.blocks.load(Ordering::Relaxed);
    anyhow::ensure!(total_blocks > 0, "Silent worker did not render");
    println!("PASS wall-clock soak · {seconds}s · {seconds} panel auditions · {total_blocks} blocks · no non-finite audio/render failures");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::StereoFrame;

    #[test]
    fn stereo_fft_keeps_antiphase_signals_and_expected_frequency_bin() {
        let samples: Vec<_> = (0..1024)
            .map(|i| {
                let l = (std::f32::consts::TAU * 16.0 * i as f32 / 1024.0).sin();
                StereoFrame { l, r: -l }
            })
            .collect();
        let spectrum = stereo_spectrum(&samples);
        let peak = spectrum
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .expect("bins");
        assert_eq!(peak.0, 16);
        assert!(*peak.1 > 0.8);
        assert_eq!(poly_parameters().count(), 30);
    }

    #[test]
    fn caught_renderer_failure_is_silenced_and_counted() {
        let mut renderer =
            RenderAdapter(|_: &mut [StereoFrame], _: f32| panic!("intentional renderer probe"));
        let telemetry = exercise(&mut renderer, 44_100.0, 1);
        assert_eq!(telemetry.health.render_failures.load(Ordering::Relaxed), 1);
        assert!(telemetry
            .snapshot()
            .iter()
            .all(|frame| frame.l == 0.0 && frame.r == 0.0));
    }

    #[test]
    fn actual_egui_keyboard_focus_loss_releases_notes() {
        let context = egui::Context::default();
        let mut keyboard = Keyboard::default();
        let mut events = Vec::new();
        let input = egui::RawInput {
            focused: true,
            events: vec![egui::Event::Key {
                key: egui::Key::Z,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::default(),
            }],
            ..Default::default()
        };
        let _ = context.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                events = keyboard.ui(ui);
            });
        });
        assert_eq!(events, vec![(60, true)]);
        let _ = context.run(
            egui::RawInput {
                focused: false,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    events = keyboard.ui(ui);
                });
            },
        );
        assert_eq!(events, vec![(60, false)]);
    }

    #[test]
    fn returning_renderer_gets_the_shared_monotonic_clock() {
        struct Clock {
            last: Arc<std::sync::atomic::AtomicU64>,
        }
        impl BlockRenderer for Clock {
            fn set_time(&mut self, seconds: f64) {
                let old = f64::from_bits(self.last.load(Ordering::Relaxed));
                assert!(seconds >= old, "host clock went backwards");
                self.last.store(seconds.to_bits(), Ordering::Relaxed);
            }
            fn render(&mut self, frames: &mut [StereoFrame], _: f32) {
                frames.fill(StereoFrame::default());
            }
        }
        let last = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let mut audio = GuiAudio::default();
        for _ in 0..4 {
            audio
                .start(
                    Box::new(Clock {
                        last: Arc::clone(&last),
                    }),
                    true,
                )
                .expect("worker");
            std::thread::sleep(std::time::Duration::from_millis(20));
            audio.stop();
        }
        assert!(f64::from_bits(last.load(Ordering::Relaxed)) > 0.0);
        assert_eq!(
            audio
                .telemetry
                .health
                .render_failures
                .load(Ordering::Relaxed),
            0
        );
    }

    #[test]
    fn telemetry_is_bounded_and_sanitizes_non_finite() {
        let telemetry = Telemetry::default();
        let mut frames = vec![
            StereoFrame {
                l: f32::NAN,
                r: f32::INFINITY
            };
            SCOPE_SIZE + 100
        ];
        telemetry.publish(&mut frames);
        assert_eq!(telemetry.snapshot().len(), SCOPE_SIZE);
        assert_eq!(
            telemetry.health.non_finite.load(Ordering::Relaxed),
            (frames.len() * 2) as u64
        );
        assert!(telemetry
            .snapshot()
            .iter()
            .all(|f| f.l == 0.0 && f.r == 0.0));
    }
    #[test]
    fn channel_formats_and_interleaved_adapter() {
        let frame = StereoFrame { l: 0.25, r: -0.5 };
        assert_eq!(channel_sample(frame, 0, 1), -0.125);
        assert_eq!(channel_sample(frame, 0, 2), 0.25);
        assert_eq!(channel_sample(frame, 1, 6), -0.5);
        assert_eq!(channel_sample(frame, 5, 6), 0.0);
        let mut adapter = InterleavedAdapter::new(|out: &mut [f32], rate: f32| {
            assert!(out.len() <= BLOCK_SIZE * 2);
            assert_eq!(rate, 48_000.0);
            for frame in out.chunks_exact_mut(2) {
                frame.copy_from_slice(&[0.25, -0.5]);
            }
        });
        let mut frames = [StereoFrame::default(); BLOCK_SIZE * 3 + 1];
        adapter.render(&mut frames, 48_000.0);
        assert!(frames.iter().all(|f| f.l == frame.l && f.r == frame.r));
        #[cfg(feature = "native")]
        {
            use cpal::Sample;
            assert_eq!(i16::from_sample(-1.0_f32), i16::MIN);
            assert_eq!(u16::from_sample(0.0_f32), 32768);
        }
    }
    #[test]
    fn keyboard_focus_chord_overlap_and_octave_release() {
        let mut keyboard = Keyboard::default();
        let mut down = [false; 25];
        down[0] = true;
        assert_eq!(keyboard.update(true, down, false), vec![(60, true)]);
        assert_eq!(
            keyboard.update(true, down, true),
            vec![(64, true), (67, true)]
        );
        assert_eq!(
            keyboard.update(true, down, false),
            vec![(64, false), (67, false)]
        );
        keyboard.octave = 5;
        assert_eq!(
            keyboard.update(true, down, false),
            vec![(60, false), (72, true)]
        );
        assert_eq!(keyboard.update(false, down, false), vec![(72, false)]);
        assert!(keyboard.update(false, down, false).is_empty());
    }
    #[test]
    fn engine_adapter_does_not_wait_for_ui_lock() {
        let engine = Arc::new(std::sync::Mutex::new(crate::engine::Engine::new(44_100.0)));
        let health = Arc::new(AudioHealth::default());
        let mut renderer = EngineRenderer::new(Arc::clone(&engine), Arc::clone(&health));
        let _guard = engine.lock().expect("test engine");
        let mut frames = [StereoFrame { l: 1.0, r: 1.0 }; BLOCK_SIZE];
        renderer.render(&mut frames, 44_100.0);
        assert!(frames.iter().all(|f| f.l == 0.0 && f.r == 0.0));
        assert_eq!(health.contention.load(Ordering::Relaxed), 1);
    }
    #[test]
    fn switching_and_drop_never_overlap_workers() {
        struct Probe {
            active: Arc<std::sync::atomic::AtomicUsize>,
        }
        impl Drop for Probe {
            fn drop(&mut self) {
                self.active.fetch_sub(1, Ordering::SeqCst);
            }
        }
        impl BlockRenderer for Probe {
            fn render(&mut self, frames: &mut [StereoFrame], _: f32) {
                frames.fill(StereoFrame::default());
            }
        }
        struct Panel {
            name: String,
            active: Arc<std::sync::atomic::AtomicUsize>,
            peak: Arc<std::sync::atomic::AtomicUsize>,
            cleanups: Arc<std::sync::atomic::AtomicUsize>,
        }
        impl GuiPanel for Panel {
            fn name(&self) -> &str {
                &self.name
            }
            fn ui(&mut self, ctx: &egui::Context) {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.label(&self.name);
                });
            }
            fn renderer(&mut self, _: f32, _: Arc<AudioHealth>) -> Box<dyn BlockRenderer> {
                let count = self.active.fetch_add(1, Ordering::SeqCst) + 1;
                self.peak.fetch_max(count, Ordering::SeqCst);
                Box::new(Probe {
                    active: Arc::clone(&self.active),
                })
            }
            fn deactivate(&mut self) {
                assert_eq!(self.active.load(Ordering::SeqCst), 0);
                self.cleanups.fetch_add(1, Ordering::SeqCst);
            }
        }
        let active = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let peak = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let cleanups = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let factories: Vec<PanelFactory> = (0..3)
            .map(|i| {
                let active = Arc::clone(&active);
                let peak = Arc::clone(&peak);
                let cleanups = Arc::clone(&cleanups);
                Box::new(move |_| {
                    Box::new(Panel {
                        name: i.to_string(),
                        active: Arc::clone(&active),
                        peak: Arc::clone(&peak),
                        cleanups: Arc::clone(&cleanups),
                    }) as Box<dyn GuiPanel>
                }) as PanelFactory
            })
            .collect();
        let mut app = OmniApp::new(factories, None, true).expect("silent host");
        for i in 0..60 {
            app.select((i + 1) % 3).expect("switch");
        }
        assert!(app.select(100).is_err());
        drop(app);
        assert_eq!(active.load(Ordering::SeqCst), 0);
        assert_eq!(peak.load(Ordering::SeqCst), 1);
        assert_eq!(cleanups.load(Ordering::SeqCst), 61);
    }
    #[test]
    fn every_panel_has_device_free_layout_and_audio_at_both_rates() {
        stress(builtin_factories(), 1).expect("smoke");
        let mut app = OmniApp::new(builtin_factories(), None, true).expect("shell");
        let ctx = egui::Context::default();
        for size in [[900.0, 700.0], [1440.0, 1000.0]] {
            for index in 0..app.panels.len() {
                app.select(index).expect("panel");
                let output = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(size[0], size[1]),
                        )),
                        ..Default::default()
                    },
                    |ctx| app.draw(ctx),
                );
                assert!(!output.shapes.is_empty());
            }
        }
    }
}
