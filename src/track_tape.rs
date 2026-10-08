//! One to four synchronized pre-strip stereo lanes. Render owns state; workers own PCM I/O.
use crate::frame::StereoFrame;
use crate::live_control::SpscRing;
use std::cell::UnsafeCell;
use std::sync::{
    atomic::{AtomicU32, AtomicU64, Ordering},
    Arc,
};
const CAPACITY: usize = 131072;
#[derive(Clone, Copy, Default)]
pub(crate) struct TapeFrame(pub [f32; 8]);
struct FrameRing {
    samples: Box<[UnsafeCell<TapeFrame>]>,
    head: AtomicU64,
    tail: AtomicU64,
}
unsafe impl Send for FrameRing {}
unsafe impl Sync for FrameRing {}
impl FrameRing {
    fn new() -> Self {
        Self {
            samples: (0..CAPACITY)
                .map(|_| UnsafeCell::new(TapeFrame::default()))
                .collect(),
            head: AtomicU64::new(0),
            tail: AtomicU64::new(0),
        }
    }
    fn push(&self, v: TapeFrame) -> bool {
        let t = self.tail.load(Ordering::Relaxed);
        if t - self.head.load(Ordering::Acquire) >= CAPACITY as u64 {
            return false;
        }
        unsafe {
            *self.samples[t as usize % CAPACITY].get() = v;
        }
        self.tail.store(t + 1, Ordering::Release);
        true
    }
    fn pop(&self) -> Option<TapeFrame> {
        let h = self.head.load(Ordering::Relaxed);
        if h == self.tail.load(Ordering::Acquire) {
            return None;
        }
        let v = unsafe { *self.samples[h as usize % CAPACITY].get() };
        self.head.store(h + 1, Ordering::Release);
        Some(v)
    }
    // Consumer-only discard. Frames are Copy: no render-thread destructors.
    fn discard(&self) {
        self.head
            .store(self.tail.load(Ordering::Acquire), Ordering::Release);
    }
}
#[derive(Clone, Copy)]
pub(crate) enum Command {
    Arm(f64),
    ArmNextBar,
    Stop,
    Play(u64),
    Live,
}
pub(crate) struct Shared {
    pub channels: usize,
    commands: SpscRing<(u64, Command), 16>,
    capture: FrameRing,
    playback: FrameRing,
    pub state: AtomicU32,
    pub frames: AtomicU64,
    pub applied: AtomicU64,
    next: AtomicU64,
}
impl Shared {
    pub fn new(channels: usize) -> Arc<Self> {
        Arc::new(Self {
            channels,
            commands: SpscRing::new(),
            capture: FrameRing::new(),
            playback: FrameRing::new(),
            state: AtomicU32::new(0),
            frames: AtomicU64::new(0),
            applied: AtomicU64::new(0),
            next: AtomicU64::new(1),
        })
    }
    pub fn command(&self, c: Command) -> u64 {
        let g = self.next.fetch_add(1, Ordering::Relaxed);
        if self.commands.push((g, c)).is_ok() {
            g
        } else {
            0
        }
    }
    pub fn drain(&self, output: &mut [f32]) -> usize {
        let mut n = 0;
        for f in output.chunks_exact_mut(self.channels) {
            let Some(v) = self.capture.pop() else { break };
            f.copy_from_slice(&v.0[..self.channels]);
            n += 1;
        }
        n
    }
    pub fn feed(&self, input: &[f32]) -> usize {
        let mut n = 0;
        for f in input.chunks_exact(self.channels) {
            let mut frame = TapeFrame::default();
            frame.0[..self.channels].copy_from_slice(f);
            if !self.playback.push(frame) {
                break;
            }
            n += 1;
        }
        n
    }
    // Worker producer may reset capture only while render is acknowledged idle/stopped.
    pub fn discard_capture(&self) {
        self.capture.discard();
    }
}
pub(crate) struct Renderer {
    pub shared: Arc<Shared>,
    pub tracks: [usize; 4],
    mode: u32,
    start: f64,
    cursor: u64,
    total: u64,
}
impl Renderer {
    pub fn new(shared: Arc<Shared>, tracks: [usize; 4]) -> Self {
        Self {
            shared,
            tracks,
            mode: 0,
            start: 0.0,
            cursor: 0,
            total: 0,
        }
    }
    pub fn begin_buffer(&mut self, beat: f64) {
        while let Some((g, c)) = self.shared.commands.pop() {
            match c {
                Command::ArmNextBar => {
                    self.mode = 1;
                    self.start = (beat / 4.0).floor() * 4.0 + 4.0;
                    self.cursor = 0;
                    self.shared.frames.store(0, Ordering::Release);
                    self.shared.playback.discard();
                }
                Command::Arm(target) => {
                    self.mode = 1;
                    self.start = if beat > target + 1e-9 {
                        (beat / 4.0).floor() * 4.0 + 4.0
                    } else {
                        target
                    };
                    self.cursor = 0;
                    self.shared.frames.store(0, Ordering::Release);
                    self.shared.playback.discard();
                }
                Command::Stop => {
                    self.mode = if self.mode >= 4 { 5 } else { 3 };
                }
                Command::Play(total) => {
                    self.mode = 4;
                    self.total = total;
                    self.cursor = 0;
                    self.shared.frames.store(0, Ordering::Release);
                }
                Command::Live => {
                    self.mode = 0;
                    self.shared.playback.discard();
                }
            }
            self.shared.state.store(self.mode, Ordering::Release);
            self.shared.applied.store(g, Ordering::Release);
        }
    }
    pub fn frame(
        &mut self,
        beat: f64,
        running: bool,
        dry: [StereoFrame; 4],
    ) -> Option<[StereoFrame; 4]> {
        if self.mode == 1 && running && beat + 1e-9 >= self.start {
            self.mode = 2;
            self.shared.state.store(2, Ordering::Release);
        }
        if self.mode == 2 {
            if !self.shared.capture.push(TapeFrame([
                dry[0].l, dry[0].r, dry[1].l, dry[1].r, dry[2].l, dry[2].r, dry[3].l, dry[3].r,
            ])) {
                self.mode = 6;
                self.shared.state.store(6, Ordering::Release);
            } else {
                self.cursor += 1;
                self.shared.frames.store(self.cursor, Ordering::Release);
            }
        }
        if self.mode == 4 {
            if self.cursor >= self.total {
                self.mode = 5;
                self.shared.state.store(5, Ordering::Release);
            } else if let Some(v) = self.shared.playback.pop() {
                self.cursor += 1;
                self.shared.frames.store(self.cursor, Ordering::Release);
                return Some(std::array::from_fn(|i| StereoFrame {
                    l: v.0[i * 2],
                    r: v.0[i * 2 + 1],
                }));
            } else {
                self.mode = 7;
                self.shared.state.store(7, Ordering::Release);
            }
        }
        if self.mode >= 4 {
            Some([StereoFrame::default(); 4])
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn synchronized_next_bar_dry_capture() {
        let s = Shared::new(6);
        let mut r = Renderer::new(s.clone(), [0, 1, 2, 0]);
        let g = s.command(Command::Arm(4.0));
        r.begin_buffer(0.0);
        assert_eq!(s.applied.load(Ordering::Acquire), g);
        let dry = [
            StereoFrame { l: 1., r: 2. },
            StereoFrame { l: 3., r: 4. },
            StereoFrame { l: 5., r: 6. },
            StereoFrame::default(),
        ];
        r.frame(3.99, true, dry);
        assert_eq!(s.frames.load(Ordering::Acquire), 0);
        r.frame(4., true, dry);
        s.command(Command::Stop);
        r.begin_buffer(0.0);
        let mut pcm = [0.; 12];
        assert_eq!(s.drain(&mut pcm), 1);
        assert_eq!(&pcm[..6], &[1., 2., 3., 4., 5., 6.]);
    }
    #[test]
    fn replay_once_and_silence_without_feedback() {
        let s = Shared::new(6);
        let mut r = Renderer::new(s.clone(), [0, 1, 2, 0]);
        s.feed(&[1., 2., 3., 4., 5., 6.]);
        s.command(Command::Play(1));
        r.begin_buffer(0.0);
        let dry = [StereoFrame::mono(99.); 4];
        assert_eq!(r.frame(0., false, dry).unwrap()[0].l, 1.);
        assert_eq!(r.frame(0., false, dry).unwrap()[0].l, 0.);
        let mut pcm = [0.; 6];
        assert_eq!(s.drain(&mut pcm), 0);
        assert_eq!(s.state.load(Ordering::Acquire), 5);
    }
    #[test]
    fn underrun_and_capture_overflow_are_explicit() {
        let s = Shared::new(6);
        let mut r = Renderer::new(s.clone(), [0, 1, 2, 0]);
        s.command(Command::Play(1));
        r.begin_buffer(0.0);
        r.frame(0., true, [StereoFrame::default(); 4]);
        assert_eq!(s.state.load(Ordering::Acquire), 7);
        s.command(Command::Arm(0.));
        r.begin_buffer(0.0);
        for _ in 0..=CAPACITY {
            r.frame(0., true, [StereoFrame::default(); 4]);
        }
        assert_eq!(s.state.load(Ordering::Acquire), 6);
        assert_eq!(s.frames.load(Ordering::Acquire), CAPACITY as u64);
    }
    #[test]
    fn stopped_transport_never_starts_capture() {
        let s = Shared::new(6);
        let mut r = Renderer::new(s.clone(), [0, 1, 2, 0]);
        s.command(Command::Arm(0.));
        r.begin_buffer(0.0);
        r.frame(0., false, [StereoFrame::default(); 4]);
        assert_eq!(s.frames.load(Ordering::Acquire), 0);
    }
}
