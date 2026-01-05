/// Interactive envelope editor for debugging and experimentation
/// This module provides a visual editor for ADSR envelopes with draggable control points

use crate::envelope::ADSRConfig;
use gl::types::*;
use glfw::{Action, Context, Key, MouseButton, Window, WindowEvent};
use std::ffi::CString;

const VERTEX_SHADER: &str = r#"
    #version 330 core
    layout (location = 0) in vec2 aPos;
    void main() {
        gl_Position = vec4(aPos.x, aPos.y, 0.0, 1.0);
    }
"#;

const FRAGMENT_SHADER: &str = r#"
    #version 330 core
    out vec4 FragColor;
    uniform vec3 color;
    void main() {
        FragColor = vec4(color, 1.0);
    }
"#;

/// Represents a draggable control point in the envelope
#[derive(Debug, Clone, Copy)]
pub struct EnvelopePoint {
    pub time: f32,      // Time in seconds
    pub amplitude: f32, // Amplitude 0.0 to 1.0
}

/// Interactive envelope editor with OpenGL rendering
pub struct EnvelopeEditor {
    window: glfw::Window,
    glfw: glfw::Glfw,
    events: std::sync::mpsc::Receiver<(f64, glfw::WindowEvent)>,
    shader_program: GLuint,
    vao: GLuint,
    vbo: GLuint,

    // Envelope control points
    attack_point: EnvelopePoint,
    decay_point: EnvelopePoint,
    sustain_point: EnvelopePoint,
    release_point: EnvelopePoint,

    // Interaction state
    dragging_point: Option<usize>, // 0=attack, 1=decay, 2=sustain, 3=release
    mouse_pos: (f64, f64),

    // Display settings
    width: u32,
    height: u32,
    margin: f32,
}

impl EnvelopeEditor {
    /// Create a new envelope editor window
    pub fn new(width: u32, height: u32, initial_config: ADSRConfig) -> Result<Self, String> {
        // Initialize GLFW
        let mut glfw = glfw::init(glfw::FAIL_ON_ERRORS)
            .map_err(|e| format!("Failed to initialize GLFW: {}", e))?;

        // Set OpenGL version hints
        glfw.window_hint(glfw::WindowHint::ContextVersion(3, 3));
        glfw.window_hint(glfw::WindowHint::OpenGlProfile(glfw::OpenGlProfileHint::Core));

        #[cfg(target_os = "macos")]
        glfw.window_hint(glfw::WindowHint::OpenGlForwardCompat(true));

        // Create window
        let (mut window, events) = glfw
            .create_window(width, height, "Envelope Editor", glfw::WindowMode::Windowed)
            .ok_or("Failed to create GLFW window")?;

        window.make_current();
        window.set_key_polling(true);
        window.set_mouse_button_polling(true);
        window.set_cursor_pos_polling(true);
        window.set_framebuffer_size_polling(true);
        glfw.set_swap_interval(glfw::SwapInterval::Sync(1));

        // Load OpenGL function pointers
        gl::load_with(|symbol| window.get_proc_address(symbol) as *const _);

        // Create shader program
        let shader_program = unsafe {
            let vertex_shader = Self::compile_shader(VERTEX_SHADER, gl::VERTEX_SHADER)?;
            let fragment_shader = Self::compile_shader(FRAGMENT_SHADER, gl::FRAGMENT_SHADER)?;
            let program = gl::CreateProgram();
            gl::AttachShader(program, vertex_shader);
            gl::AttachShader(program, fragment_shader);
            gl::LinkProgram(program);

            // Check for linking errors
            let mut success = 0;
            gl::GetProgramiv(program, gl::LINK_STATUS, &mut success);
            if success == 0 {
                let mut len = 0;
                gl::GetProgramiv(program, gl::INFO_LOG_LENGTH, &mut len);
                let mut buffer = vec![0u8; len as usize];
                gl::GetProgramInfoLog(program, len, std::ptr::null_mut(), buffer.as_mut_ptr() as *mut GLchar);
                return Err(format!("Shader linking failed: {}", String::from_utf8_lossy(&buffer)));
            }

            gl::DeleteShader(vertex_shader);
            gl::DeleteShader(fragment_shader);
            program
        };

        // Create VAO and VBO
        let mut vao = 0;
        let mut vbo = 0;
        unsafe {
            gl::GenVertexArrays(1, &mut vao);
            gl::GenBuffers(1, &mut vbo);
            gl::BindVertexArray(vao);
            gl::BindBuffer(gl::ARRAY_BUFFER, vbo);
            gl::VertexAttribPointer(0, 2, gl::FLOAT, gl::FALSE, 2 * std::mem::size_of::<f32>() as GLsizei, std::ptr::null());
            gl::EnableVertexAttribArray(0);
        }

        // Initialize control points from config
        let attack_point = EnvelopePoint {
            time: initial_config.attack_time,
            amplitude: 1.0,
        };
        let decay_point = EnvelopePoint {
            time: initial_config.attack_time + initial_config.decay_time,
            amplitude: initial_config.sustain_level,
        };
        let sustain_point = EnvelopePoint {
            time: initial_config.attack_time + initial_config.decay_time + 0.5, // Visual sustain duration
            amplitude: initial_config.sustain_level,
        };
        let release_point = EnvelopePoint {
            time: initial_config.attack_time + initial_config.decay_time + 0.5 + initial_config.release_time,
            amplitude: 0.0,
        };

        Ok(Self {
            window,
            glfw,
            events,
            shader_program,
            vao,
            vbo,
            attack_point,
            decay_point,
            sustain_point,
            release_point,
            dragging_point: None,
            mouse_pos: (0.0, 0.0),
            width,
            height,
            margin: 0.1,
        })
    }

    /// Compile a shader
    unsafe fn compile_shader(source: &str, shader_type: GLenum) -> Result<GLuint, String> {
        let shader = gl::CreateShader(shader_type);
        let c_str = CString::new(source.as_bytes()).unwrap();
        gl::ShaderSource(shader, 1, &c_str.as_ptr(), std::ptr::null());
        gl::CompileShader(shader);

        // Check for compilation errors
        let mut success = 0;
        gl::GetShaderiv(shader, gl::COMPILE_STATUS, &mut success);
        if success == 0 {
            let mut len = 0;
            gl::GetShaderiv(shader, gl::INFO_LOG_LENGTH, &mut len);
            let mut buffer = vec![0u8; len as usize];
            gl::GetShaderInfoLog(shader, len, std::ptr::null_mut(), buffer.as_mut_ptr() as *mut GLchar);
            return Err(format!("Shader compilation failed: {}", String::from_utf8_lossy(&buffer)));
        }

        Ok(shader)
    }

    /// Process window events
    pub fn process_events(&mut self) {
        self.glfw.poll_events();
        for (_, event) in glfw::flush_messages(&self.events) {
            match event {
                WindowEvent::Key(Key::Escape, _, Action::Press, _) => {
                    self.window.set_should_close(true);
                }
                WindowEvent::MouseButton(MouseButton::Button1, Action::Press, _) => {
                    self.handle_mouse_press();
                }
                WindowEvent::MouseButton(MouseButton::Button1, Action::Release, _) => {
                    self.dragging_point = None;
                }
                WindowEvent::CursorPos(x, y) => {
                    self.mouse_pos = (x, y);
                    if self.dragging_point.is_some() {
                        self.handle_mouse_drag();
                    }
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
    }

    /// Handle mouse press to start dragging a control point
    fn handle_mouse_press(&mut self) {
        let (mx, my) = self.screen_to_normalized(self.mouse_pos.0, self.mouse_pos.1);
        let threshold = 0.05;

        // Check each control point
        let points = [
            (self.attack_point, 0),
            (self.decay_point, 1),
            (self.sustain_point, 2),
            (self.release_point, 3),
        ];

        for (point, idx) in points.iter() {
            let (px, py) = self.point_to_normalized(point.time, point.amplitude);
            let dist = ((mx - px).powi(2) + (my - py).powi(2)).sqrt();
            if dist < threshold {
                self.dragging_point = Some(*idx);
                return;
            }
        }
    }

    /// Handle mouse drag to move control points
    fn handle_mouse_drag(&mut self) {
        if let Some(idx) = self.dragging_point {
            let (mx, my) = self.screen_to_normalized(self.mouse_pos.0, self.mouse_pos.1);
            let (time, amplitude) = self.normalized_to_point(mx, my);

            match idx {
                0 => {
                    // Attack point - only vertical movement
                    self.attack_point.time = time.max(0.001).min(2.0);
                    self.attack_point.amplitude = 1.0; // Always at peak
                }
                1 => {
                    // Decay point - adjust both time and sustain level
                    self.decay_point.time = time.max(self.attack_point.time + 0.001).min(self.sustain_point.time - 0.001);
                    self.decay_point.amplitude = amplitude.clamp(0.0, 1.0);
                    self.sustain_point.amplitude = self.decay_point.amplitude;
                }
                2 => {
                    // Sustain point - horizontal movement (duration)
                    self.sustain_point.time = time.max(self.decay_point.time + 0.001).min(self.release_point.time - 0.001);
                }
                3 => {
                    // Release point - only horizontal movement
                    self.release_point.time = time.max(self.sustain_point.time + 0.001).min(5.0);
                }
                _ => {}
            }
        }
    }

    /// Convert screen coordinates to normalized coordinates (-1 to 1)
    fn screen_to_normalized(&self, x: f64, y: f64) -> (f32, f32) {
        let nx = (2.0 * x / self.width as f64 - 1.0) as f32;
        let ny = (1.0 - 2.0 * y / self.height as f64) as f32;
        (nx, ny)
    }

    /// Convert envelope point (time, amplitude) to normalized coordinates
    fn point_to_normalized(&self, time: f32, amplitude: f32) -> (f32, f32) {
        let max_time = self.release_point.time * 1.2;
        let x = -1.0 + 2.0 * self.margin + (time / max_time) * (2.0 - 4.0 * self.margin);
        let y = -1.0 + 2.0 * self.margin + amplitude * (2.0 - 4.0 * self.margin);
        (x, y)
    }

    /// Convert normalized coordinates to envelope point (time, amplitude)
    fn normalized_to_point(&self, x: f32, y: f32) -> (f32, f32) {
        let max_time = self.release_point.time * 1.2;
        let time = ((x + 1.0 - 2.0 * self.margin) / (2.0 - 4.0 * self.margin)) * max_time;
        let amplitude = (y + 1.0 - 2.0 * self.margin) / (2.0 - 4.0 * self.margin);
        (time, amplitude)
    }

    /// Render the envelope
    pub fn render(&mut self) {
        unsafe {
            gl::ClearColor(0.1, 0.1, 0.12, 1.0);
            gl::Clear(gl::COLOR_BUFFER_BIT);

            gl::UseProgram(self.shader_program);
            gl::BindVertexArray(self.vao);

            // Draw envelope curve
            self.draw_envelope_curve();

            // Draw control points
            self.draw_control_points();

            // Draw grid
            self.draw_grid();
        }

        self.window.swap_buffers();
    }

    /// Draw the envelope curve
    fn draw_envelope_curve(&self) {
        let mut vertices = Vec::new();

        // Generate curve points
        let steps = 200;
        for i in 0..=steps {
            let progress = i as f32 / steps as f32;
            let time = progress * self.release_point.time * 1.2;
            let amplitude = self.get_amplitude_at_time(time);
            let (x, y) = self.point_to_normalized(time, amplitude);
            vertices.push(x);
            vertices.push(y);
        }

        unsafe {
            // Set color for envelope curve
            let color_loc = gl::GetUniformLocation(self.shader_program, b"color\0".as_ptr() as *const i8);
            gl::Uniform3f(color_loc, 0.3, 0.8, 0.4);

            gl::BindBuffer(gl::ARRAY_BUFFER, self.vbo);
            gl::BufferData(
                gl::ARRAY_BUFFER,
                (vertices.len() * std::mem::size_of::<f32>()) as GLsizeiptr,
                vertices.as_ptr() as *const _,
                gl::DYNAMIC_DRAW,
            );
            gl::LineWidth(2.0);
            gl::DrawArrays(gl::LINE_STRIP, 0, vertices.len() as i32 / 2);
        }
    }

    /// Get amplitude at a specific time in the envelope
    fn get_amplitude_at_time(&self, time: f32) -> f32 {
        if time < self.attack_point.time {
            // Attack phase
            time / self.attack_point.time
        } else if time < self.decay_point.time {
            // Decay phase
            let decay_progress = (time - self.attack_point.time) / (self.decay_point.time - self.attack_point.time);
            1.0 - (1.0 - self.decay_point.amplitude) * decay_progress
        } else if time < self.sustain_point.time {
            // Sustain phase
            self.sustain_point.amplitude
        } else if time < self.release_point.time {
            // Release phase
            let release_progress = (time - self.sustain_point.time) / (self.release_point.time - self.sustain_point.time);
            self.sustain_point.amplitude * (1.0 - release_progress)
        } else {
            0.0
        }
    }

    /// Draw control points
    fn draw_control_points(&self) {
        let points = [
            self.attack_point,
            self.decay_point,
            self.sustain_point,
            self.release_point,
        ];

        for (i, point) in points.iter().enumerate() {
            let (x, y) = self.point_to_normalized(point.time, point.amplitude);
            let is_dragging = self.dragging_point == Some(i);
            self.draw_circle(x, y, 0.02, is_dragging);
        }
    }

    /// Draw a circle at the specified position
    fn draw_circle(&self, x: f32, y: f32, radius: f32, highlighted: bool) {
        let segments = 20;
        let mut vertices = Vec::new();

        for i in 0..=segments {
            let angle = 2.0 * std::f32::consts::PI * i as f32 / segments as f32;
            vertices.push(x + radius * angle.cos());
            vertices.push(y + radius * angle.sin());
        }

        unsafe {
            let color_loc = gl::GetUniformLocation(self.shader_program, b"color\0".as_ptr() as *const i8);
            if highlighted {
                gl::Uniform3f(color_loc, 1.0, 0.8, 0.2);
            } else {
                gl::Uniform3f(color_loc, 0.9, 0.9, 0.95);
            }

            gl::BindBuffer(gl::ARRAY_BUFFER, self.vbo);
            gl::BufferData(
                gl::ARRAY_BUFFER,
                (vertices.len() * std::mem::size_of::<f32>()) as GLsizeiptr,
                vertices.as_ptr() as *const _,
                gl::DYNAMIC_DRAW,
            );
            gl::DrawArrays(gl::TRIANGLE_FAN, 0, vertices.len() as i32 / 2);
        }
    }

    /// Draw background grid
    fn draw_grid(&self) {
        let mut vertices = Vec::new();

        // Vertical grid lines (time)
        for i in 0..=10 {
            let x = -1.0 + 2.0 * self.margin + (i as f32 / 10.0) * (2.0 - 4.0 * self.margin);
            vertices.push(x);
            vertices.push(-1.0 + 2.0 * self.margin);
            vertices.push(x);
            vertices.push(1.0 - 2.0 * self.margin);
        }

        // Horizontal grid lines (amplitude)
        for i in 0..=10 {
            let y = -1.0 + 2.0 * self.margin + (i as f32 / 10.0) * (2.0 - 4.0 * self.margin);
            vertices.push(-1.0 + 2.0 * self.margin);
            vertices.push(y);
            vertices.push(1.0 - 2.0 * self.margin);
            vertices.push(y);
        }

        unsafe {
            let color_loc = gl::GetUniformLocation(self.shader_program, b"color\0".as_ptr() as *const i8);
            gl::Uniform3f(color_loc, 0.2, 0.2, 0.25);

            gl::BindBuffer(gl::ARRAY_BUFFER, self.vbo);
            gl::BufferData(
                gl::ARRAY_BUFFER,
                (vertices.len() * std::mem::size_of::<f32>()) as GLsizeiptr,
                vertices.as_ptr() as *const _,
                gl::DYNAMIC_DRAW,
            );
            gl::LineWidth(1.0);
            gl::DrawArrays(gl::LINES, 0, vertices.len() as i32 / 2);
        }
    }

    /// Check if the window should close
    pub fn should_close(&self) -> bool {
        self.window.should_close()
    }

    /// Get the current ADSR configuration from the editor
    pub fn get_config(&self) -> ADSRConfig {
        ADSRConfig::new(
            self.attack_point.time,
            self.decay_point.time - self.attack_point.time,
            self.sustain_point.amplitude,
            self.release_point.time - self.sustain_point.time,
        )
    }

    /// Update the editor with a new configuration
    pub fn set_config(&mut self, config: ADSRConfig) {
        self.attack_point.time = config.attack_time;
        self.attack_point.amplitude = 1.0;

        self.decay_point.time = config.attack_time + config.decay_time;
        self.decay_point.amplitude = config.sustain_level;

        self.sustain_point.time = config.attack_time + config.decay_time + 0.5;
        self.sustain_point.amplitude = config.sustain_level;

        self.release_point.time = config.attack_time + config.decay_time + 0.5 + config.release_time;
        self.release_point.amplitude = 0.0;
    }
}

impl Drop for EnvelopeEditor {
    fn drop(&mut self) {
        unsafe {
            gl::DeleteProgram(self.shader_program);
            gl::DeleteVertexArrays(1, &self.vao);
            gl::DeleteBuffers(1, &self.vbo);
        }
    }
}
