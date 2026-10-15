//! `convert <save> <base.json> <race table or -> <out.json>`: the same job as `tools/import_mc4_save.py`, for comparing the two.
use serde_json::Value;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() < 5 {
        eprintln!("usage: convert <mc4.sav> <base.json> <race table | -> <out.json>");
        std::process::exit(2);
    }
    let sav = std::fs::read(&a[1]).expect("read save");
    let base: Value = serde_json::from_slice(&std::fs::read(&a[2]).expect("read base")).expect("base json");
    let races = if a[3] == "-" { vec![] } else { mcla_savedecode::race_table(&std::fs::read(&a[3]).expect("race table")) };
    let Value::Object(base) = base else { panic!("base is not an object") };
    match mcla_savedecode::convert(&sav, base, &races) {
        Ok(c) => {
            std::fs::write(&a[4], serde_json::to_vec(&Value::Object(c.save)).unwrap()).unwrap();
            println!("{:?}", c.summary);
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}
