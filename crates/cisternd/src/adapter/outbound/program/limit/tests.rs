use tempfile::TempDir;

use super::*;

fn claude() -> Definition {
    Definition::of("claude", None).unwrap()
}

#[test]
fn the_definition_that_ships_says_how_to_ask() {
    let held = TempDir::new().unwrap();
    let asking = ProgramLimit::at(claude(), held.path().to_path_buf());
    assert_eq!(asking.definition.limit.reader, LimitReader::StatusLine);
    assert!(asking.definition.limit.settings.contains("{script}"));
}

#[test]
fn the_control_characters_a_terminal_writes_are_not_part_of_what_it_said() {
    let written = b"\x1b[1mYes, I \x1b[32mtrust\x1b[0m this folder\x1b[0m";
    assert_eq!(plainly(written), "yes,itrustthisfolder");
}

#[test]
fn the_figure_is_read_by_the_paths_the_definition_names() {
    let held = TempDir::new().unwrap();
    let asking = ProgramLimit::at(claude(), held.path().to_path_buf());
    let line = serde_json::json!({
        "rate_limits": {
            "five_hour": { "used_percentage": 7.000000000000001, "resets_at": 1786285800u64 },
            "seven_day": { "used_percentage": 44, "resets_at": 1786316400u64 }
        }
    });
    assert_eq!(
        asking.reading_in(&line),
        Some(Reading {
            used: "700".to_owned(),
            resets_at: "1786285800".to_owned()
        })
    );
}

#[test]
fn a_status_line_that_says_nothing_about_the_limit_is_not_a_reading() {
    let held = TempDir::new().unwrap();
    let asking = ProgramLimit::at(claude(), held.path().to_path_buf());
    assert_eq!(asking.reading_in(&serde_json::json!({ "cost": {} })), None);
}

/// Runs the vendor.
/// Not part of `cargo test`, since it costs a turn.
#[test]
#[ignore = "reaches the vendor"]
fn the_vendor_says_where_its_limit_stands() {
    let held = TempDir::new().unwrap();
    let reading = ProgramLimit::at(claude(), held.path().to_path_buf())
        .read()
        .unwrap();

    // Hundredths of a percent, so a full limit reads as ten thousand.
    let used: u64 = reading.used.parse().unwrap();
    assert!(used <= 10_000, "{used}");
    assert!(reading.resets_at.parse::<u64>().unwrap() > 0);
    println!(
        "used {}.{:02}%, resets at {}",
        used / 100,
        used % 100,
        reading.resets_at
    );
}
