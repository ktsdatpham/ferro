//! `ferro-cli bench`: time each call of a script over N runs, each run in a
//! fresh session (with `--in` reopened), for performance budgets (D-62).
//! Times are wall-clock milliseconds; the opening of `--in` is not timed.

use crate::script::{self, Runner, Step};
use crate::CliError;
use serde_json::{json, Value};
use std::path::Path;
use std::process::ExitCode;
use std::time::Instant;

/// Median of a non-empty slice (mean of the middle two for even lengths);
/// 0 for an empty one.
pub fn median(samples: &[f64]) -> f64 {
    let mut v = samples.to_vec();
    v.sort_by(f64::total_cmp);
    let n = v.len();
    match n {
        0 => 0.0,
        _ if n % 2 == 1 => v[n / 2],
        _ => (v[n / 2 - 1] + v[n / 2]) / 2.0,
    }
}

fn stats(samples: &[f64]) -> Value {
    let max = samples.iter().copied().fold(0.0, f64::max);
    let r = |x: f64| (x * 1000.0).round() / 1000.0;
    json!({"median_ms": r(median(samples)), "max_ms": r(max)})
}

/// Run the script `repeat` times; prints
/// `{"repeat", "calls": [{step, tool, median_ms, max_ms}], "total": {…}}`.
/// A failing call stops the bench: its line is printed and the exit code is 1.
pub fn bench(steps: &[Step], input: Option<&Path>, repeat: usize) -> Result<ExitCode, CliError> {
    let mut times: Vec<Vec<f64>> = vec![Vec::with_capacity(repeat); steps.len()];
    let mut totals = Vec::with_capacity(repeat);
    for _ in 0..repeat {
        let mut server = script::session(input)?;
        let mut runner = Runner::default();
        let mut total = 0.0;
        for (i, step) in steps.iter().enumerate() {
            let t = Instant::now();
            let r = runner.step(&mut server, step);
            let ms = t.elapsed().as_secs_f64() * 1000.0;
            if r.is_err() {
                crate::outln!("{}", script::line(Some(i), &step.tool, &r));
                return Ok(ExitCode::from(1));
            }
            times[i].push(ms);
            total += ms;
        }
        totals.push(total);
    }
    let calls: Vec<Value> = steps
        .iter()
        .zip(&times)
        .enumerate()
        .map(|(i, (s, t))| {
            let mut v = stats(t);
            v["step"] = json!(i);
            v["tool"] = json!(s.tool);
            v
        })
        .collect();
    let out = json!({"repeat": repeat, "calls": calls, "total": stats(&totals)});
    crate::outln!("{}", crate::pretty(&out));
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn median_of_odd_even_and_empty() {
        assert_eq!(median(&[3.0, 1.0, 2.0]), 2.0);
        assert_eq!(median(&[4.0, 1.0, 2.0, 3.0]), 2.5);
        assert_eq!(median(&[]), 0.0);
        assert_eq!(median(&[7.5]), 7.5);
    }
}
