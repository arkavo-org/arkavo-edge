use std::io::BufRead;

use arkavo_routines::evaluation::{Measurement, compare};

fn read(path: &str) -> Result<Vec<Measurement>, Box<dyn std::error::Error>> {
    let file = std::fs::File::open(path)?;
    std::io::BufReader::new(file)
        .lines()
        .map(|line| {
            let row = line?;
            Ok(serde_json::from_str(&row)?)
        })
        .collect()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 2 {
        return Err("Usage: routine-evaluate BASELINE.jsonl CANDIDATE.jsonl".into());
    }
    let report = compare(&read(&args[0])?, &read(&args[1])?)?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    if !report.passes_pilot_gate {
        std::process::exit(2);
    }
    Ok(())
}
