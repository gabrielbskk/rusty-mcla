//! Original engine connection/free-speed block with already evaluated torque.
//! Curve and shift-envelope evaluation and full-frame dispatch are caller work.

#[derive(Clone, Copy, Debug)]
pub struct PreparedEngine {
    pub speed: f32,
    pub torque: f32,
    pub pedal: f32,
    pub idle_speed: f32,
    pub max_speed: f32,
    pub inertia: f32,
    pub engage_rpm: f32,
    pub ratio: f32,
    /// Original convention: negative forward shaft speed.
    pub shaft: f32,
    /// Original transmission index: reverse 0, neutral 1, first 2.
    pub gear: i32,
    pub running: bool,
    pub inhibited: bool,
    pub bound: bool,
    /// Engine +80, approached toward zero while connected.
    pub auxiliary: f32,
    /// Chassis +372 == 4 inhibits binding in sub_82568D78.
    pub can_bind: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct EngineResult {
    pub speed: f32,
    pub torque: f32,
    pub bound: bool,
    pub auxiliary: f32,
}

/// sub_82559A70 0x82559CD4..0x82559E6C and binding/approach leaves.
pub fn update_prepared(e: PreparedEngine, dt: f32) -> EngineResult {
    let mut out = EngineResult { speed: e.speed, torque: e.torque,
                                bound: e.bound, auxiliary: e.auxiliary };
    if e.ratio != 0.0 && e.running && !e.inhibited {
        let locked = (e.ratio * e.shaft).abs();
        if locked < e.idle_speed * 0.75 && (e.gear == 0 || e.gear == 2) && e.pedal < 0.1 {
            out.bound = false;
        } else if locked > e.idle_speed * 1.25 || e.pedal > 0.0 {
            if !out.bound && e.can_bind { out.bound = true; }
        }
    } else {
        out.bound = false;
    }
    if out.bound && !e.inhibited {
        let rate = e.engage_rpm * 0.104_719_76;
        out.auxiliary = if e.auxiliary < 0.0 { rate.mul_add(dt, e.auxiliary).min(0.0) }
            else if e.auxiliary > 0.0 { (-rate.mul_add(dt, -e.auxiliary)).max(0.0) }
            else { e.auxiliary };
    } else if !e.running {
        out.speed = 0.0;
        out.torque = 0.0;
    } else {
        out.speed = (e.torque / e.inertia).mul_add(dt, e.speed).clamp(0.0, e.max_speed);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_capture(p: &[f32]) {
        assert_eq!(p.len(), 20);
        let e = PreparedEngine { speed: p[0], torque: p[1], pedal: p[2], idle_speed: p[3],
            max_speed: p[4], inertia: p[5], engage_rpm: p[6], ratio: p[7], shaft: p[8],
            gear: p[9] as i32, running: p[10] != 0.0, inhibited: p[11] != 0.0,
            bound: p[12] != 0.0, auxiliary: p[13], can_bind: p[14] != 4.0 };
        let actual = update_prepared(e, p[15]);
        for (a, b) in [actual.speed, actual.torque, actual.auxiliary].into_iter()
            .zip([p[16], p[17], p[19]]) {
            assert_eq!(a.to_bits(), b.to_bits(), "{p:?}: {actual:?}");
        }
        assert_eq!(actual.bound, p[18] != 0.0, "{p:?}: {actual:?}");
    }

    #[test]
    fn connection_boundaries_and_free_engine_match_original_captures() {
        // Half-open clutch threshold, fresh binding, free integration and stop.
        for line in [
            "100,100,0,100,800,0.125,3500,10,-7.5,2,1,0,1,10,2,0.0166666675,100,100,1,3.89134693",
            "100,100,0,100,800,0.125,3500,10,-7.49999905,2,1,0,1,10,2,0.0166666675,113.333336,100,0,10",
            "100,100,0.100000001,100,800,0.125,3500,10,0,2,1,0,0,10,2,0.0166666675,100,100,1,3.89134693",
            "790,400,1,100,800,0.0599999987,3500,10,0,2,1,1,1,10,2,0.0166666675,800,400,0,10",
            "100,-400,0.5,100,800,0.0599999987,3500,10,-12.499999,3,0,1,1,10,0,0.0333333351,0,0,0,10",
        ] {
            let p: Vec<f32> = line.split(',').map(|s| s.parse().unwrap()).collect();
            assert_capture(&p);
        }
    }

    #[test]
    #[ignore = "requires MCLA_ENGINE_CONNECTION_REFERENCE from tools/engine_connection_reference.py"]
    fn connection_and_free_speed_match_original_generated_code() {
        let path = std::env::var("MCLA_ENGINE_CONNECTION_REFERENCE").expect("MCLA_ENGINE_CONNECTION_REFERENCE");
        let csv = std::fs::read_to_string(path).unwrap();
        let mut count = 0;
        for line in csv.lines().skip(1) {
            let p: Vec<f32> = line.split(',').map(|s| s.parse().unwrap()).collect();
            assert_capture(&p);
            count += 1;
        }
        assert_eq!(count, 4000);
    }
}
