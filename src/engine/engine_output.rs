#[cfg(feature = "native")]
use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    SizedSample, FromSample, Sample, Stream, Device, StreamConfig,
};
use super::Engine;
use std::sync::{Arc, Mutex};
use std::time::Instant;

#[cfg(feature = "native")]
pub struct EngineOutput {
    stream: Option<Stream>,
    device: Option<Device>,
    config: Option<StreamConfig>,
    sample_rate: f32,
    is_active: bool,
    start_time: Option<Instant>,
    sample_counter: Arc<Mutex<u64>>,
}

#[cfg(feature = "native")]
impl EngineOutput {
    pub fn new() -> Self {
        Self {
            stream: None,
            device: None,
            config: None,
            sample_rate: 44100.0,
            is_active: false,
            start_time: None,
            sample_counter: Arc::new(Mutex::new(0)),
        }
    }
    
    /// Initialize the audio output with the given sample rate
    pub fn initialize(&mut self, sample_rate: f32) -> Result<(), anyhow::Error> {
        self.sample_rate = sample_rate;
        self.setup_host_device()?;
        Ok(())
    }
    
    /// Create a stream with an Engine
    pub fn create_stream_with_engine(
        &mut self,
        engine: Arc<Mutex<Engine>>,
    ) -> Result<(), anyhow::Error> {
        let device = self.device.as_ref().ok_or_else(|| anyhow::anyhow!("Device not initialized"))?;
        let config = self.config.as_ref().ok_or_else(|| anyhow::anyhow!("Config not initialized"))?;
        
        let supported_config = device.default_output_config()?;
        let sample_counter = self.sample_counter.clone();
        
        let stream = match supported_config.sample_format() {
            cpal::SampleFormat::I8 => Self::make_stream::<i8>(device, config, engine, sample_counter)?,
            cpal::SampleFormat::I16 => Self::make_stream::<i16>(device, config, engine, sample_counter)?,
            cpal::SampleFormat::I32 => Self::make_stream::<i32>(device, config, engine, sample_counter)?,
            cpal::SampleFormat::I64 => Self::make_stream::<i64>(device, config, engine, sample_counter)?,
            cpal::SampleFormat::U8 => Self::make_stream::<u8>(device, config, engine, sample_counter)?,
            cpal::SampleFormat::U16 => Self::make_stream::<u16>(device, config, engine, sample_counter)?,
            cpal::SampleFormat::U32 => Self::make_stream::<u32>(device, config, engine, sample_counter)?,
            cpal::SampleFormat::U64 => Self::make_stream::<u64>(device, config, engine, sample_counter)?,
            cpal::SampleFormat::F32 => Self::make_stream::<f32>(device, config, engine, sample_counter)?,
            cpal::SampleFormat::F64 => Self::make_stream::<f64>(device, config, engine, sample_counter)?,
            sample_format => return Err(anyhow::anyhow!("Unsupported sample format '{}'", sample_format)),
        };
        
        self.stream = Some(stream);
        Ok(())
    }
    
    /// Setup the CPAL host and device
    fn setup_host_device(&mut self) -> Result<(), anyhow::Error> {
        let host = cpal::default_host();
        
        let device = host
            .default_output_device()
            .ok_or_else(|| anyhow::anyhow!("Default output device is not available"))?;
        
        println!("Output device: {}", device.name()?);
        
        let config = device.default_output_config()?;
        println!("Default output config: {:?}", config);
        
        self.sample_rate = config.sample_rate().0 as f32;
        self.device = Some(device);
        self.config = Some(config.into());
        
        Ok(())
    }
    
    /// Create a typed stream for the given sample format
    fn make_stream<T>(
        device: &Device,
        config: &StreamConfig,
        engine: Arc<Mutex<Engine>>,
        sample_counter: Arc<Mutex<u64>>,
    ) -> Result<Stream, anyhow::Error>
    where
        T: SizedSample + FromSample<f32>,
    {
        let num_channels = config.channels as usize;
        let sample_rate = config.sample_rate.0 as f64;
        
        let err_fn = |err| eprintln!("Error building output sound stream: {}", err);
        
        let stream = device.build_output_stream(
            config,
            move |output: &mut [T], _: &cpal::OutputCallbackInfo| {
                Self::process_frame(
                    output,
                    &engine,
                    num_channels,
                    &sample_counter,
                    sample_rate,
                );
            },
            err_fn,
            None,
        )?;
        
        Ok(stream)
    }
    
    /// Process a single frame of audio data
    fn process_frame<SampleType>(
        output: &mut [SampleType],
        engine: &Arc<Mutex<Engine>>,
        num_channels: usize,
        sample_counter: &Arc<Mutex<u64>>,
        sample_rate: f64,
    ) where
        SampleType: Sample + FromSample<f32>,
    {
        let frames_to_process = output.len() / num_channels;
        
        // Get the current sample counter and increment it atomically
        let start_sample = {
            let mut counter = sample_counter.lock().unwrap();
            let current = *counter;
            *counter += frames_to_process as u64;
            current
        };
        
        // Lock the engine once for the entire buffer
        let mut engine_guard = engine.lock().unwrap();
        
        for (frame_index, frame) in output.chunks_mut(num_channels).enumerate() {
            // Calculate precise time using sample-based timing like Web Audio
            let current_sample = start_sample + frame_index as u64;
            let current_time = current_sample as f64 / sample_rate;
            
            // Call engine.tick() to generate audio
            let value: SampleType = SampleType::from_sample(engine_guard.tick(current_time as f32));
            
            // println!("value");

            // Copy the same value to all channels
            for sample in frame.iter_mut() {
                *sample = value;
            }
        }
    }
    
    /// Start the audio stream
    pub fn start(&mut self) -> Result<(), anyhow::Error> {
        if let Some(stream) = &self.stream {
            // Reset sample counter when starting
            *self.sample_counter.lock().unwrap() = 0;
            stream.play()?;
            self.is_active = true;
            self.start_time = Some(Instant::now());
            println!("Audio stream started at sample rate: {}", self.sample_rate);
        } else {
            return Err(anyhow::anyhow!("Stream not created. Call create_stream_with_engine first."));
        }
        
        Ok(())
    }
    
    /// Stop the audio stream
    pub fn stop(&mut self) -> Result<(), anyhow::Error> {
        if let Some(stream) = &self.stream {
            stream.pause()?;
            self.is_active = false;
            println!("Audio stream stopped");
        }
        
        Ok(())
    }
    
    /// Get the current sample rate
    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
    }
    
    /// Check if the audio output is active
    pub fn is_active(&self) -> bool {
        self.is_active
    }
}

