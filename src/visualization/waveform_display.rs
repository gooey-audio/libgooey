use super::AudioBuffer;
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
}

impl WaveformDisplay {
    /// Create a new waveform display window
    pub fn new(audio_buffer: AudioBuffer, width: u32, height: u32) -> Result<Self, String> {
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

    /// Render the waveform
    fn render(&self) {
        unsafe {
            // Clear the screen
            gl::ClearColor(0.1, 0.1, 0.15, 1.0);
            gl::Clear(gl::COLOR_BUFFER_BIT);

            // Use our shader program
            gl::UseProgram(self.shader_program);

            // Get samples from the audio buffer
            let samples = self.audio_buffer.get_samples();
            if samples.is_empty() {
                return;
            }

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
            gl::LineWidth(1.0);

            // Set a different color for the center line
            let color_location =
                gl::GetUniformLocation(self.shader_program, b"color\0".as_ptr() as *const i8);
            gl::Uniform3f(color_location, 0.3, 0.3, 0.4);
            gl::DrawArrays(gl::LINES, 0, 2);

            // Reset color to white for waveform
            gl::Uniform3f(color_location, 0.2, 1.0, 0.5);

            gl::BindVertexArray(0);
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
