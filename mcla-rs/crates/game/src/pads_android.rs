//! Temporary Android input adapter. NativeActivity has no SDL Activity, so
//! desktop SDL gamepad initialization cannot be used here.

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct PadId(pub i32);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Button {
    South, East, West, North, LeftShoulder, RightShoulder,
    LeftTrigger, RightTrigger, Back, Start, LeftStick, RightStick,
    DPadUp, DPadDown, DPadLeft, DPadRight,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Axis { LeftX, LeftY, RightX, RightY, LeftTrigger, RightTrigger }

#[derive(Clone, Debug)]
pub enum PadEvent { Connected, Disconnected, Pressed(Button), Released(Button), Moved(Axis, f32), Raw(String) }

pub struct Pad;
impl Pad {
    pub fn name(&self) -> String { String::new() }
    pub fn axis(&self, _a: Axis) -> f32 { 0.0 }
    pub fn pressed(&self, _b: Button) -> bool { false }
    pub fn kind(&self) -> &'static str { "Android" }
    pub fn has_rumble(&self) -> bool { false }
    pub fn mapping(&self) -> String { String::new() }
    pub fn usb_id(&self) -> (u16, u16) { (0, 0) }
    pub fn rank(&self) -> u8 { 0 }
}

pub struct Pads { pub raw: bool }
impl Pads {
    pub fn new() -> Result<Self, String> { Err("Android gamepad adapter is not implemented".into()) }
    pub fn get(&self, _id: PadId) -> Option<&Pad> { None }
    pub fn poll(&mut self) -> Vec<(PadId, PadEvent)> { Vec::new() }
}
