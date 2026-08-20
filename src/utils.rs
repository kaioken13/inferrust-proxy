use crate::AppState;

pub fn get_p95_timeout(state: &AppState) -> u64 {
    let latencies = state.latencies.read().unwrap();
    if latencies.len() < 10 {
        return 2500;
    }

    let mut sorted = latencies.iter().cloned().collect::<Vec<_>>();
    sorted.sort_unstable();

    let index = (sorted.len() as f64 * 0.95) as usize;
    let p95 = sorted[index];

    p95.clamp(500, 5000)
}
