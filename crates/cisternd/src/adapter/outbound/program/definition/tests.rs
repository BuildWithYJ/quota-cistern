use super::*;

/// The file travels in the binary, so a build that broke it would fail every task.
#[test]
fn the_definition_that_ships_is_readable() {
    let claude = Definition::of("claude", None).unwrap();
    assert_eq!(claude.program, "claude");
    assert_eq!(claude.answer.reader, Reader::LastJsonLine);
    assert!(claude.goal.starts_with("/goal "), "{}", claude.goal);
}

/// A task that named no model must not hand the program a flag with nothing after it,
/// which is why the arguments are written in groups.
#[test]
fn the_definition_that_ships_holds_the_places_it_must() {
    let claude = Definition::of("claude", None).unwrap();
    let written: String = claude.args.iter().flatten().cloned().collect();
    for place in ["{goal}", "{instruction}", "{model}", "{turns}", "{spend}"] {
        assert!(written.contains(place), "{place} is not in claude.toml");
    }
}

#[test]
fn a_file_the_user_placed_is_read_instead_of_the_one_that_ships() {
    let theirs = SHIPPED[0]
        .1
        .replace(r#"program = "claude""#, r#"program = "elsewhere""#);
    let read = Definition::of("claude", Some(&theirs)).unwrap();
    assert_eq!(read.program, "elsewhere");
}

#[test]
fn a_name_nobody_defined_is_refused() {
    let refused = Definition::of("codex", None).unwrap_err();
    assert!(refused.reason.contains("codex"), "{}", refused.reason);
}

fn some(s: &str) -> Option<OsString> {
    Some(OsString::from(s))
}

#[test]
fn the_configuration_home_is_where_a_user_puts_one() {
    assert_eq!(
        placed_in(some("/x/.config"), some("/home/a")),
        Some(PathBuf::from("/x/.config/cistern/vendors"))
    );
    assert_eq!(
        placed_in(None, some("/home/a")),
        Some(PathBuf::from("/home/a/.config/cistern/vendors"))
    );
    assert_eq!(placed_in(None, None), None);
}

/// The specification holds that a path in one of these has to be absolute and that anything
/// else is to be ignored. A variable taken at its word would have the daemon read definitions
/// from whatever directory it was started in.
#[test]
fn a_variable_that_is_not_an_absolute_path_is_passed_over() {
    assert_eq!(
        placed_in(some(""), some("/home/a")),
        Some(PathBuf::from("/home/a/.config/cistern/vendors"))
    );
    assert_eq!(
        placed_in(some(".config"), some("/home/a")),
        Some(PathBuf::from("/home/a/.config/cistern/vendors"))
    );
    assert_eq!(placed_in(some(""), some("")), None);
}

/// A file naming a field this does not have is a file written against another version.
#[test]
fn a_field_the_format_does_not_have_fails() {
    let odd = format!("colour = \"red\"\n{}", SHIPPED[0].1);
    assert!(Definition::parse("theirs.toml", &odd).is_err());
}
