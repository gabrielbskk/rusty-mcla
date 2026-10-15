//! Original engine's twelve-node Catmull-Rom curve, with prepared coefficients.

#[derive(Clone, Debug)]
pub(super) struct Curve {
    idle: f32,
    max: f32,
    first: f32,
    step: f32,
    inverse_step: f32,
    pub(super) coefficients: [[f32; 4]; 12],
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_extrapolation_and_peak_match_original_vantage_capture() {
        let torque = Curve::new(&[170.0, 199.0, 232.0, 261.0, 286.0, 302.0, 312.0, 308.0, 254.0, 0.0],
            104.719757, 816.814087, false);
        assert_eq!(torque.coefficients[0], [2.0, -2.0, 29.0, 170.0]);
        assert_eq!(torque.evaluate(0.0).to_bits(), 164.809082_f32.to_bits());
        assert_eq!(torque.evaluate(816.814087).to_bits(), 2.17708985e-05_f32.to_bits());
        let (speed, sync_torque) = torque.peak();
        assert_eq!(speed.to_bits(), 686.42572_f32.to_bits());
        assert_eq!(sync_torque.to_bits(), 295.305237_f32.to_bits());
        let idle = Curve::new(&[100.0, 0.0, -100.0, -200.0, -300.0, -400.0, -500.0, -600.0, -700.0, -800.0],
            104.719757, 816.814087, true);
        assert_eq!(idle.coefficients[0], [0.0, 0.0, -100.0, 100.0]);
        assert_eq!(idle.evaluate(0.0).to_bits(), 117.647057_f32.to_bits());
    }
}

impl Curve {
    /// Torque pads with zero, idle torque with its last authored control value.
    pub(super) fn new(values: &[f32], idle: f32, max: f32, idle_curve: bool) -> Self {
        assert!((3..=12).contains(&values.len()));
        let step = (max - idle) / (values.len() as f32 - 2.0);
        let pad = if idle_curve { *values.last().unwrap() } else { 0.0 };
        let mut nodes = [pad; 12];
        nodes[..values.len()].copy_from_slice(values);
        let mut coefficients = [[0.0; 4]; 12];
        // Original zero endpoint tangents extrapolate the missing control node.
        let mut prior = (nodes[0] - nodes[1]) * 2.0 + nodes[1];
        let (mut current, mut next) = (nodes[0], nodes[1]);
        const MATRIX: [[f32; 4]; 4] = [
            [-0.5, 1.5, -1.5, 0.5], [1.0, -2.5, 2.0, -0.5],
            [-0.5, 0.0, 0.5, 0.0], [0.0, 1.0, 0.0, 0.0],
        ];
        for (i, c) in coefficients.iter_mut().enumerate() {
            let fourth = if i == 10 { (nodes[11] - nodes[10]) * 2.0 + nodes[10] }
                         else { nodes[(i + 2) % 12] };
            for (result, row) in c.iter_mut().zip(MATRIX) {
                // Generated VMX multiply/add instructions use distinct rounding;
                // replacing this sequence with fused mul_add changes coefficients.
                let first = current * row[1] + prior * row[0];
                let second = next * row[2] + first;
                *result = fourth * row[3] + second;
            }
            prior = current;
            current = next;
            next = fourth;
        }
        Self { idle, max, first: idle - step, step, inverse_step: 1.0 / step, coefficients }
    }

    pub(super) fn evaluate(&self, speed: f32) -> f32 {
        let distance = speed - self.first;
        let segment = ((distance * self.inverse_step) as i32).clamp(0, 11) as usize;
        // Clamp the index, not the position/fraction: original curve extrapolates.
        let fraction = -(segment as f32).mul_add(self.step, -distance) * self.inverse_step;
        let square = fraction * fraction;
        let cube = square * fraction;
        let [a, b, c, d] = self.coefficients[segment];
        // Native generated vmsum4fp sums the two pairs in this order.
        (d + c * fraction) + (b * square + a * cube)
    }

    /// vehEngine::Init compares midpoint power with lower-bound power, in
    /// angular speed, until the bracket is at most one radian per second wide.
    pub(super) fn peak(&self) -> (f32, f32) {
        let (mut low, mut high) = (self.idle, self.max);
        while high - low > 1.0 {
            let middle = (high + low) * 0.5;
            let mid_power = (self.evaluate(middle) * middle) * 0.001_818_181_8;
            let low_power = (self.evaluate(low) * low) * 0.001_818_181_8;
            if mid_power > low_power { low = middle; }
            else { high = middle; }
        }
        let speed = (high + low) * 0.5;
        (speed, self.evaluate(speed))
    }
}
