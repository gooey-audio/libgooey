use super::{AudioBuffer, SpectrogramAnalyzer};
use glfw::{Action, Context, GlfwReceiver, Key, WindowEvent};
use std::ffi::CString;

/// Events that can be emitted by the waveform display
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayEvent {
    SpacePressed,
    Quit,
}

/// Waveform display using OpenGL
pub struct WaveformDisplay {
    glfw: glfw::Glfw,
    window: glfw::PWindow,
    events: GlfwReceiver<(f64, WindowEvent)>,
    audio_buffer: AudioBuffer,
    width: u32,
    height: u32,
    shader_program: u32,
    vao: u32,
    vbo: u32,
    spectrogram: SpectrogramAnalyzer,
    sample_rate: f32,
}

impl WaveformDisplay {
    /// Create a new waveform display window
    pub fn new(
        audio_buffer: AudioBuffer,
        width: u32,
        height: u32,
        sample_rate: f32,
    ) -> Result<Self, String> {
        // Initialize GLFW
        let mut glfw = glfw::init(glfw::fail_on_errors)
            .map_err(|e| format!("Failed to initialize GLFW: {:?}", e))?;

        // Request OpenGL 3.3 Core Profile
        glfw.window_hint(glfw::WindowHint::ContextVersion(3, 3));
        glfw.window_hint(glfw::WindowHint::OpenGlProfile(
            glfw::OpenGlProfileHint::Core,
        ));
        glfw.window_hint(glfw::WindowHint::OpenGlForwardCompat(true));

        // Create a windowed mode window and its OpenGL context
        let (mut window, events) = glfw
            .create_window(
                width,
                height,
                "Waveform Display",
                glfw::WindowMode::Windowed,
            )
            .ok_or_else(|| "Failed to create GLFW window".to_string())?;

        // Make the window's context current
        window.make_current();
        window.set_key_polling(true);
        window.set_framebuffer_size_polling(true);

        // Load OpenGL function pointers
        gl::load_with(|symbol| window.get_proc_address(symbol) as *const _);

        // Create shader program and buffers
        let shader_program = unsafe { create_shader_program()? };
        let (vao, vbo) = unsafe { create_buffers()? };

        // Create spectrogram analyzer (512-point FFT, 200 history frames)
        let spectrogram = SpectrogramAnalyzer::new(512, sample_rate, 200);

        Ok(Self {
            glfw,
            window,
            events,
            audio_buffer,
            width,
            height,
            shader_program,
            vao,
            vbo,
            spectrogram,
            sample_rate,
        })
    }

    /// Check if the window should close
    pub fn should_close(&self) -> bool {
        self.window.should_close()
    }

    /// Process events and render the waveform, returns list of events
    pub fn update(&mut self) -> Vec<DisplayEvent> {
        let mut display_events = Vec::new();

        // Process events
        self.glfw.poll_events();
        for (_, event) in glfw::flush_messages(&self.events) {
            match event {
                WindowEvent::Key(Key::Escape, _, Action::Press, _) => {
                    self.window.set_should_close(true);
                    display_events.push(DisplayEvent::Quit);
                }
                WindowEvent::Key(Key::Space, _, Action::Press, _) => {
                    display_events.push(DisplayEvent::SpacePressed);
                }
                WindowEvent::FramebufferSize(width, height) => {
                    self.width = width as u32;
                    self.height = height as u32;
                    unsafe {
                        gl::Viewport(0, 0, width, height);
                    }
                }
                _ => {}
            }
        }

        // Render the waveform
        self.render();

        // Swap front and back buffers
        self.window.swap_buffers();

        display_events
    }

    /// Render the waveform and spectrogram
    fn render(&mut self) {
        unsafe {
            // Clear the screen
            gl::ClearColor(0.05, 0.05, 0.1, 1.0);
            gl::Clear(gl::COLOR_BUFFER_BIT);

            // Use our shader program
            gl::UseProgram(self.shader_program);

            // Get samples from the audio buffer
            let samples = self.audio_buffer.get_samples();
            if samples.is_empty() {
                return;
            }

            // Analyze samples for spectrogram
            self.spectrogram.analyze(&samples);

            // Get color uniform location
            let color_location =
                gl::GetUniformLocation(self.shader_program, b"color\0".as_ptr() as *const i8);

            // ====== TOP PANEL: WAVEFORM ======
            // Set viewport for top half
            gl::Viewport(
                0,
                (self.height / 2) as i32,
                self.width as i32,
                (self.height / 2) as i32,
            );

            // Convert samples to vertices (normalized coordinates)
            let mut vertices: Vec<f32> = Vec::with_capacity(samples.len() * 2);
            let num_samples = samples.len();

            for (i, &sample) in samples.iter().enumerate() {
                let x = (i as f32 / num_samples as f32) * 2.0 - 1.0; // Map to -1.0 to 1.0
                let y = sample.clamp(-1.0, 1.0); // Clamp to valid range
                vertices.push(x);
                vertices.push(y);
            }

            // Update VBO with new vertices
            gl::BindVertexArray(self.vao);
            gl::BindBuffer(gl::ARRAY_BUFFER, self.vbo);
            gl::BufferData(
                gl::ARRAY_BUFFER,
                (vertices.len() * std::mem::size_of::<f32>()) as isize,
                vertices.as_ptr() as *const _,
                gl::DYNAMIC_DRAW,
            );

            // Draw the waveform as a line strip
            gl::Uniform3f(color_location, 0.2, 1.0, 0.5);
            gl::LineWidth(2.0);
            gl::DrawArrays(gl::LINE_STRIP, 0, (vertices.len() / 2) as i32);

            // Draw center line (y=0)
            let center_line: [f32; 4] = [-1.0, 0.0, 1.0, 0.0];
            gl::BufferData(
                gl::ARRAY_BUFFER,
                (center_line.len() * std::mem::size_of::<f32>()) as isize,
                center_line.as_ptr() as *const _,
                gl::DYNAMIC_DRAW,
            );
            gl::Uniform3f(color_location, 0.3, 0.3, 0.4);
            gl::LineWidth(1.0);
            gl::DrawArrays(gl::LINES, 0, 2);

            // ====== BOTTOM PANEL: SPECTROGRAM ======
            // Set viewport for bottom half
            gl::Viewport(0, 0, self.width as i32, (self.height / 2) as i32);

            // Draw spectrogram as rectangles
            self.render_spectrogram(color_location);

            // Reset viewport
            gl::Viewport(0, 0, self.width as i32, self.height as i32);
            gl::BindVertexArray(0);
        }
    }

    /// Render the spectrogram
    unsafe fn render_spectrogram(&self, color_location: i32) {
        let history = self.spectrogram.get_history();
        if history.is_empty() {
            return;
        }

        let num_time_steps = history.len();
        let num_freq_bins = self.spectrogram.num_bins();

        // Draw each time step as a vertical strip
        for (time_idx, spectrum) in history.iter().enumerate() {
            let x_start = (time_idx as f32 / num_time_steps as f32) * 2.0 - 1.0;
            let x_end = ((time_idx + 1) as f32 / num_time_steps as f32) * 2.0 - 1.0;

            // Draw frequency bins (only show up to 10 kHz for visibility)
            let max_freq_idx = (10000.0 / (self.sample_rate / 2.0) * num_freq_bins as f32) as usize;
            let display_bins = max_freq_idx.min(num_freq_bins);

            for freq_idx in 0..display_bins {
                let y_start = (freq_idx as f32 / display_bins as f32) * 2.0 - 1.0;
                let y_end = ((freq_idx + 1) as f32 / display_bins as f32) * 2.0 - 1.0;

                // Get magnitude in dB and normalize to 0-1 range (assuming -80 dB to 0 dB)
                let mag_db = spectrum[freq_idx];
                let normalized = ((mag_db + 80.0) / 80.0).clamp(0.0, 1.0);

                // Map to color (blue-green-yellow-red heat map)
                let (r, g, b) = magnitude_to_color(normalized);
                gl::Uniform3f(color_location, r, g, b);

                // Draw rectangle for this frequency bin at this time
                let quad: [f32; 8] = [
                    x_start, y_start, x_end, y_start, x_end, y_end, x_start, y_end,
                ];

                gl::BindBuffer(gl::ARRAY_BUFFER, self.vbo);
                gl::BufferData(
                    gl::ARRAY_BUFFER,
                    (quad.len() * std::mem::size_of::<f32>()) as isize,
                    quad.as_ptr() as *const _,
                    gl::DYNAMIC_DRAW,
                );

                gl::DrawArrays(gl::TRIANGLE_FAN, 0, 4);
            }
        }
    }

    /// Get a reference to the GLFW instance
    pub fn glfw(&self) -> &glfw::Glfw {
        &self.glfw
    }
}

impl Drop for WaveformDisplay {
    fn drop(&mut self) {
        unsafe {
            gl::DeleteProgram(self.shader_program);
            gl::DeleteBuffers(1, &self.vbo);
            gl::DeleteVertexArrays(1, &self.vao);
        }
    }
}

/// Create shader program
unsafe fn create_shader_program() -> Result<u32, String> {
    // Vertex shader source
    let vertex_shader_source = CString::new(
        r#"
        #version 330 core
        layout (location = 0) in vec2 aPos;
        
        void main() {
            gl_Position = vec4(aPos.x, aPos.y, 0.0, 1.0);
        }
        "#,
    )
    .unwrap();

    // Fragment shader source
    let fragment_shader_source = CString::new(
        r#"
        #version 330 core
        out vec4 FragColor;
        uniform vec3 color;
        
        void main() {
            FragColor = vec4(color, 1.0);
        }
        "#,
    )
    .unwrap();

    // Compile vertex shader
    let vertex_shader = gl::CreateShader(gl::VERTEX_SHADER);
    gl::ShaderSource(
        vertex_shader,
        1,
        &vertex_shader_source.as_ptr(),
        std::ptr::null(),
    );
    gl::CompileShader(vertex_shader);
    check_shader_compile_errors(vertex_shader, "VERTEX")?;

    // Compile fragment shader
    let fragment_shader = gl::CreateShader(gl::FRAGMENT_SHADER);
    gl::ShaderSource(
        fragment_shader,
        1,
        &fragment_shader_source.as_ptr(),
        std::ptr::null(),
    );
    gl::CompileShader(fragment_shader);
    check_shader_compile_errors(fragment_shader, "FRAGMENT")?;

    // Link shaders into a program
    let shader_program = gl::CreateProgram();
    gl::AttachShader(shader_program, vertex_shader);
    gl::AttachShader(shader_program, fragment_shader);
    gl::LinkProgram(shader_program);
    check_program_link_errors(shader_program)?;

    // Delete shaders (they're linked into the program now)
    gl::DeleteShader(vertex_shader);
    gl::DeleteShader(fragment_shader);

    // Set default color
    gl::UseProgram(shader_program);
    let color_location = gl::GetUniformLocation(shader_program, b"color\0".as_ptr() as *const i8);
    gl::Uniform3f(color_location, 0.2, 1.0, 0.5); // Green-ish color

    Ok(shader_program)
}

/// Create VAO and VBO
unsafe fn create_buffers() -> Result<(u32, u32), String> {
    let mut vao = 0;
    let mut vbo = 0;

    gl::GenVertexArrays(1, &mut vao);
    gl::GenBuffers(1, &mut vbo);

    gl::BindVertexArray(vao);
    gl::BindBuffer(gl::ARRAY_BUFFER, vbo);

    // Configure vertex attributes
    gl::VertexAttribPointer(
        0,
        2,
        gl::FLOAT,
        gl::FALSE,
        2 * std::mem::size_of::<f32>() as i32,
        std::ptr::null(),
    );
    gl::EnableVertexAttribArray(0);

    gl::BindBuffer(gl::ARRAY_BUFFER, 0);
    gl::BindVertexArray(0);

    Ok((vao, vbo))
}

/// Check shader compilation errors
unsafe fn check_shader_compile_errors(shader: u32, shader_type: &str) -> Result<(), String> {
    let mut success = 0;
    gl::GetShaderiv(shader, gl::COMPILE_STATUS, &mut success);

    if success == 0 {
        let mut log_length = 0;
        gl::GetShaderiv(shader, gl::INFO_LOG_LENGTH, &mut log_length);

        let mut log = vec![0u8; log_length as usize];
        gl::GetShaderInfoLog(
            shader,
            log_length,
            std::ptr::null_mut(),
            log.as_mut_ptr() as *mut i8,
        );

        return Err(format!(
            "Shader compilation error ({}): {}",
            shader_type,
            String::from_utf8_lossy(&log)
        ));
    }

    Ok(())
}

/// Check program linking errors
unsafe fn check_program_link_errors(program: u32) -> Result<(), String> {
    let mut success = 0;
    gl::GetProgramiv(program, gl::LINK_STATUS, &mut success);

    if success == 0 {
        let mut log_length = 0;
        gl::GetProgramiv(program, gl::INFO_LOG_LENGTH, &mut log_length);

        let mut log = vec![0u8; log_length as usize];
        gl::GetProgramInfoLog(
            program,
            log_length,
            std::ptr::null_mut(),
            log.as_mut_ptr() as *mut i8,
        );

        return Err(format!(
            "Program linking error: {}",
            String::from_utf8_lossy(&log)
        ));
    }

    Ok(())
}

/// Convert magnitude (0.0 to 1.0) to heat map color (RGB)
/// 0.0 = dark blue (silent)
/// 0.5 = green/yellow (moderate)
/// 1.0 = red (loud)
fn magnitude_to_color(magnitude: f32) -> (f32, f32, f32) {
    if magnitude < 0.25 {
        // Dark blue to blue
        let t = magnitude / 0.25;
        (0.0, 0.0, 0.2 + t * 0.6)
    } else if magnitude < 0.5 {
        // Blue to cyan
        let t = (magnitude - 0.25) / 0.25;
        (0.0, t * 0.8, 0.8)
    } else if magnitude < 0.75 {
        // Cyan to yellow
        let t = (magnitude - 0.5) / 0.25;
        (t, 0.8, 0.8 * (1.0 - t))
    } else {
        // Yellow to red
        let t = (magnitude - 0.75) / 0.25;
        (1.0, 0.8 * (1.0 - t), 0.0)
    }
}
