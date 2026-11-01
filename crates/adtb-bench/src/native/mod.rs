mod counter;
mod driver;
mod engine;
#[cfg(test)]
mod history_fixture;
mod model;
mod sqlite;

include!(concat!(env!("OUT_DIR"), "/baseline.rs"));

pub fn run(args: Vec<String>) -> Result<String, String> {
    if args.len() != 5 {
        return Err(
            "usage: adtb-bench (adtb|sqlite) (counter|counter-smoke|lifecycle) NEW-DIRECTORY N WRITERS".into(),
        );
    }
    let peer = match args[0].as_str() {
        "adtb" => driver::Peer::Adtb,
        "sqlite" => driver::Peer::Sqlite,
        _ => return Err("peer must be adtb or sqlite".into()),
    };
    let lifecycle = match args[1].as_str() {
        "counter" | "counter-smoke" => false,
        "lifecycle" => true,
        _ => return Err("workload must be counter or lifecycle".into()),
    };
    if args[1] == "counter-smoke" && args[3] != "4" {
        return Err("counter-smoke requires N=4".into());
    }
    driver::correctness(
        peer,
        (&args[2]).into(),
        args[3].parse::<i64>().map_err(|e| e.to_string())?,
        args[4].parse::<usize>().map_err(|e| e.to_string())?,
        lifecycle,
        if args[1] == "counter-smoke" {
            8
        } else {
            20_000
        },
    )
}
