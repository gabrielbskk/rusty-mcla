//! Prepared wheel/shaft solve from vehDrivetrain::Update (0x82569440..0x82569598).
//!
//! This core uses the original signed shaft coordinates (forward rolling is
//! negative). Contact preparation and differential redistribution must supply
//! its limits and stiffnesses. It is staged separately from vehsim's existing
//! explicit solver until those inputs have been ported and compared as well.

/// Original rpm conversion used by the converter and RPM matching.
const RPM_TO_RADIANS: f32 = 0.104_719_76;

/// The connected engine's converter changes ratio from its live RPM, not from
/// the drivetrain's locked RPM (sub_82568F10, 0x82569038..0x82569078).
pub fn effective_ratio(ratio: f32, engine_speed: f32, engine_torque: f32,
                       engage_rpm: f32, cvt: f32) -> f32 {
    if cvt > 1.0 && engine_torque > 1.0 {
        let share = (engine_speed.abs() / (engage_rpm * RPM_TO_RADIANS)).clamp(0.0, 1.0);
        (1.0 - share).mul_add(cvt, 1.0) * ratio
    } else { ratio }
}

/// Original extra drive term, staged for the connected implicit drivetrain.
/// Uses original signed shaft speed and requires no gear-change direction.
#[allow(clippy::too_many_arguments)]
pub fn gear_change_inertia(effective_ratio: f32, engine_speed: f32, shaft_speed: f32,
                           pedal: f32, inertia: f32, inverse_step: f32, direction: i8) -> f32 {
    if inertia <= 0.0 { return 0.0; }
    let mismatch = shaft_speed.mul_add(effective_ratio, engine_speed);
    if mismatch <= 0.01 || direction != 0 { return 0.0; }
    let share = ((pedal - 0.5) * 2.0).clamp(0.0, 1.0);
    (((share * inertia) * inverse_step) * mismatch) * effective_ratio
}

/// One wheel's coefficients after contact and differential preparation.
#[derive(Clone, Copy, Debug, Default)]
pub struct ShaftWheel {
    pub limit: f32,
    pub stiffness: f32,
    pub limited_stiffness: f32,
}

/// Scalar inputs after a wheel's contact/load and longitudinal force preparation.
#[derive(Clone, Copy, Debug, Default)]
pub struct WheelResponse {
    pub spring: f32,
    pub damper: f32,
    pub inverse_step: f32,
    pub velocity: f32,
    pub radius: f32,
    pub optimum_slip: f32,
    pub friction: f32,
    pub load: f32,
    /// Original signed wheel rotation, negative rolling forward.
    pub spin: f32,
    pub slip: f32,
}

impl WheelResponse {
    /// sub_82565E00, 0x82566388..0x825665D4. A wheel switches between a
    /// spring/friction slope and zero slope at a boundary chosen for this force.
    pub fn prepare(&self, force: f32) -> ShaftWheel {
        let spring_slope = (self.damper.mul_add(self.inverse_step, self.spring)
            * self.radius) * self.radius;
        let mut stiffness = spring_slope;
        let mut spring_limited = true;
        if self.velocity.abs() >= 1.0e-7 {
            let friction_slope = (((((1.0 / self.velocity.abs()) / self.optimum_slip)
                * self.load) * self.radius) * self.radius) * self.friction;
            if spring_slope > friction_slope {
                stiffness = friction_slope;
                spring_limited = false;
            }
        }
        let (limit, limited_stiffness) = if spring_limited {
            let bound = ((self.load * self.inverse_step) * self.friction)
                / (self.spring * self.radius);
            if force < 0.0 {
                if self.spin < -bound { (-bound, stiffness) }
                else if self.spin < bound { (bound, 0.0) }
                else { (1.0e8, stiffness) }
            } else if self.spin > bound { (bound, stiffness) }
            else if self.spin > -bound { (-bound, 0.0) }
            else { (-1.0e9, stiffness) }
        } else {
            let rolling = -(self.velocity / self.radius);
            let a = (1.0 - self.optimum_slip) * rolling;
            let b = (self.optimum_slip + 1.0) * rolling;
            let lo_slip = if self.slip - (-self.optimum_slip) >= 0.0 {
                self.slip
            } else { -self.optimum_slip };
            let clamped_slip = if lo_slip - self.optimum_slip >= 0.0 {
                self.optimum_slip
            } else { lo_slip };
            let previous = (1.0 - clamped_slip) * rolling;
            let nonnegative = rolling - previous >= 0.0;
            let (high, low) = if a > b {
                if nonnegative { (rolling, previous) } else { (previous, rolling) }
            } else if nonnegative { (previous, rolling) } else { (rolling, previous) };
            if force < 0.0 {
                let (entry, exit) = if self.velocity > 0.0 { (low, a) } else { (high, b) };
                if self.spin < entry { (entry, stiffness) }
                else if self.spin < exit { (exit, 0.0) }
                else { (1.0e11, stiffness) }
            } else {
                let (entry, exit) = if self.velocity > 0.0 { (high, b) } else { (low, a) };
                if self.spin > entry { (entry, stiffness) }
                else if self.spin > exit { (exit, 0.0) }
                else { (-1.0e11, stiffness) }
            }
        };
        ShaftWheel { limit, stiffness: if limited_stiffness == 0.0 { stiffness } else { 0.0 },
                     limited_stiffness }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ShaftResult {
    pub speed: f32,
    pub candidate: f32,
    pub stiffness: f32,
    pub residual: f32,
    pub iterations: usize,
}

#[derive(Clone, Copy, Debug)]
pub struct DifferentialResult {
    pub ratio: f32,
    pub reciprocal: f32,
    pub residual: f32,
}

/// Engine/transmission fields consumed by the original connected drivetrain.
/// Engine curves and connection lifecycle are updated outside the shaft solve.
#[derive(Clone, Copy, Debug)]
pub struct EngineDrive {
    pub ratio: f32,
    pub speed: f32,
    pub torque: f32,
    pub inertia: f32,
    pub sync_torque: f32,
    pub idle_speed: f32,
    pub max_speed: f32,
    pub engage_rpm: f32,
    pub cvt: f32,
    pub pedal: f32,
    pub gear_change_inertia: f32,
    pub direction: i8,
    /// Transmission +32 inhibits drive torque while the engine can remain bound.
    pub disconnected: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PreparedWheel {
    pub response: WheelResponse,
    pub contact: bool,
    /// Stored wheel +548, not differential-weighted chassis/surface drag torque.
    pub torque: f32,
    pub brake: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct CoupledResult {
    pub speed: f32,
    pub differential: f32,
    pub engine_speed: Option<f32>,
    pub wheel_speeds: [f32; 4],
}

/// Original scalar drivetrain update with prepared contact/load data.
/// Uses original shaft coordinates and previous wheel torque/slip/velocity.
/// The caller still owns collision, suspension and the order of the full tick.
pub fn update_prepared(tune: &mcla_formats::rage::simtune::DrivetrainTune,
    speed: f32, differential: f32, engine: Option<EngineDrive>,
    dt: f32, wheels: &[PreparedWheel]) -> CoupledResult {
    assert!(!wheels.is_empty() && wheels.len() <= 4);
    let inverse_step = 1.0 / dt;
    let brake_coef = if speed.abs() < 0.001 { tune.brake_static_coef } else { tune.brake_dynamic_coef };
    let mut brakes = tune.ang_drag;
    for wheel in wheels { brakes = wheel.brake.mul_add(brake_coef, brakes); }
    let (mut drive, mut extra) = (0.0, 0.0);
    if let Some(e) = engine.filter(|e| !e.disconnected) {
        let gear = effective_ratio(e.ratio, e.speed, e.torque, e.engage_rpm, e.cvt);
        drive = gear * e.torque;
        extra = gear_change_inertia(gear, e.speed, speed, e.pedal,
                                   e.gear_change_inertia, inverse_step, e.direction);
    }
    let mut force = extra + drive;
    for wheel in wheels { force -= wheel.torque; }
    let braking;
    if speed.abs() < 0.001 {
        if force < 0.0 {
            let adjusted = force + brakes;
            force = if -adjusted >= 0.0 { adjusted } else { 0.0 };
        } else {
            let adjusted = force - brakes;
            force = if -adjusted >= 0.0 { 0.0 } else { adjusted };
        }
        braking = force == 0.0;
    } else {
        let drag = if speed < 0.0 { -brakes } else { brakes };
        braking = drag.abs() >= force.abs();
        force += drag;
    }
    let mut coefficients = [ShaftWheel::default(); 4];
    let mut torques = [0.0; 4];
    for (i, wheel) in wheels.iter().enumerate() {
        torques[i] = wheel.torque;
        coefficients[i] = if wheel.contact && wheel.response.load != 0.0 {
            let mut response = wheel.response;
            response.inverse_step = inverse_step;
            response.prepare(force)
        } else {
            let sign = if force > 0.0 { 1.0 } else if force < 0.0 { -1.0 } else { 0.0 };
            ShaftWheel { limit: sign * -1.0e10, stiffness: 0.0, limited_stiffness: 0.0 }
        };
    }
    let diff = balance_differential(tune, speed, differential, force,
                                   &torques[..wheels.len()], &mut coefficients[..wheels.len()]);
    let mut inertia = tune.inertia;
    if let Some(e) = engine.filter(|e| e.ratio != 0.0) {
        inertia = (e.ratio * e.ratio).mul_add(e.inertia, inertia);
    }
    let solved = solve_shaft(speed, inertia, dt, diff.residual, force, braking,
                             &coefficients[..wheels.len()]);
    let mut speed = solved.speed;
    let engine_speed = engine.map(|e| {
        // The original clamps the shaft to half idle / maximum with the nominal
        // ratio, then slews engine speed toward at least full idle.
        let mut target = -(e.ratio * speed);
        let half_idle = e.idle_speed * 0.5;
        if target < half_idle {
            target = half_idle;
            speed = (-1.0 / e.ratio) * target;
        } else if target > e.max_speed {
            target = e.max_speed;
            speed = (-1.0 / e.ratio) * target;
        }
        if target < e.idle_speed { target = e.idle_speed; }
        super::vehsim::synchronize_engine(e.speed, target, e.sync_torque, e.inertia, dt)
    });
    let mut wheel_speeds = [0.0; 4];
    let pairs = wheels.len() / 2 * 2;
    for (i, spin) in wheel_speeds[..pairs].iter_mut().enumerate() {
        *spin = speed * if i % 2 == 0 { diff.ratio } else { diff.reciprocal };
    }
    if wheels.len() % 2 != 0 { wheel_speeds[wheels.len() - 1] = speed; }
    CoupledResult { speed, differential: diff.ratio, engine_speed, wheel_speeds }
}

/// Original paired-wheel differential block (0x825691DC..0x825693AC).
/// Torque signs and speed are in the original convention; wheel data is rescaled
/// in place before the shaft solve. This is one original physics step's easing.
pub fn balance_differential(tune: &mcla_formats::rage::simtune::DrivetrainTune,
    speed: f32, old_ratio: f32, force: f32, torques: &[f32], wheels: &mut [ShaftWheel]) -> DifferentialResult {
    assert_eq!(torques.len(), wheels.len());
    assert!(wheels.len() <= 4);
    let residual = -force;
    if tune.diff_low_speed_limit <= 1.0 {
        return DifferentialResult { ratio: old_ratio, reciprocal: 1.0 / old_ratio, residual };
    }
    if wheels.len() <= 1 || (speed < 0.001 && speed > -0.001) {
        return DifferentialResult { ratio: 1.0, reciprocal: 1.0, residual };
    }
    let magnitude = speed.abs();
    let limit = if magnitude >= tune.diff_high_limit_speed {
        tune.diff_high_speed_limit
    } else {
        (tune.diff_high_limit_speed - magnitude).mul_add(tune.diff_low_speed_limit,
            tune.diff_high_speed_limit * magnitude) / tune.diff_high_limit_speed
    };
    let minimum = 1.0 / limit;
    let mut stiffness = 0.0;
    let mut torque_delta = 0.0;
    for (w, t) in wheels.chunks_exact(2).zip(torques.chunks_exact(2)) {
        stiffness = (w[1].stiffness + w[0].stiffness) + stiffness;
        torque_delta = (t[0] - t[1]) + torque_delta;
    }
    let mut target = torque_delta / ((tune.inertia + stiffness) * speed) + old_ratio;
    if target > limit { target = limit; }
    else if target < minimum { target = minimum; }
    let ratio = old_ratio.mul_add(9.0, target) * 0.1;
    let reciprocal = 1.0 / ratio;
    let shift = (old_ratio - ratio) * speed;
    // fnmsubs negates the fused result; moving the negation to an input
    // changes the sign of exact zero and loses original scalar bit parity.
    let residual = -shift.mul_add(stiffness, -residual);
    for pair in wheels.chunks_exact_mut(2) {
        pair[0].stiffness *= ratio;
        pair[0].limit *= reciprocal;
        pair[0].limited_stiffness *= ratio;
        pair[1].stiffness *= reciprocal;
        pair[1].limit *= ratio;
        pair[1].limited_stiffness *= reciprocal;
    }
    DifferentialResult { ratio, reciprocal, residual }
}

/// Follow the original piecewise implicit solve as wheel speed limits are crossed.
/// `force` selects the limit direction; `residual` also includes the differential
/// correction. `inertia` includes any engine inertia reflected through the gear.
pub fn solve_shaft(speed: f32, inertia: f32, dt: f32, mut residual: f32,
                   force: f32, braking: bool, wheels: &[ShaftWheel]) -> ShaftResult {
    assert!(wheels.len() <= 4);
    let mut limits = [0.0; 4];
    let mut stiffness = 0.0;
    for (i, wheel) in wheels.iter().enumerate() {
        limits[i] = wheel.limit;
        stiffness += wheel.stiffness;
    }
    let mut anchor = speed;
    let mut iterations = 0;
    let candidate = loop {
        iterations += 1;
        let denominator = dt.mul_add(stiffness, inertia);
        let candidate = (residual / denominator).mul_add(dt, anchor);
        let upper = force < 0.0;
        let mut limit = if upper { 1.0e9 } else { -1.0e10 };
        let mut index = None;
        for (i, &value) in limits[..wheels.len()].iter().enumerate() {
            if (upper && limit > value) || (!upper && limit < value) {
                limit = value;
                index = Some(i);
            }
        }
        if (upper && candidate <= limit) || (!upper && candidate >= limit) {
            break candidate;
        }
        // Finite prepared wheel limits keep the original's sentinel index valid.
        let i = index.expect("shaft candidate crossed an unbounded wheel limit");
        limits[i] = if upper { 1.0e11 } else { -1.0e11 };
        let delta = limit - anchor;
        anchor = limit;
        residual = -delta.mul_add(stiffness, -residual);
        stiffness = (wheels[i].limited_stiffness - wheels[i].stiffness) + stiffness;
        if iterations > wheels.len() {
            break candidate;
        }
    };
    // Original brake dominance flag prevents crossing zero into reverse rotation.
    let stopped = braking && ((candidate < 0.0 && speed > 0.0)
        || (speed < 0.0 && candidate > 0.0));
    ShaftResult { speed: if stopped { 0.0 } else { candidate }, candidate,
                  stiffness, residual, iterations }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_coupled_capture(p: &[f32]) {
        assert_eq!(p.len(), 75);
        let tune = mcla_formats::rage::simtune::DrivetrainTune { inertia: p[0], ang_drag: p[1],
            brake_static_coef: p[2], brake_dynamic_coef: p[3], diff_low_speed_limit: p[4],
            diff_high_speed_limit: p[5], diff_high_limit_speed: p[6] };
        let engine = (p[11] != 0.0).then_some(EngineDrive { ratio: p[12], speed: p[13],
            torque: p[14], inertia: p[15], sync_torque: p[16], idle_speed: p[17], max_speed: p[18],
            engage_rpm: p[19], cvt: p[20], pedal: p[21], gear_change_inertia: p[22],
            direction: p[23] as i8, disconnected: p[11] == 2.0 });
        let n = p[10] as usize;
        let wheels: Vec<_> = (0..n).map(|i| {
            let q = &p[24 + i * 11..];
            let pairs = n / 2 * 2;
            let spin = p[7] * if i < pairs { if i % 2 == 0 { p[8] } else { 1.0 / p[8] } } else { 1.0 };
            PreparedWheel { torque: q[0], brake: q[1], contact: q[2] != 0.0,
                response: WheelResponse { spring: q[3], damper: q[4], velocity: q[5], radius: q[6],
                    optimum_slip: q[7], friction: q[8], load: q[9], slip: q[10], spin,
                    inverse_step: 1.0 / p[9] } }
        }).collect();
        let actual = update_prepared(&tune, p[7], p[8], engine, p[9], &wheels);
        let mut result = vec![actual.speed, actual.differential, actual.engine_speed.unwrap_or(p[13])];
        // Unattached scratch wheels are not touched by the original update.
        for i in 0..4 {
            result.push(if i < n { actual.wheel_speeds[i] } else {
                p[7] * if i < n - n % 2 { if i % 2 == 0 { p[8] } else { 1.0 / p[8] } } else { 1.0 }
            });
        }
        for (a, e) in result.into_iter().zip(p[68..].iter().copied()) {
            assert_eq!(a.to_bits(), e.to_bits(), "{p:?}: {actual:?}");
        }
    }

    #[test]
    fn combined_drive_rpm_clamps_and_brake_stop_match_original_captures() {
        // Unchanged generated scalar update: bound/disconnected engine, four
        // contacts with drive, and an unbound shaft stopped by braking.
        for line in [
            "2,30,2,0.800000012,1.25,1.02999997,200,200,0.899999976,0.0166666675,2,2,20,100,100,0.0599999987,200,100,800,3500,3,0.5,0.5,1,-500,0,1,50000,1000,1,0.326000005,0.140000001,1.85000002,3000,-1,500,0,1,50000,100,1,0.326000005,0.540000021,1.85000002,3000,-1,500,0,1,50000,100,0,0.326000005,0.140000001,1.85000002,0,-1,500,0,0,50000,100,0,0.326000005,0.540000021,1.85000002,6000,-1,-2.5,0.907087326,100,-2.26771832,-2.75607443,200,200",
            "3,30,1,0.800000012,1,1.02999997,200,0.00100000005,1,0.00416666688,4,1,3,100,100,0.200000003,200,100,800,3500,3,1,0.5,0,0,1000,1,50000,100,0,0.326000005,0.540000021,1.85000002,3000,0,500,1000,1,50000,100,0,0.326000005,0.140000001,1.85000002,6000,0,0,0,1,50000,100,0,0.326000005,0.140000001,1.85000002,3000,-0.100000001,-500,0,1,50000,0,30,0.326000005,0.140000001,1.85000002,0,0,-16.6666679,1,100,-16.6666679,-16.6666679,-16.6666679,-16.6666679",
            "2,0,1,0.800000012,1.25,1.02999997,200,-0.00100000005,1,0.0333333351,4,0,3,300,-100,0.0599999987,200,100,800,3500,3,0.5,0,1,-500,1000,0,50000,0,-30,0.326000005,0.140000001,1.85000002,6000,-1,500,1000,0,50000,1000,30,0.326000005,0.140000001,1.85000002,0,0.100000001,-500,1000,0,50000,0,1,0.326000005,0.140000001,1.85000002,3000,-1,0,200,0,50000,0,-1,0.326000005,0.140000001,1.85000002,6000,0,0,1.02499998,300,0,0,0,0",
        ] {
            let p: Vec<f32> = line.split(',').map(|s| s.parse().unwrap()).collect();
            assert_coupled_capture(&p);
        }
    }

    #[test]
    #[ignore = "requires MCLA_COUPLED_REFERENCE from tools/coupled_reference.py"]
    fn prepared_combined_update_matches_original_generated_code() {
        let path = std::env::var("MCLA_COUPLED_REFERENCE").expect("MCLA_COUPLED_REFERENCE");
        let csv = std::fs::read_to_string(path).unwrap();
        let mut count = 0;
        for line in csv.lines().skip(1) {
            let p: Vec<f32> = line.split(',').map(|s| s.parse().unwrap()).collect();
            assert_coupled_capture(&p);
            count += 1;
        }
        assert_eq!(count, 3000);
        println!("{count} prepared combined drivetrain cases match exactly");
    }

    #[test]
    fn differential_redistribution_matches_original_capture() {
        let tune = mcla_formats::rage::simtune::DrivetrainTune { inertia: 2.0,
            diff_low_speed_limit: 1.03, diff_high_speed_limit: 1.03,
            diff_high_limit_speed: 200.0, ..Default::default() };
        let mut wheels = [ShaftWheel { stiffness: 10000.0, limit: 100.0, limited_stiffness: 0.0 },
                          ShaftWheel { stiffness: 100.0, limit: 0.0, limited_stiffness: 10000.0 }];
        let actual = balance_differential(&tune, -10.0, 1.2, -20000.0, &[1000.0, 1000.0], &mut wheels);
        assert_eq!(actual.ratio.to_bits(), 1.18299997_f32.to_bits());
        assert_eq!(actual.reciprocal.to_bits(), 0.845308542_f32.to_bits());
        assert_eq!(actual.residual.to_bits(), 21717.0078_f32.to_bits());
        assert_eq!(wheels[0].stiffness, 11830.0);
        assert_eq!(wheels[0].limit.to_bits(), 84.5308533_f32.to_bits());
        assert_eq!(wheels[1].limited_stiffness.to_bits(), 8453.08496_f32.to_bits());
    }

    #[test]
    #[ignore = "requires MCLA_DIFFERENTIAL_REFERENCE from tools/differential_reference.py"]
    fn differential_matches_original_generated_code() {
        let path = std::env::var("MCLA_DIFFERENTIAL_REFERENCE").expect("MCLA_DIFFERENTIAL_REFERENCE");
        let csv = std::fs::read_to_string(path).unwrap();
        let mut count = 0;
        for line in csv.lines().skip(1) {
            let p: Vec<f32> = line.split(',').map(|s| s.parse().unwrap()).collect();
            assert_eq!(p.len(), 39);
            let n = p[7] as usize;
            let tune = mcla_formats::rage::simtune::DrivetrainTune { inertia: p[0],
                diff_low_speed_limit: p[1], diff_high_speed_limit: p[2],
                diff_high_limit_speed: p[3], ..Default::default() };
            let torques: Vec<_> = (0..n).map(|i| p[8 + i * 4]).collect();
            let mut wheels: Vec<_> = (0..4).map(|i| ShaftWheel { stiffness: p[9 + i * 4],
                limit: p[10 + i * 4], limited_stiffness: p[11 + i * 4] }).collect();
            let actual = balance_differential(&tune, p[4], p[5], p[6], &torques, &mut wheels[..n]);
            let mut outputs = vec![actual.ratio, actual.reciprocal, actual.residual];
            for w in wheels { outputs.extend([w.stiffness, w.limit, w.limited_stiffness]); }
            for (a, e) in outputs.into_iter().zip(p[24..].iter().copied()) {
                assert_eq!(a.to_bits(), e.to_bits(), "{line}: {actual:?}");
            }
            count += 1;
        }
        assert_eq!(count, 6000);
        println!("{count} original differential cases match exactly");
    }

    #[test]
    fn converter_and_inertia_match_captured_original_values() {
        let ratio = effective_ratio(8.0, 100.0, 200.0, 3500.0, 3.0);
        assert_eq!(ratio.to_bits(), 25.4519119_f32.to_bits());
        assert_eq!(effective_ratio(8.0, -100.0, 200.0, 3500.0, 3.0).to_bits(), ratio.to_bits());
        assert_eq!(effective_ratio(8.0, 100.0, 1.0, 3500.0, 3.0), 8.0);
        assert_eq!(effective_ratio(8.0, 100.0, 200.0, 3500.0, 1.0), 8.0);
        assert_eq!(gear_change_inertia(20.0, 800.0, 1.0, 1.0, 0.5, 30.0, 0), 246000.0);
        assert_eq!(gear_change_inertia(20.0, 800.0, 1.0, 1.0, 0.5, 30.0, 1), 0.0);
        assert_eq!(gear_change_inertia(20.0, 100.0, -5.0, 1.0, 0.5, 60.0, 0), 0.0);
    }

    #[test]
    fn wheel_response_boundaries_match_captured_original_values() {
        let fixtures = [
            "5000,100,60,0,0.326000005,0.540000021,1.85000002,3000,-0,0,0,1169.03601,-204.294479,0",
            "5000,0,240,1,0.400000006,0.540000021,0.800000012,1000,-2.5,0.540000021,20000,237.037033,-3.8499999,0",
            "50000,100,60,-60,0.400000006,0.0700000003,0.800000012,6000,300,1,0,0,150,182.857162",
        ];
        for line in fixtures {
            let p: Vec<f32> = line.split(',').map(|s| s.parse().unwrap()).collect();
            let actual = WheelResponse { spring: p[0], damper: p[1], inverse_step: p[2],
                velocity: p[3], radius: p[4], optimum_slip: p[5], friction: p[6],
                load: p[7], spin: p[8], slip: p[9] }.prepare(p[10]);
            for (a, e) in [actual.stiffness, actual.limit, actual.limited_stiffness]
                .into_iter().zip(p[11..14].iter().copied()) {
                assert_eq!(a.to_bits(), e.to_bits(), "{line}: {actual:?}");
            }
        }
    }

    #[test]
    #[ignore = "requires MCLA_DRIVE_REFERENCE from tools/drive_reference.py"]
    fn connected_drive_matches_original_generated_code() {
        let path = std::env::var("MCLA_DRIVE_REFERENCE").expect("MCLA_DRIVE_REFERENCE");
        let csv = std::fs::read_to_string(path).unwrap();
        let mut count = 0;
        for line in csv.lines().skip(1) {
            let p: Vec<f32> = line.split(',').map(|s| s.parse().unwrap()).collect();
            assert_eq!(p.len(), 13);
            let gear = effective_ratio(p[0], p[1], p[2], p[3], p[4]);
            let boost = gear_change_inertia(gear, p[1], p[5], p[6], p[7], p[8], p[9] as i8);
            for (a, e) in [gear, gear * p[2], boost].into_iter().zip(p[10..13].iter().copied()) {
                assert_eq!(a.to_bits(), e.to_bits(), "{line}: gear {gear}, boost {boost}");
            }
            count += 1;
        }
        assert_eq!(count, 1600);
        println!("{count} original drive cases match exactly");
    }

    #[test]
    #[ignore = "requires MCLA_WHEEL_RESPONSE_REFERENCE from tools/wheel_response_reference.py"]
    fn wheel_response_matches_original_generated_code() {
        let path = std::env::var("MCLA_WHEEL_RESPONSE_REFERENCE").expect("MCLA_WHEEL_RESPONSE_REFERENCE");
        let csv = std::fs::read_to_string(path).unwrap();
        let mut count = 0;
        let mut current = 0;
        let mut after = 0;
        for line in csv.lines().skip(1) {
            let p: Vec<f32> = line.split(',').map(|s| s.parse().unwrap()).collect();
            assert_eq!(p.len(), 14);
            let actual = WheelResponse { spring: p[0], damper: p[1], inverse_step: p[2],
                velocity: p[3], radius: p[4], optimum_slip: p[5], friction: p[6],
                load: p[7], spin: p[8], slip: p[9] }.prepare(p[10]);
            for (a, e) in [actual.stiffness, actual.limit, actual.limited_stiffness]
                .into_iter().zip(p[11..14].iter().copied()) {
                assert_eq!(a.to_bits(), e.to_bits(), "{line}: actual {actual:?}");
            }
            count += 1;
            current += usize::from(actual.stiffness != 0.0);
            after += usize::from(actual.limited_stiffness != 0.0);
        }
        assert_eq!(count, 12000);
        assert_eq!((current, after), (6335, 3644));
        println!("{count} wheel responses match original exactly");
    }

    #[test]
    fn wheel_limit_transitions_match_captured_original_values() {
        // Golden captures from shaft_reference.py, retained for portable tests.
        let positive = [ShaftWheel { limit: -118.938187, stiffness: 1000.0, limited_stiffness: 1000.0 },
                        ShaftWheel { limit: -119.938187, stiffness: 1000.0, limited_stiffness: 0.0 }];
        let a = solve_shaft(-119.938187, 0.5, 1.0 / 240.0, 20000.0, -20000.0, false, &positive);
        assert_eq!(a.speed.to_bits(), (-101.9739_f32).to_bits());
        assert_eq!(a.stiffness, 1000.0);
        assert_eq!(a.residual, 19000.0);
        assert_eq!(a.iterations, 3);

        let negative = [ShaftWheel { limit: 0.0273672361, stiffness: 10000.0, limited_stiffness: 5000.0 },
                        ShaftWheel { limit: -0.962632775, stiffness: 10000.0, limited_stiffness: 1000.0 },
                        ShaftWheel { limit: 0.0373672359, stiffness: 10.0, limited_stiffness: 0.0 },
                        ShaftWheel { limit: -0.962632775, stiffness: 10.0, limited_stiffness: 0.0 }];
        let b = solve_shaft(0.0373672359, 20.0, 1.0 / 60.0, -20000.0, 20000.0, false, &negative);
        assert_eq!(b.speed.to_bits(), (-1.64874399_f32).to_bits());
        assert_eq!(b.stiffness, 6000.0);
        assert_eq!(b.residual.to_bits(), (-4940.00049_f32).to_bits());
        assert_eq!(b.iterations, 5);

        let braking = [ShaftWheel { limit: 1.0530082, stiffness: 10.0, limited_stiffness: 5.0 },
                       ShaftWheel { limit: -48.9369926, stiffness: 10000.0, limited_stiffness: 1000.0 }];
        let c = solve_shaft(1.06300819, 2.0, 1.0 / 240.0, -20000.0, 20000.0, true, &braking);
        assert_eq!(c.speed, 0.0);
        assert_eq!(c.candidate.to_bits(), (-0.844931722_f32).to_bits());
        assert_eq!(c.iterations, 2);
    }

    #[test]
    fn braking_stops_at_zero_while_drive_can_reverse_the_shaft() {
        let drive = solve_shaft(1.0, 1.0, 1.0 / 60.0, -120.0, 120.0, false, &[]);
        let brake = solve_shaft(1.0, 1.0, 1.0 / 60.0, -120.0, 120.0, true, &[]);
        assert!(drive.speed < 0.0);
        assert_eq!(brake.speed, 0.0);
        assert_eq!(drive.candidate.to_bits(), brake.candidate.to_bits());
    }

    #[test]
    #[ignore = "requires MCLA_SHAFT_REFERENCE from tools/shaft_reference.py"]
    fn prepared_shaft_solver_matches_original_generated_code() {
        let path = std::env::var("MCLA_SHAFT_REFERENCE").expect("MCLA_SHAFT_REFERENCE");
        let csv = std::fs::read_to_string(path).unwrap();
        let mut count = 0;
        let mut crossing_cases = 0;
        let mut brake_stops = 0;
        for line in csv.lines().skip(1) {
            let p: Vec<f32> = line.split(',').map(|s| s.parse().unwrap()).collect();
            assert_eq!(p.len(), 24);
            let n = p[6] as usize;
            let wheels: Vec<_> = (0..n).map(|i| ShaftWheel {
                limit: p[7 + i * 3], stiffness: p[8 + i * 3],
                limited_stiffness: p[9 + i * 3],
            }).collect();
            let actual = solve_shaft(p[0], p[1], p[2], p[3], p[4], p[5] != 0.0, &wheels);
            for (a, e) in [actual.speed, actual.candidate, actual.stiffness, actual.residual]
                .into_iter().zip(p[19..23].iter().copied()) {
                assert_eq!(a.to_bits(), e.to_bits(), "{line}: actual {actual:?}");
            }
            assert_eq!(actual.iterations, p[23] as usize, "{line}");
            crossing_cases += usize::from(actual.iterations > 1);
            brake_stops += usize::from(actual.speed == 0.0 && actual.candidate != 0.0);
            count += 1;
        }
        assert_eq!(count, 8000);
        assert!(crossing_cases > 1000, "only {crossing_cases} crossing cases");
        assert!(brake_stops > 100, "only {brake_stops} brake stops");
        println!("{count} original shaft cases match exactly; {crossing_cases} cross wheel limits; {brake_stops} brake stops");
    }
}
