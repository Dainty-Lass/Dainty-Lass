use std::process::ExitCode;

use isles_db::{Command, USAGE, parse_args, run_migrate, run_verify};

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = match parse_args(&args) {
        Ok(cmd) => cmd,
        Err(err) => {
            eprintln!("error: {err}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };

    match cmd {
        Command::Help => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        Command::Migrate { opts, json } => match run_migrate(&opts).await {
            Ok(report) => {
                if json {
                    if let Ok(s) = serde_json::to_string_pretty(&report) {
                        println!("{s}");
                    }
                } else {
                    let mode = if report.dry_run { "dry-run" } else { "migrated" };
                    println!("isles-db migrate ({mode}):");
                    for (table, count) in &report.counts {
                        if *count > 0 {
                            println!("  {table}: {count}");
                        }
                    }
                    println!(
                        "  rb11 transforms: approvals={}, inventory_arrays={}, starter_instances={}, loadout_slots={}",
                        report.rb11_launcher_approvals_backfilled,
                        report.rb11_inventory_arrays_transformed,
                        report.rb11_starter_instances_repaired,
                        report.rb11_loadout_slots_reconciled
                    );
                    if report.skipped_invalid > 0 {
                        println!("  skipped_invalid: {}", report.skipped_invalid);
                    }
                }
                ExitCode::SUCCESS
            }
            Err(err) => {
                eprintln!("isles-db migrate failed: {err}");
                ExitCode::FAILURE
            }
        },
        Command::Verify { opts, json } => match run_verify(&opts).await {
            Ok(report) => {
                if json {
                    if let Ok(s) = serde_json::to_string_pretty(&report) {
                        println!("{s}");
                    }
                } else {
                    println!(
                        "isles-db verify: OK ({} collections verified)",
                        report.collections.len()
                    );
                    for col in &report.collections {
                        println!(
                            "  [OK] {} (area={}, count={}, sha256={})",
                            col.name,
                            col.area.as_str(),
                            col.actual_count,
                            &col.actual_sha256[..16]
                        );
                    }
                }
                ExitCode::SUCCESS
            }
            Err(err) => {
                eprintln!("isles-db verify failed: {err}");
                ExitCode::FAILURE
            }
        },
    }
}
