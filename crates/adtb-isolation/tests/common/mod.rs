use adtb_isolation::{Anomaly, Txn};

pub fn kind(a: &Anomaly) -> &'static str {
    match a {
        Anomaly::G0 { .. } => "G0",
        Anomaly::G1a { .. } => "G1a",
        Anomaly::G1b { .. } => "G1b",
        Anomaly::G1c { .. } => "G1c",
        Anomaly::GSingle { .. } => "G-single",
        Anomaly::G2 { .. } => "G2",
        Anomaly::Internal { .. } => "internal",
        Anomaly::Garbage { .. } => "garbage",
        Anomaly::Duplicate { .. } => "duplicate",
        Anomaly::IncompatibleOrder { .. } => "incompatible order",
    }
}

pub fn kinds(history: &[Txn]) -> Vec<&'static str> {
    adtb_isolation::check(history)
        .expect("well formed")
        .iter()
        .map(kind)
        .collect()
}
