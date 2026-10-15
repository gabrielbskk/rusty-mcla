//! AVM1 (ActionScript 1/2) byte code as stored in the game's movies. The
//! records are standard SWF actions: one code byte, and for codes >= 0x80 a
//! little-endian u16 payload length and the payload.

use std::fmt::Write;

/// Length of an action block up to and including its End (0x00) action, or the
/// whole slice when it is not terminated.
pub fn block_len(code: &[u8]) -> usize {
    let mut p = 0;
    while p < code.len() {
        let c = code[p];
        if c == 0 {
            return p + 1;
        }
        if c >= 0x80 {
            if p + 3 > code.len() {
                return code.len();
            }
            p += 3 + u16::from_le_bytes([code[p + 1], code[p + 2]]) as usize;
        } else {
            p += 1;
        }
    }
    code.len()
}

/// A value of an ActionPush record.
#[derive(Debug, Clone, PartialEq)]
pub enum PushValue {
    Str(String),
    Float(f32),
    Null,
    Undefined,
    Register(u8),
    Bool(bool),
    Double(f64),
    Int(i32),
    Constant(u16),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Function2 {
    pub name: String,
    pub params: Vec<(u8, String)>,
    pub registers: u8,
    pub flags: u16,
    /// offset of the body relative to the start of the code block
    pub body: usize,
    pub body_len: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    End,
    /// simple action without payload
    Op(u8),
    GotoFrame(u16),
    GetUrl(String, String),
    StoreRegister(u8),
    ConstantPool(Vec<String>),
    WaitForFrame(u16, u8),
    SetTarget(String),
    GotoLabel(String),
    Push(Vec<PushValue>),
    Jump(i16),
    If(i16),
    GotoFrame2(u8, u16),
    DefineFunction { name: String, params: Vec<String>, body: usize, body_len: usize },
    DefineFunction2(Function2),
    With(u16),
    Try,
    /// a payload action this decoder does not know
    Unknown(u8, Vec<u8>),
}

fn cstr(b: &[u8], p: &mut usize) -> String {
    let s = *p;
    while *p < b.len() && b[*p] != 0 {
        *p += 1;
    }
    let out = String::from_utf8_lossy(&b[s..*p]).into_owned();
    *p += 1;
    out
}

/// Decode one action at `p`; returns the action and the offset of the next one.
pub fn decode(code: &[u8], p: usize) -> Option<(Action, usize)> {
    let c = *code.get(p)?;
    if c == 0 {
        return Some((Action::End, p + 1));
    }
    if c < 0x80 {
        return Some((Action::Op(c), p + 1));
    }
    let len = u16::from_le_bytes([*code.get(p + 1)?, *code.get(p + 2)?]) as usize;
    let s = p + 3;
    let e = s + len;
    let b = code.get(s..e)?;
    let u16at = |o: usize| u16::from_le_bytes([b[o], b[o + 1]]);
    let a = match c {
        0x81 => Action::GotoFrame(u16at(0)),
        0x83 => {
            let mut q = 0;
            let url = cstr(b, &mut q);
            let target = cstr(b, &mut q);
            Action::GetUrl(url, target)
        }
        0x87 => Action::StoreRegister(b[0]),
        0x88 => {
            let n = u16at(0) as usize;
            let mut q = 2;
            Action::ConstantPool((0..n).map(|_| cstr(b, &mut q)).collect())
        }
        0x8a => Action::WaitForFrame(u16at(0), b[2]),
        0x8b => {
            let mut q = 0;
            Action::SetTarget(cstr(b, &mut q))
        }
        0x8c => {
            let mut q = 0;
            Action::GotoLabel(cstr(b, &mut q))
        }
        0x96 => {
            let mut v = Vec::new();
            let mut q = 0;
            while q < b.len() {
                let t = b[q];
                q += 1;
                v.push(match t {
                    0 => PushValue::Str(cstr(b, &mut q)),
                    1 => {
                        let f = f32::from_le_bytes(b[q..q + 4].try_into().ok()?);
                        q += 4;
                        PushValue::Float(f)
                    }
                    2 => PushValue::Null,
                    3 => PushValue::Undefined,
                    4 => {
                        q += 1;
                        PushValue::Register(b[q - 1])
                    }
                    5 => {
                        q += 1;
                        PushValue::Bool(b[q - 1] != 0)
                    }
                    6 => {
                        // SWF doubles are stored as two little-endian halves, high word first
                        let hi = u32::from_le_bytes(b[q..q + 4].try_into().ok()?);
                        let lo = u32::from_le_bytes(b[q + 4..q + 8].try_into().ok()?);
                        q += 8;
                        PushValue::Double(f64::from_bits((hi as u64) << 32 | lo as u64))
                    }
                    7 => {
                        let i = i32::from_le_bytes(b[q..q + 4].try_into().ok()?);
                        q += 4;
                        PushValue::Int(i)
                    }
                    8 => {
                        q += 1;
                        PushValue::Constant(b[q - 1] as u16)
                    }
                    9 => {
                        q += 2;
                        PushValue::Constant(u16at(q - 2))
                    }
                    _ => return Some((Action::Unknown(c, b.to_vec()), e)),
                });
            }
            Action::Push(v)
        }
        0x99 => Action::Jump(u16at(0) as i16),
        0x9d => Action::If(u16at(0) as i16),
        0x9f => Action::GotoFrame2(b[0], if b[0] & 2 != 0 { u16at(1) } else { 0 }),
        0x9b => {
            let mut q = 0;
            let name = cstr(b, &mut q);
            let n = u16at(q) as usize;
            q += 2;
            let params = (0..n).map(|_| cstr(b, &mut q)).collect();
            let body_len = u16at(q) as usize;
            Action::DefineFunction { name, params, body: e, body_len }
        }
        0x8e => {
            let mut q = 0;
            let name = cstr(b, &mut q);
            let n = u16at(q) as usize;
            let registers = b[q + 2];
            let flags = u16at(q + 3);
            q += 5;
            let mut params = Vec::new();
            for _ in 0..n {
                let r = b[q];
                q += 1;
                params.push((r, cstr(b, &mut q)));
            }
            let body_len = u16at(q) as usize;
            Action::DefineFunction2(Function2 { name, params, registers, flags, body: e, body_len })
        }
        0x94 => Action::With(u16at(0)),
        0x8f => Action::Try,
        _ => Action::Unknown(c, b.to_vec()),
    };
    Some((a, e))
}

/// Name of a payload-free action code.
pub fn op_name(c: u8) -> &'static str {
    match c {
        0x04 => "NextFrame",
        0x05 => "PrevFrame",
        0x06 => "Play",
        0x07 => "Stop",
        0x08 => "ToggleQuality",
        0x09 => "StopSounds",
        0x0a => "Add",
        0x0b => "Subtract",
        0x0c => "Multiply",
        0x0d => "Divide",
        0x0e => "Equals",
        0x0f => "Less",
        0x10 => "And",
        0x11 => "Or",
        0x12 => "Not",
        0x13 => "StringEquals",
        0x14 => "StringLength",
        0x15 => "StringExtract",
        0x17 => "Pop",
        0x18 => "ToInteger",
        0x1c => "GetVariable",
        0x1d => "SetVariable",
        0x20 => "SetTarget2",
        0x21 => "StringAdd",
        0x22 => "GetProperty",
        0x23 => "SetProperty",
        0x24 => "CloneSprite",
        0x25 => "RemoveSprite",
        0x26 => "Trace",
        0x27 => "StartDrag",
        0x28 => "EndDrag",
        0x29 => "StringLess",
        0x2a => "Throw",
        0x2b => "CastOp",
        0x2c => "ImplementsOp",
        0x30 => "RandomNumber",
        0x31 => "MBStringLength",
        0x32 => "CharToAscii",
        0x33 => "AsciiToChar",
        0x34 => "GetTime",
        0x35 => "MBStringExtract",
        0x36 => "MBCharToAscii",
        0x37 => "MBAsciiToChar",
        0x3a => "Delete",
        0x3b => "Delete2",
        0x3c => "DefineLocal",
        0x3d => "CallFunction",
        0x3e => "Return",
        0x3f => "Modulo",
        0x40 => "NewObject",
        0x41 => "DefineLocal2",
        0x42 => "InitArray",
        0x43 => "InitObject",
        0x44 => "TypeOf",
        0x45 => "TargetPath",
        0x46 => "Enumerate",
        0x47 => "Add2",
        0x48 => "Less2",
        0x49 => "Equals2",
        0x4a => "ToNumber",
        0x4b => "ToString",
        0x4c => "PushDuplicate",
        0x4d => "StackSwap",
        0x4e => "GetMember",
        0x4f => "SetMember",
        0x50 => "Increment",
        0x51 => "Decrement",
        0x52 => "CallMethod",
        0x53 => "NewMethod",
        0x54 => "InstanceOf",
        0x55 => "Enumerate2",
        0x60 => "BitAnd",
        0x61 => "BitOr",
        0x62 => "BitXor",
        0x63 => "BitLShift",
        0x64 => "BitRShift",
        0x65 => "BitURShift",
        0x66 => "StrictEquals",
        0x67 => "Greater",
        0x68 => "StringGreater",
        0x69 => "Extends",
        _ => "?",
    }
}

/// Human-readable listing of a code block, resolving constant-pool references.
pub fn disassemble(code: &[u8]) -> String {
    let mut out = String::new();
    let mut pool: Vec<String> = Vec::new();
    let mut p = 0;
    while let Some((a, n)) = decode(code, p) {
        let _ = write!(out, "{:05x}  ", p);
        match &a {
            Action::End => {
                out.push_str("End\n");
                // function bodies follow their definitions; a top-level End closes the block
                break;
            }
            Action::Op(c) => {
                let _ = writeln!(out, "{}", op_name(*c));
            }
            Action::ConstantPool(v) => {
                let _ = writeln!(out, "ConstantPool [{} strings]", v.len());
                pool = v.clone();
            }
            Action::Push(v) => {
                let items: Vec<String> = v
                    .iter()
                    .map(|x| match x {
                        PushValue::Constant(i) => format!("{:?}", pool.get(*i as usize).map(|s| s.as_str()).unwrap_or("?")),
                        PushValue::Str(s) => format!("{:?}", s),
                        PushValue::Register(r) => format!("r{}", r),
                        other => format!("{:?}", other),
                    })
                    .collect();
                let _ = writeln!(out, "Push {}", items.join(", "));
            }
            Action::Jump(o) | Action::If(o) => {
                let name = if matches!(a, Action::Jump(_)) { "Jump" } else { "If" };
                let _ = writeln!(out, "{} -> {:05x}", name, n as i64 + *o as i64);
            }
            Action::DefineFunction2(f) => {
                let _ = writeln!(out, "DefineFunction2 {:?} {:?} regs={} flags={:#x} body {:05x}..{:05x}", f.name, f.params, f.registers, f.flags, f.body, f.body + f.body_len);
            }
            other => {
                let _ = writeln!(out, "{:?}", other);
            }
        }
        p = n;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_and_pool() {
        // ConstantPool ["a","bc"], Push c8:0, f32 1.5, Push reg 2, GetVariable, End
        let mut code = vec![0x88, 7, 0, 2, 0, b'a', 0, b'b', b'c', 0];
        code.extend([0x96, 7, 0, 8, 0, 1]);
        code.extend(1.5f32.to_le_bytes());
        code.extend([0x96, 2, 0, 4, 2, 0x1c, 0x00]);
        assert_eq!(block_len(&code), code.len());
        let (a, n) = decode(&code, 0).unwrap();
        assert_eq!(a, Action::ConstantPool(vec!["a".into(), "bc".into()]));
        let (a, n) = decode(&code, n).unwrap();
        assert_eq!(a, Action::Push(vec![PushValue::Constant(0), PushValue::Float(1.5)]));
        let (a, n) = decode(&code, n).unwrap();
        assert_eq!(a, Action::Push(vec![PushValue::Register(2)]));
        let (a, n) = decode(&code, n).unwrap();
        assert_eq!(a, Action::Op(0x1c));
        assert_eq!(decode(&code, n).unwrap().0, Action::End);
    }

    #[test]
    fn swf_double_word_order() {
        let v = 3.25f64.to_bits();
        let mut code = vec![0x96, 9, 0, 6];
        code.extend(((v >> 32) as u32).to_le_bytes());
        code.extend((v as u32).to_le_bytes());
        assert_eq!(decode(&code, 0).unwrap().0, Action::Push(vec![PushValue::Double(3.25)]));
    }
}
