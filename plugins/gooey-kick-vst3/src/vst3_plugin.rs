#![allow(non_snake_case)]

use crate::editor::{EditorHost, KickEditor, EDITOR_HEIGHT, EDITOR_WIDTH};
use crate::params::{AtomicParameters, ParamId, Parameters, STATE_SIZE};
use crate::KickAdapter;
use egui_baseview::baseview::dpi::LogicalSize;
use egui_baseview::{EguiWindow, EguiWindowSettings, RepaintNotifier};
use raw_window_handle::{
    AppKitWindowHandle, HandleError, HasWindowHandle, RawWindowHandle, WindowHandle,
};
use std::cell::{RefCell, UnsafeCell};
use std::ffi::{c_char, c_void, CStr};
use std::ptr::{self, NonNull};
use std::slice;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use vst3::{uid, Class, ComPtr, ComRef, ComWrapper, Steinberg::Vst::*, Steinberg::*};

const PLUGIN_NAME: &str = "Gooey Kick POC";
const VENDOR: &str = "Gooey Audio";
const VERSION: &str = "0.1.0";

pub const PROCESSOR_CID: TUID = uid(0x7D29F218, 0x35C84BDF, 0xA7E7D903, 0xB4896721);
pub const CONTROLLER_CID: TUID = uid(0xD9F6B809, 0x64F24367, 0x98CC1A27, 0x1CD103B2);

fn copy_cstring(src: &str, dst: &mut [c_char]) {
    dst.fill(0);
    let capacity = dst.len().saturating_sub(1);
    for (source, target) in src.bytes().zip(dst.iter_mut().take(capacity)) {
        *target = source as c_char;
    }
}

fn copy_wstring(src: &str, dst: &mut [TChar]) {
    dst.fill(0);
    let capacity = dst.len().saturating_sub(1);
    for (source, target) in src.encode_utf16().zip(dst.iter_mut().take(capacity)) {
        *target = source;
    }
}

unsafe fn platform_is(candidate: FIDString, expected: FIDString) -> bool {
    !candidate.is_null()
        && !expected.is_null()
        && CStr::from_ptr(candidate) == CStr::from_ptr(expected)
}

unsafe fn stream_read(stream: *mut IBStream) -> Option<[u8; STATE_SIZE]> {
    let stream = ComRef::from_raw(stream)?;
    let mut bytes = [0_u8; STATE_SIZE];
    let mut cursor = 0;
    while cursor < bytes.len() {
        let mut read = 0;
        let result = stream.read(
            bytes[cursor..].as_mut_ptr().cast(),
            (bytes.len() - cursor) as i32,
            &mut read,
        );
        if result != kResultOk || read <= 0 {
            return None;
        }
        cursor += read as usize;
    }
    Some(bytes)
}

unsafe fn stream_write(stream: *mut IBStream, bytes: &[u8]) -> tresult {
    let Some(stream) = ComRef::from_raw(stream) else {
        return kInvalidArgument;
    };
    let mut cursor = 0;
    while cursor < bytes.len() {
        let mut written = 0;
        let result = stream.write(
            bytes[cursor..].as_ptr().cast_mut().cast(),
            (bytes.len() - cursor) as i32,
            &mut written,
        );
        if result != kResultOk || written <= 0 {
            return kResultFalse;
        }
        cursor += written as usize;
    }
    kResultOk
}

struct Processor {
    audio: UnsafeCell<KickAdapter>,
    parameters: Arc<AtomicParameters>,
    initialized: AtomicBool,
    sample_rate_bits: std::sync::atomic::AtomicU64,
}

// VST3 calls setup/state methods outside the real-time process call and does not
// concurrently enter `process` for one component. UnsafeCell avoids a mutex in
// that callback; the atomic mirror is the only state read from other threads.
unsafe impl Sync for Processor {}

impl Processor {
    fn new() -> Self {
        let parameters = Parameters::default();
        Self {
            audio: UnsafeCell::new(KickAdapter::new(44_100.0, parameters)),
            parameters: Arc::new(AtomicParameters::new(parameters)),
            initialized: AtomicBool::new(false),
            sample_rate_bits: std::sync::atomic::AtomicU64::new(44_100.0_f64.to_bits()),
        }
    }

    fn audio_ptr(&self) -> *mut KickAdapter {
        self.audio.get()
    }

    unsafe fn apply_parameter_changes(&self, changes: *mut IParameterChanges, offset: i32) {
        let Some(changes) = ComRef::from_raw(changes) else {
            return;
        };
        for queue_index in 0..changes.getParameterCount() {
            let Some(queue) = ComRef::from_raw(changes.getParameterData(queue_index)) else {
                continue;
            };
            let Some(id) = ParamId::from_raw(queue.getParameterId()) else {
                continue;
            };
            for point_index in 0..queue.getPointCount() {
                let mut point_offset = 0;
                let mut value = 0.0;
                if queue.getPoint(point_index, &mut point_offset, &mut value) == kResultTrue
                    && point_offset == offset
                {
                    let value = value as f32;
                    (*self.audio_ptr()).set_parameter(id, value);
                    self.parameters.set(id, value);
                }
            }
        }
    }

    unsafe fn flush_parameter_changes(&self, changes: *mut IParameterChanges) {
        let Some(changes) = ComRef::from_raw(changes) else {
            return;
        };
        for queue_index in 0..changes.getParameterCount() {
            let Some(queue) = ComRef::from_raw(changes.getParameterData(queue_index)) else {
                continue;
            };
            let Some(id) = ParamId::from_raw(queue.getParameterId()) else {
                continue;
            };
            let point_count = queue.getPointCount();
            if point_count <= 0 {
                continue;
            }
            let mut offset = 0;
            let mut value = 0.0;
            if queue.getPoint(point_count - 1, &mut offset, &mut value) == kResultTrue {
                (*self.audio_ptr()).set_parameter(id, value as f32);
                self.parameters.set(id, value as f32);
            }
        }
    }

    unsafe fn apply_note_events(&self, events: *mut IEventList, offset: i32) {
        let Some(events) = ComRef::from_raw(events) else {
            return;
        };
        for event_index in 0..events.getEventCount() {
            let mut event: Event = std::mem::zeroed();
            if events.getEvent(event_index, &mut event) == kResultOk
                && event.sampleOffset == offset
                && event.r#type == Event_::EventTypes_::kNoteOnEvent as u16
            {
                (*self.audio_ptr()).trigger(event.__field0.noteOn.velocity);
            }
        }
    }
}

impl Class for Processor {
    type Interfaces = (IComponent, IAudioProcessor);
}

impl IPluginBaseTrait for Processor {
    unsafe fn initialize(&self, _context: *mut FUnknown) -> tresult {
        if self.initialized.swap(true, Ordering::AcqRel) {
            kResultFalse
        } else {
            kResultOk
        }
    }

    unsafe fn terminate(&self) -> tresult {
        self.initialized.store(false, Ordering::Release);
        kResultOk
    }
}

impl IComponentTrait for Processor {
    unsafe fn getControllerClassId(&self, class_id: *mut TUID) -> tresult {
        let Some(class_id) = class_id.as_mut() else {
            return kInvalidArgument;
        };
        *class_id = CONTROLLER_CID;
        kResultOk
    }

    unsafe fn setIoMode(&self, _mode: IoMode) -> tresult {
        kResultOk
    }

    unsafe fn getBusCount(&self, media_type: MediaType, direction: BusDirection) -> i32 {
        match (media_type, direction) {
            (x, y) if x == MediaTypes_::kAudio as i32 && y == BusDirections_::kOutput as i32 => 1,
            (x, y) if x == MediaTypes_::kEvent as i32 && y == BusDirections_::kInput as i32 => 1,
            _ => 0,
        }
    }

    unsafe fn getBusInfo(
        &self,
        media_type: MediaType,
        direction: BusDirection,
        index: i32,
        bus: *mut BusInfo,
    ) -> tresult {
        let Some(bus) = bus.as_mut() else {
            return kInvalidArgument;
        };
        if index != 0 {
            return kInvalidArgument;
        }
        match (media_type, direction) {
            (x, y) if x == MediaTypes_::kAudio as i32 && y == BusDirections_::kOutput as i32 => {
                bus.mediaType = MediaTypes_::kAudio as i32;
                bus.direction = BusDirections_::kOutput as i32;
                bus.channelCount = 2;
                copy_wstring("Stereo Output", &mut bus.name);
                bus.busType = BusTypes_::kMain as i32;
                bus.flags = BusInfo_::BusFlags_::kDefaultActive;
                kResultOk
            }
            (x, y) if x == MediaTypes_::kEvent as i32 && y == BusDirections_::kInput as i32 => {
                bus.mediaType = MediaTypes_::kEvent as i32;
                bus.direction = BusDirections_::kInput as i32;
                bus.channelCount = 16;
                copy_wstring("MIDI In", &mut bus.name);
                bus.busType = BusTypes_::kMain as i32;
                bus.flags = BusInfo_::BusFlags_::kDefaultActive;
                kResultOk
            }
            _ => kInvalidArgument,
        }
    }

    unsafe fn getRoutingInfo(
        &self,
        _input: *mut RoutingInfo,
        _output: *mut RoutingInfo,
    ) -> tresult {
        kNotImplemented
    }

    unsafe fn activateBus(
        &self,
        media_type: MediaType,
        direction: BusDirection,
        index: i32,
        _state: TBool,
    ) -> tresult {
        if index == 0 && self.getBusCount(media_type, direction) == 1 {
            kResultOk
        } else {
            kInvalidArgument
        }
    }

    unsafe fn setActive(&self, _state: TBool) -> tresult {
        kResultOk
    }

    unsafe fn setState(&self, state: *mut IBStream) -> tresult {
        let Some(bytes) = stream_read(state) else {
            return kResultFalse;
        };
        let Ok(parameters) = Parameters::decode(&bytes) else {
            return kResultFalse;
        };
        (*self.audio_ptr()).replace_parameters(parameters);
        self.parameters.replace(parameters);
        kResultOk
    }

    unsafe fn getState(&self, state: *mut IBStream) -> tresult {
        stream_write(state, &self.parameters.load().encode())
    }
}

impl IAudioProcessorTrait for Processor {
    unsafe fn setBusArrangements(
        &self,
        _inputs: *mut SpeakerArrangement,
        num_ins: i32,
        outputs: *mut SpeakerArrangement,
        num_outs: i32,
    ) -> tresult {
        if num_ins == 0 && num_outs == 1 && !outputs.is_null() && *outputs == SpeakerArr::kStereo {
            kResultTrue
        } else {
            kResultFalse
        }
    }

    unsafe fn getBusArrangement(
        &self,
        direction: BusDirection,
        index: i32,
        arrangement: *mut SpeakerArrangement,
    ) -> tresult {
        if direction == BusDirections_::kOutput as i32 && index == 0 {
            let Some(arrangement) = arrangement.as_mut() else {
                return kInvalidArgument;
            };
            *arrangement = SpeakerArr::kStereo;
            kResultOk
        } else {
            kInvalidArgument
        }
    }

    unsafe fn canProcessSampleSize(&self, symbolic_sample_size: i32) -> tresult {
        if symbolic_sample_size == SymbolicSampleSizes_::kSample32 as i32 {
            kResultTrue
        } else {
            kResultFalse
        }
    }

    unsafe fn getLatencySamples(&self) -> u32 {
        0
    }

    unsafe fn setupProcessing(&self, setup: *mut ProcessSetup) -> tresult {
        let Some(setup) = setup.as_ref() else {
            return kInvalidArgument;
        };
        if setup.symbolicSampleSize != SymbolicSampleSizes_::kSample32 as i32
            || !setup.sampleRate.is_finite()
            || setup.sampleRate <= 0.0
            || setup.maxSamplesPerBlock < 0
        {
            return kInvalidArgument;
        }
        if (*self.audio_ptr()).set_sample_rate(setup.sampleRate) {
            self.sample_rate_bits
                .store(setup.sampleRate.to_bits(), Ordering::Relaxed);
            kResultOk
        } else {
            kInvalidArgument
        }
    }

    unsafe fn setProcessing(&self, _state: TBool) -> tresult {
        kResultOk
    }

    unsafe fn process(&self, data: *mut ProcessData) -> tresult {
        let Some(data) = data.as_mut() else {
            return kInvalidArgument;
        };
        if data.symbolicSampleSize != SymbolicSampleSizes_::kSample32 as i32 || data.numSamples < 0
        {
            return kInvalidArgument;
        }

        if data.numSamples == 0 {
            self.flush_parameter_changes(data.inputParameterChanges);
            return kResultOk;
        }
        if data.numInputs != 0 || data.numOutputs != 1 || data.outputs.is_null() {
            return kInvalidArgument;
        }
        let output_bus = &mut *data.outputs;
        if output_bus.numChannels != 2 || output_bus.__field0.channelBuffers32.is_null() {
            return kInvalidArgument;
        }
        let channels = slice::from_raw_parts_mut(output_bus.__field0.channelBuffers32, 2);
        if channels[0].is_null() || channels[1].is_null() {
            return kInvalidArgument;
        }
        let frames = data.numSamples as usize;
        let left = slice::from_raw_parts_mut(channels[0], frames);
        let right = slice::from_raw_parts_mut(channels[1], frames);
        let mut silent = true;
        for frame in 0..frames {
            self.apply_parameter_changes(data.inputParameterChanges, frame as i32);
            self.apply_note_events(data.inputEvents, frame as i32);
            let sample = (*self.audio_ptr()).next_sample();
            left[frame] = sample;
            right[frame] = sample;
            silent &= sample == 0.0;
        }
        output_bus.silenceFlags = if silent { 0b11 } else { 0 };
        kResultOk
    }

    unsafe fn getTailSamples(&self) -> u32 {
        let sample_rate = f64::from_bits(self.sample_rate_bits.load(Ordering::Relaxed));
        (sample_rate * 4.0).round().clamp(0.0, u32::MAX as f64) as u32
    }
}

struct Controller {
    parameters: Arc<AtomicParameters>,
    handler: Arc<Mutex<Option<ComPtr<IComponentHandler>>>>,
    repaint: RepaintNotifier,
    initialized: AtomicBool,
}

impl Controller {
    fn new() -> Self {
        Self {
            parameters: Arc::new(AtomicParameters::default()),
            handler: Arc::new(Mutex::new(None)),
            repaint: RepaintNotifier::new(),
            initialized: AtomicBool::new(false),
        }
    }
}

impl Class for Controller {
    type Interfaces = (IEditController,);
}

impl IPluginBaseTrait for Controller {
    unsafe fn initialize(&self, _context: *mut FUnknown) -> tresult {
        if self.initialized.swap(true, Ordering::AcqRel) {
            kResultFalse
        } else {
            kResultOk
        }
    }

    unsafe fn terminate(&self) -> tresult {
        self.handler.lock().expect("component handler mutex").take();
        self.initialized.store(false, Ordering::Release);
        kResultOk
    }
}

impl IEditControllerTrait for Controller {
    unsafe fn setComponentState(&self, state: *mut IBStream) -> tresult {
        let Some(bytes) = stream_read(state) else {
            return kResultFalse;
        };
        let Ok(parameters) = Parameters::decode(&bytes) else {
            return kResultFalse;
        };
        self.parameters.replace(parameters);
        self.repaint.request_repaint();
        kResultOk
    }

    unsafe fn setState(&self, state: *mut IBStream) -> tresult {
        self.setComponentState(state)
    }

    unsafe fn getState(&self, state: *mut IBStream) -> tresult {
        stream_write(state, &self.parameters.load().encode())
    }

    unsafe fn getParameterCount(&self) -> i32 {
        crate::PARAM_COUNT as i32
    }

    unsafe fn getParameterInfo(&self, index: i32, info: *mut ParameterInfo) -> tresult {
        let Some(id) = usize::try_from(index)
            .ok()
            .and_then(|index| ParamId::ALL.get(index).copied())
        else {
            return kInvalidArgument;
        };
        let Some(info) = info.as_mut() else {
            return kInvalidArgument;
        };
        info.id = id.raw();
        copy_wstring(id.name(), &mut info.title);
        copy_wstring(id.name(), &mut info.shortTitle);
        copy_wstring(id.units(), &mut info.units);
        info.stepCount = 0;
        info.defaultNormalizedValue = Parameters::default().get(id) as f64;
        info.unitId = 0;
        info.flags = ParameterInfo_::ParameterFlags_::kCanAutomate;
        kResultOk
    }

    unsafe fn getParamStringByValue(&self, id: u32, value: f64, string: *mut String128) -> tresult {
        let (Some(id), Some(string)) = (ParamId::from_raw(id), string.as_mut()) else {
            return kInvalidArgument;
        };
        copy_wstring(&id.display(value), string);
        kResultOk
    }

    unsafe fn getParamValueByString(
        &self,
        id: u32,
        string: *mut TChar,
        normalized: *mut f64,
    ) -> tresult {
        let (Some(id), Some(normalized)) = (ParamId::from_raw(id), normalized.as_mut()) else {
            return kInvalidArgument;
        };
        if string.is_null() {
            return kInvalidArgument;
        }
        let mut length = 0;
        while length < 128 && *string.add(length) != 0 {
            length += 1;
        }
        let Ok(text) = String::from_utf16(slice::from_raw_parts(string, length)) else {
            return kInvalidArgument;
        };
        let Some(token) = text.split_whitespace().next() else {
            return kInvalidArgument;
        };
        let Ok(plain) = token.trim_end_matches('%').parse::<f64>() else {
            return kInvalidArgument;
        };
        *normalized = id.plain_to_normalized(plain);
        kResultOk
    }

    unsafe fn normalizedParamToPlain(&self, id: u32, value: f64) -> f64 {
        ParamId::from_raw(id)
            .map(|id| id.normalized_to_plain(value))
            .unwrap_or(0.0)
    }

    unsafe fn plainParamToNormalized(&self, id: u32, value: f64) -> f64 {
        ParamId::from_raw(id)
            .map(|id| id.plain_to_normalized(value))
            .unwrap_or(0.0)
    }

    unsafe fn getParamNormalized(&self, id: u32) -> f64 {
        ParamId::from_raw(id)
            .map(|id| self.parameters.get(id) as f64)
            .unwrap_or(0.0)
    }

    unsafe fn setParamNormalized(&self, id: u32, value: f64) -> tresult {
        let Some(id) = ParamId::from_raw(id) else {
            return kInvalidArgument;
        };
        self.parameters.set(id, value as f32);
        self.repaint.request_repaint();
        kResultOk
    }

    unsafe fn setComponentHandler(&self, handler: *mut IComponentHandler) -> tresult {
        let retained = ComRef::from_raw(handler).map(|handler| handler.to_com_ptr());
        *self.handler.lock().expect("component handler mutex") = retained;
        kResultOk
    }

    unsafe fn createView(&self, name: *const c_char) -> *mut IPlugView {
        if !platform_is(name, ViewType::kEditor) {
            return ptr::null_mut();
        }
        ComWrapper::new(PlugView::new(
            self.parameters.clone(),
            self.handler.clone(),
            self.repaint.clone(),
        ))
        .to_com_ptr::<IPlugView>()
        .expect("PlugView exposes IPlugView")
        .into_raw()
    }
}

struct VstEditorHost {
    parameters: Arc<AtomicParameters>,
    handler: Arc<Mutex<Option<ComPtr<IComponentHandler>>>>,
}

impl EditorHost for VstEditorHost {
    fn parameters(&self) -> &AtomicParameters {
        &self.parameters
    }

    fn begin_edit(&self, id: ParamId) {
        let handler = self
            .handler
            .lock()
            .expect("component handler mutex")
            .clone();
        if let Some(handler) = handler {
            unsafe { handler.beginEdit(id.raw()) };
        }
    }

    fn perform_edit(&self, id: ParamId, value: f32) {
        self.parameters.set(id, value);
        let handler = self
            .handler
            .lock()
            .expect("component handler mutex")
            .clone();
        if let Some(handler) = handler {
            unsafe { handler.performEdit(id.raw(), value as f64) };
        }
    }

    fn end_edit(&self, id: ParamId) {
        let handler = self
            .handler
            .lock()
            .expect("component handler mutex")
            .clone();
        if let Some(handler) = handler {
            unsafe { handler.endEdit(id.raw()) };
        }
    }
}

struct NsViewParent(AppKitWindowHandle);

impl HasWindowHandle for NsViewParent {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        Ok(unsafe { WindowHandle::borrow_raw(RawWindowHandle::AppKit(self.0)) })
    }
}

struct PlugView {
    parameters: Arc<AtomicParameters>,
    handler: Arc<Mutex<Option<ComPtr<IComponentHandler>>>>,
    repaint: RepaintNotifier,
    window: RefCell<Option<baseview::Window>>,
    frame: RefCell<Option<ComPtr<IPlugFrame>>>,
}

impl PlugView {
    fn new(
        parameters: Arc<AtomicParameters>,
        handler: Arc<Mutex<Option<ComPtr<IComponentHandler>>>>,
        repaint: RepaintNotifier,
    ) -> Self {
        Self {
            parameters,
            handler,
            repaint,
            window: RefCell::new(None),
            frame: RefCell::new(None),
        }
    }
}

impl Class for PlugView {
    type Interfaces = (IPlugView,);
}

impl IPlugViewTrait for PlugView {
    unsafe fn isPlatformTypeSupported(&self, platform_type: FIDString) -> tresult {
        if platform_is(platform_type, kPlatformTypeNSView) {
            kResultTrue
        } else {
            kResultFalse
        }
    }

    unsafe fn attached(&self, parent: *mut c_void, platform_type: FIDString) -> tresult {
        if self.isPlatformTypeSupported(platform_type) != kResultTrue || parent.is_null() {
            return kInvalidArgument;
        }
        if self.window.borrow().is_some() {
            return kResultFalse;
        }
        let ns_view = NsViewParent(AppKitWindowHandle::new(
            NonNull::new(parent).expect("parent was checked non-null"),
        ));
        let host = VstEditorHost {
            parameters: self.parameters.clone(),
            handler: self.handler.clone(),
        };
        let settings = EguiWindowSettings::new()
            .with_title(PLUGIN_NAME)
            .with_size(LogicalSize {
                width: EDITOR_WIDTH,
                height: EDITOR_HEIGHT,
            })
            .with_resizable(false)
            .with_parent(&ns_view)
            .with_repaint_notifier(self.repaint.clone());
        match EguiWindow::create(settings, KickEditor::new(Box::new(host), false)) {
            Ok(window) => {
                if window.show().is_err() {
                    return kResultFalse;
                }
                self.window.replace(Some(window));
                kResultOk
            }
            Err(_) => kResultFalse,
        }
    }

    unsafe fn removed(&self) -> tresult {
        self.window.borrow_mut().take();
        kResultOk
    }

    unsafe fn onWheel(&self, _distance: f32) -> tresult {
        kResultFalse
    }

    unsafe fn onKeyDown(&self, _key: char16, _key_code: int16, _modifiers: int16) -> tresult {
        kResultFalse
    }

    unsafe fn onKeyUp(&self, _key: char16, _key_code: int16, _modifiers: int16) -> tresult {
        kResultFalse
    }

    unsafe fn getSize(&self, size: *mut ViewRect) -> tresult {
        let Some(size) = size.as_mut() else {
            return kInvalidArgument;
        };
        size.left = 0;
        size.top = 0;
        size.right = EDITOR_WIDTH as i32;
        size.bottom = EDITOR_HEIGHT as i32;
        kResultOk
    }

    unsafe fn onSize(&self, new_size: *mut ViewRect) -> tresult {
        let Some(new_size) = new_size.as_ref() else {
            return kInvalidArgument;
        };
        if new_size.right - new_size.left == EDITOR_WIDTH as i32
            && new_size.bottom - new_size.top == EDITOR_HEIGHT as i32
        {
            kResultOk
        } else {
            kResultFalse
        }
    }

    unsafe fn onFocus(&self, _state: TBool) -> tresult {
        kResultOk
    }

    unsafe fn setFrame(&self, frame: *mut IPlugFrame) -> tresult {
        let retained = ComRef::from_raw(frame).map(|frame| frame.to_com_ptr());
        self.frame.replace(retained);
        kResultOk
    }

    unsafe fn canResize(&self) -> tresult {
        kResultFalse
    }

    unsafe fn checkSizeConstraint(&self, rect: *mut ViewRect) -> tresult {
        self.getSize(rect)
    }
}

struct Factory;

impl Class for Factory {
    type Interfaces = (IPluginFactory2,);
}

impl IPluginFactoryTrait for Factory {
    unsafe fn getFactoryInfo(&self, info: *mut PFactoryInfo) -> tresult {
        let Some(info) = info.as_mut() else {
            return kInvalidArgument;
        };
        copy_cstring(VENDOR, &mut info.vendor);
        copy_cstring("https://github.com/gooey-audio/libgooey", &mut info.url);
        copy_cstring("", &mut info.email);
        info.flags = PFactoryInfo_::FactoryFlags_::kUnicode as i32;
        kResultOk
    }

    unsafe fn countClasses(&self) -> i32 {
        2
    }

    unsafe fn getClassInfo(&self, index: i32, info: *mut PClassInfo) -> tresult {
        let Some(info) = info.as_mut() else {
            return kInvalidArgument;
        };
        match index {
            0 => {
                info.cid = PROCESSOR_CID;
                info.cardinality = PClassInfo_::ClassCardinality_::kManyInstances as i32;
                copy_cstring("Audio Module Class", &mut info.category);
                copy_cstring(PLUGIN_NAME, &mut info.name);
                kResultOk
            }
            1 => {
                info.cid = CONTROLLER_CID;
                info.cardinality = PClassInfo_::ClassCardinality_::kManyInstances as i32;
                copy_cstring("Component Controller Class", &mut info.category);
                copy_cstring(PLUGIN_NAME, &mut info.name);
                kResultOk
            }
            _ => kInvalidArgument,
        }
    }

    unsafe fn createInstance(
        &self,
        cid: FIDString,
        iid: FIDString,
        object: *mut *mut c_void,
    ) -> tresult {
        if cid.is_null() || iid.is_null() || object.is_null() {
            return kInvalidArgument;
        }
        *object = ptr::null_mut();
        let instance = match *(cid as *const TUID) {
            PROCESSOR_CID => ComWrapper::new(Processor::new())
                .to_com_ptr::<FUnknown>()
                .expect("processor exposes FUnknown"),
            CONTROLLER_CID => ComWrapper::new(Controller::new())
                .to_com_ptr::<FUnknown>()
                .expect("controller exposes FUnknown"),
            _ => return kNoInterface,
        };
        let raw = instance.as_ptr();
        ((*(*raw).vtbl).queryInterface)(raw, iid as *mut TUID, object)
    }
}

impl IPluginFactory2Trait for Factory {
    unsafe fn getClassInfo2(&self, index: i32, info: *mut PClassInfo2) -> tresult {
        let Some(info) = info.as_mut() else {
            return kInvalidArgument;
        };
        match index {
            0 => {
                info.cid = PROCESSOR_CID;
                info.cardinality = PClassInfo_::ClassCardinality_::kManyInstances as i32;
                copy_cstring("Audio Module Class", &mut info.category);
                copy_cstring(PLUGIN_NAME, &mut info.name);
                info.classFlags = 0;
                copy_cstring("Instrument|Drum", &mut info.subCategories);
                copy_cstring(VENDOR, &mut info.vendor);
                copy_cstring(VERSION, &mut info.version);
                copy_cstring("VST 3.7", &mut info.sdkVersion);
                kResultOk
            }
            1 => {
                info.cid = CONTROLLER_CID;
                info.cardinality = PClassInfo_::ClassCardinality_::kManyInstances as i32;
                copy_cstring("Component Controller Class", &mut info.category);
                copy_cstring(PLUGIN_NAME, &mut info.name);
                info.classFlags = 0;
                copy_cstring("", &mut info.subCategories);
                copy_cstring(VENDOR, &mut info.vendor);
                copy_cstring(VERSION, &mut info.version);
                copy_cstring("VST 3.7", &mut info.sdkVersion);
                kResultOk
            }
            _ => kInvalidArgument,
        }
    }
}

#[no_mangle]
pub extern "system" fn bundleEntry(_bundle_ref: *mut c_void) -> bool {
    true
}

#[no_mangle]
pub extern "system" fn bundleExit() -> bool {
    true
}

#[no_mangle]
pub extern "system" fn GetPluginFactory() -> *mut IPluginFactory {
    ComWrapper::new(Factory)
        .to_com_ptr::<IPluginFactory>()
        .expect("factory exposes IPluginFactory")
        .into_raw()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn factory_enumerates_stable_processor_and_controller_classes() {
        let factory = Factory;
        unsafe {
            assert_eq!(factory.countClasses(), 2);
            let mut processor: PClassInfo2 = std::mem::zeroed();
            let mut controller: PClassInfo2 = std::mem::zeroed();
            assert_eq!(factory.getClassInfo2(0, &mut processor), kResultOk);
            assert_eq!(factory.getClassInfo2(1, &mut controller), kResultOk);
            assert_eq!(processor.cid, PROCESSOR_CID);
            assert_eq!(controller.cid, CONTROLLER_CID);
        }
    }

    #[test]
    fn exported_factory_queries_as_plugin_factory_two() {
        unsafe {
            let factory = ComPtr::<IPluginFactory>::from_raw(GetPluginFactory())
                .expect("exported factory pointer");
            let factory_two = factory
                .cast::<IPluginFactory2>()
                .expect("IPluginFactory2 query");
            assert_eq!(factory_two.countClasses(), 2);
            let mut info: PClassInfo2 = std::mem::zeroed();
            assert_eq!(factory_two.getClassInfo2(0, &mut info), kResultOk);
            assert_eq!(info.cid, PROCESSOR_CID);
        }
    }

    #[test]
    fn processor_and_controller_interfaces_can_be_created_and_released() {
        unsafe {
            let factory = Factory;
            let mut processor = ptr::null_mut();
            assert_eq!(
                factory.createInstance(
                    PROCESSOR_CID.as_ptr().cast(),
                    IAudioProcessor_iid.as_ptr().cast(),
                    &mut processor,
                ),
                kResultOk
            );
            assert!(!processor.is_null());
            let processor = ComPtr::<IAudioProcessor>::from_raw(processor.cast())
                .expect("audio processor pointer");
            assert!(processor.cast::<IComponent>().is_some());

            let mut controller = ptr::null_mut();
            assert_eq!(
                factory.createInstance(
                    CONTROLLER_CID.as_ptr().cast(),
                    IEditController_iid.as_ptr().cast(),
                    &mut controller,
                ),
                kResultOk
            );
            assert!(!controller.is_null());
            drop(ComPtr::<IEditController>::from_raw(controller.cast()));
        }
    }

    #[test]
    fn processor_declares_required_buses_and_sample_format() {
        let processor = Processor::new();
        unsafe {
            assert_eq!(
                processor.getBusCount(MediaTypes_::kAudio as i32, BusDirections_::kInput as i32),
                0
            );
            assert_eq!(
                processor.getBusCount(MediaTypes_::kAudio as i32, BusDirections_::kOutput as i32),
                1
            );
            assert_eq!(
                processor.getBusCount(MediaTypes_::kEvent as i32, BusDirections_::kInput as i32),
                1
            );
            assert_eq!(
                processor.canProcessSampleSize(SymbolicSampleSizes_::kSample32 as i32),
                kResultTrue
            );
            assert_eq!(
                processor.canProcessSampleSize(SymbolicSampleSizes_::kSample64 as i32),
                kResultFalse
            );
        }
    }

    #[test]
    fn controller_describes_all_parameters() {
        let controller = Controller::new();
        unsafe {
            assert_eq!(controller.getParameterCount(), crate::PARAM_COUNT as i32);
            for (index, id) in ParamId::ALL.into_iter().enumerate() {
                let mut info: ParameterInfo = std::mem::zeroed();
                assert_eq!(
                    controller.getParameterInfo(index as i32, &mut info),
                    kResultOk
                );
                assert_eq!(info.id, id.raw());
                assert_eq!(info.flags, ParameterInfo_::ParameterFlags_::kCanAutomate);
            }
        }
    }
}
