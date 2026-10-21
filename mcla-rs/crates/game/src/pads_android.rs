//! Android gamepad input delivered by `NativeGameActivity` through Android's
//! KeyEvent and MotionEvent APIs. Values match the SDL adapter: sticks are
//! -1..1 with Y positive upwards; triggers are 0..1.

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct PadId(pub i32);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Button {
    South, East, West, North,
    LeftShoulder, RightShoulder, LeftTrigger, RightTrigger,
    Back, Start, LeftStick, RightStick,
    DPadUp, DPadDown, DPadLeft, DPadRight,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Axis { LeftX, LeftY, RightX, RightY, LeftTrigger, RightTrigger }

#[derive(Clone, Debug)]
pub enum PadEvent {
    Connected,
    Disconnected,
    Pressed(Button),
    Released(Button),
    Moved(Axis, f32),
    #[allow(dead_code)]
    Raw(String),
}

const AXIS_COUNT: usize = 32;
const AXIS_X: usize = 0;
const AXIS_Y: usize = 1;
const AXIS_Z: usize = 11;
const AXIS_RX: usize = 12;
const AXIS_RY: usize = 13;
const AXIS_RZ: usize = 14;
const AXIS_HAT_X: usize = 15;
const AXIS_HAT_Y: usize = 16;
const AXIS_LTRIGGER: usize = 17;
const AXIS_RTRIGGER: usize = 18;
const AXIS_GAS: usize = 22;
const AXIS_BRAKE: usize = 23;
const TRIGGER_PRESS: f32 = 0.75;

#[derive(Clone)]
struct ControllerState {
    axes: [f32; AXIS_COUNT],
    buttons: HashSet<i32>,
}

impl Default for ControllerState {
    fn default() -> Self { Self { axes: [0.0; AXIS_COUNT], buttons: HashSet::new() } }
}

enum DeviceEvent { Connected(i32), Disconnected(i32) }

#[derive(Default)]
struct Registry {
    devices: HashMap<i32, ControllerState>,
    events: Vec<DeviceEvent>,
}

static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();

fn registry() -> &'static Mutex<Registry> { REGISTRY.get_or_init(|| Mutex::new(Registry::default())) }

#[no_mangle]
pub extern "system" fn Java_org_rustymcla_game_NativeGameActivity_nativePadConnected(
    _env: *mut std::ffi::c_void, _class: *mut std::ffi::c_void, id: i32,
) {
    if let Ok(mut state) = registry().lock() {
        if !state.devices.contains_key(&id) {
            state.devices.insert(id, ControllerState::default());
            state.events.push(DeviceEvent::Connected(id));
        }
    }
}

#[no_mangle]
pub extern "system" fn Java_org_rustymcla_game_NativeGameActivity_nativePadDisconnected(
    _env: *mut std::ffi::c_void, _class: *mut std::ffi::c_void, id: i32,
) {
    if let Ok(mut state) = registry().lock() {
        if state.devices.remove(&id).is_some() { state.events.push(DeviceEvent::Disconnected(id)); }
    }
}

#[no_mangle]
pub extern "system" fn Java_org_rustymcla_game_NativeGameActivity_nativePadAxis(
    _env: *mut std::ffi::c_void, _class: *mut std::ffi::c_void, id: i32, axis: i32, value: f32,
) {
    if !(0..AXIS_COUNT as i32).contains(&axis) || !value.is_finite() { return; }
    if let Ok(mut state) = registry().lock() {
        if let Some(device) = state.devices.get_mut(&id) {
            device.axes[axis as usize] = value.clamp(-1.0, 1.0);
        }
    }
}

#[no_mangle]
pub extern "system" fn Java_org_rustymcla_game_NativeGameActivity_nativePadButton(
    _env: *mut std::ffi::c_void, _class: *mut std::ffi::c_void, id: i32, key: i32, down: u8,
) {
    if let Ok(mut state) = registry().lock() {
        if let Some(device) = state.devices.get_mut(&id) {
            if down != 0 { device.buttons.insert(key); } else { device.buttons.remove(&key); }
        }
    }
}

fn key(button: Button) -> Option<i32> {
    Some(match button {
        Button::South => 96, Button::East => 97, Button::West => 99, Button::North => 100,
        Button::LeftShoulder => 102, Button::RightShoulder => 103,
        Button::LeftTrigger => 104, Button::RightTrigger => 105,
        Button::LeftStick => 106, Button::RightStick => 107,
        Button::Start => 108, Button::Back => 109,
        Button::DPadUp => 19, Button::DPadDown => 20, Button::DPadLeft => 21, Button::DPadRight => 22,
    })
}

const BUTTONS: [Button; 16] = [
    Button::South, Button::East, Button::West, Button::North,
    Button::LeftShoulder, Button::RightShoulder, Button::LeftTrigger, Button::RightTrigger,
    Button::Back, Button::Start, Button::LeftStick, Button::RightStick,
    Button::DPadUp, Button::DPadDown, Button::DPadLeft, Button::DPadRight,
];

pub struct Pad {
    id: PadId,
    axes: [f32; AXIS_COUNT],
    buttons: HashSet<i32>,
}

impl Pad {
    pub fn name(&self) -> String { format!("Android controller {}", self.id.0) }
    pub fn kind(&self) -> &'static str { "Android gamepad" }
    pub fn usb_id(&self) -> (u16, u16) { (0, 0) }
    pub fn has_rumble(&self) -> bool { false }
    pub fn rank(&self) -> u8 { 1 }
    pub fn mapping(&self) -> String { "Android KeyEvent/MotionEvent standard mapping".into() }

    fn raw_axis(&self, axis: usize) -> f32 { self.axes.get(axis).copied().unwrap_or(0.0) }

    pub fn axis(&self, axis: Axis) -> f32 {
        match axis {
            Axis::LeftX => self.raw_axis(AXIS_X),
            Axis::LeftY => -self.raw_axis(AXIS_Y),
            Axis::RightX => {
                let z = self.raw_axis(AXIS_Z);
                if z.abs() > 0.02 { z } else { self.raw_axis(AXIS_RX) }
            }
            Axis::RightY => {
                let rz = self.raw_axis(AXIS_RZ);
                if rz.abs() > 0.02 { -rz } else { -self.raw_axis(AXIS_RY) }
            }
            Axis::LeftTrigger => self.raw_axis(AXIS_LTRIGGER).max(self.raw_axis(AXIS_BRAKE))
                .max(if self.buttons.contains(&104) { 1.0 } else { 0.0 }).clamp(0.0, 1.0),
            Axis::RightTrigger => self.raw_axis(AXIS_RTRIGGER).max(self.raw_axis(AXIS_GAS))
                .max(if self.buttons.contains(&105) { 1.0 } else { 0.0 }).clamp(0.0, 1.0),
        }
    }

    pub fn pressed(&self, button: Button) -> bool {
        let physical = key(button).is_some_and(|k| self.buttons.contains(&k));
        match button {
            Button::LeftTrigger => physical || self.axis(Axis::LeftTrigger) >= TRIGGER_PRESS,
            Button::RightTrigger => physical || self.axis(Axis::RightTrigger) >= TRIGGER_PRESS,
            Button::DPadLeft => physical || self.raw_axis(AXIS_HAT_X) < -0.5,
            Button::DPadRight => physical || self.raw_axis(AXIS_HAT_X) > 0.5,
            Button::DPadUp => physical || self.raw_axis(AXIS_HAT_Y) < -0.5,
            Button::DPadDown => physical || self.raw_axis(AXIS_HAT_Y) > 0.5,
            _ => physical,
        }
    }
}

pub struct Pads {
    pads: HashMap<PadId, Pad>,
    pub raw: bool,
}

impl Pads {
    pub fn new() -> Result<Pads, String> { Ok(Pads { pads: HashMap::new(), raw: false }) }
    pub fn get(&self, id: PadId) -> Option<&Pad> { self.pads.get(&id) }

    pub fn poll(&mut self) -> Vec<(PadId, PadEvent)> {
        let (events, snapshots) = match registry().lock() {
            Ok(mut state) => (std::mem::take(&mut state.events), state.devices.clone()),
            Err(_) => return Vec::new(),
        };
        let mut out = Vec::new();
        for event in events {
            match event {
                DeviceEvent::Connected(raw_id) => {
                    let id = PadId(raw_id);
                    let snapshot = snapshots.get(&raw_id).cloned().unwrap_or_default();
                    self.pads.insert(id, Pad { id, axes: [0.0; AXIS_COUNT], buttons: HashSet::new() });
                    out.push((id, PadEvent::Connected));
                    if let Some(pad) = self.pads.get_mut(&id) { update_pad(pad, snapshot, &mut out); }
                }
                DeviceEvent::Disconnected(raw_id) => {
                    let id = PadId(raw_id);
                    if self.pads.remove(&id).is_some() { out.push((id, PadEvent::Disconnected)); }
                }
            }
        }
        for (raw_id, snapshot) in snapshots {
            let id = PadId(raw_id);
            if let Some(pad) = self.pads.get_mut(&id) { update_pad(pad, snapshot, &mut out); }
        }
        out
    }
}

fn update_pad(pad: &mut Pad, state: ControllerState, out: &mut Vec<(PadId, PadEvent)>) {
    let old_buttons: Vec<_> = BUTTONS.iter().map(|b| (*b, pad.pressed(*b))).collect();
    let old_axes = [
        pad.axis(Axis::LeftX), pad.axis(Axis::LeftY), pad.axis(Axis::RightX),
        pad.axis(Axis::RightY), pad.axis(Axis::LeftTrigger), pad.axis(Axis::RightTrigger),
    ];
    pad.axes = state.axes;
    pad.buttons = state.buttons;
    let axes = [Axis::LeftX, Axis::LeftY, Axis::RightX, Axis::RightY, Axis::LeftTrigger, Axis::RightTrigger];
    for (axis, old) in axes.into_iter().zip(old_axes) {
        let now = pad.axis(axis);
        if (now - old).abs() > 0.004 { out.push((pad.id, PadEvent::Moved(axis, now))); }
    }
    for (button, was_down) in old_buttons {
        let now = pad.pressed(button);
        if now != was_down { out.push((pad.id, if now { PadEvent::Pressed(button) } else { PadEvent::Released(button) })); }
    }
}
