
pub fn human_bytes(size: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut s = size as f64;
    let mut idx = 0usize;
    while s >= 1024.0 && idx < UNITS.len() - 1 {
        s /= 1024.0;
        idx += 1;
    }
    if idx == 0 {
        format!("{} {}", size, UNITS[idx])
    } else {
        format!("{:.1} {}", s, UNITS[idx])
    }
}
