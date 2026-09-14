use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, Stream, StreamConfig};
use egui_baseview::baseview::dpi::LogicalSize;
use egui_baseview::{EguiWindow, EguiWindowSettings};
use gooey_kick_vst3::editor::{AtomicEditorHost, KickEditor, EDITOR_HEIGHT, EDITOR_WIDTH};
use gooey_kick_vst3::{AtomicParameters, KickAdapter, ParamId};
use std::error::Error;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

fn main() -> Result<(), Box<dyn Error>> {
    unsafe { baseview::assume_standalone_in_process() };

    let parameters = Arc::new(AtomicParameters::default());
    let audition_counter = Arc::new(AtomicU64::new(0));
    let stream = open_audio_stream(parameters.clone(), audition_counter.clone())?;
    stream.play()?;

    let audition = {
        let counter = audition_counter.clone();
        Arc::new(move || {
            counter.fetch_add(1, Ordering::Release);
        })
    };
    let editor_host = AtomicEditorHost {
        parameters,
        audition: Some(audition),
    };
    let window = EguiWindow::create(
        EguiWindowSettings::new()
            .with_title("Gooey Kick POC")
            .with_size(LogicalSize {
                width: EDITOR_WIDTH,
                height: EDITOR_HEIGHT,
            })
            .with_resizable(false),
        KickEditor::new(Box::new(editor_host), true),
    )?;
    window.run_until_closed()?;
    drop(stream);
    Ok(())
}

fn open_audio_stream(
    parameters: Arc<AtomicParameters>,
    audition_counter: Arc<AtomicU64>,
) -> Result<Stream, Box<dyn Error>> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or("no default audio output device is available")?;
    let supported = device.default_output_config()?;
    let sample_format = supported.sample_format();
    let config: StreamConfig = supported.into();
    match sample_format {
        SampleFormat::I8 => make_stream::<i8>(&device, &config, parameters, audition_counter),
        SampleFormat::I16 => make_stream::<i16>(&device, &config, parameters, audition_counter),
        SampleFormat::I32 => make_stream::<i32>(&device, &config, parameters, audition_counter),
        SampleFormat::I64 => make_stream::<i64>(&device, &config, parameters, audition_counter),
        SampleFormat::U8 => make_stream::<u8>(&device, &config, parameters, audition_counter),
        SampleFormat::U16 => make_stream::<u16>(&device, &config, parameters, audition_counter),
        SampleFormat::U32 => make_stream::<u32>(&device, &config, parameters, audition_counter),
        SampleFormat::U64 => make_stream::<u64>(&device, &config, parameters, audition_counter),
        SampleFormat::F32 => make_stream::<f32>(&device, &config, parameters, audition_counter),
        SampleFormat::F64 => make_stream::<f64>(&device, &config, parameters, audition_counter),
        format => Err(format!("unsupported output sample format: {format:?}").into()),
    }
}

fn make_stream<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    parameters: Arc<AtomicParameters>,
    audition_counter: Arc<AtomicU64>,
) -> Result<Stream, Box<dyn Error>>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = config.channels as usize;
    if channels == 0 {
        return Err("audio output reports zero channels".into());
    }
    let mut adapter = KickAdapter::new(config.sample_rate.0 as f64, parameters.load());
    let mut previous = parameters.load();
    let mut seen_audition = audition_counter.load(Ordering::Acquire);
    let stream = device.build_output_stream(
        config,
        move |output: &mut [T], _| {
            let current = parameters.load();
            for id in ParamId::ALL {
                if current.get(id) != previous.get(id) {
                    adapter.set_parameter(id, current.get(id));
                }
            }
            previous = current;

            let audition = audition_counter.load(Ordering::Acquire);
            if audition != seen_audition {
                adapter.trigger(1.0);
                seen_audition = audition;
            }
            for frame in output.chunks_mut(channels) {
                let sample = adapter.next_sample();
                let converted = T::from_sample(sample);
                for channel in frame {
                    *channel = converted;
                }
            }
        },
        |error| eprintln!("Gooey Kick POC audio stream error: {error}"),
        None,
    )?;
    Ok(stream)
}
