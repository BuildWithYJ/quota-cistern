use std::ffi::OsString;

use super::*;

fn set(dir: &str) -> Option<OsString> {
    Some(OsString::from(dir))
}

#[test]
fn what_the_core_writes_goes_under_the_state_home() {
    assert_eq!(
        state_home(set("/state"), set("/home/someone")),
        Some(PathBuf::from("/state"))
    );
}

/// The specification's own default, which is where a machine that sets nothing ends up.
#[test]
fn a_home_with_no_state_home_falls_back_to_the_place_the_specification_names() {
    assert_eq!(
        state_home(None, set("/home/someone")),
        Some(PathBuf::from("/home/someone/.local/state"))
    );
}

#[test]
fn nowhere_to_write_is_nothing_rather_than_a_guess() {
    assert_eq!(state_home(None, None), None);
}

/// The specification holds that a path in one of these has to be absolute and that anything
/// else is to be ignored. A variable taken at its word would put the file under whatever
/// directory the command was run from.
#[test]
fn a_variable_that_is_not_an_absolute_path_is_passed_over() {
    assert_eq!(
        state_home(set(""), set("/home/someone")),
        Some(PathBuf::from("/home/someone/.local/state"))
    );
    assert_eq!(
        state_home(set("state"), set("/home/someone")),
        Some(PathBuf::from("/home/someone/.local/state"))
    );
    assert_eq!(state_home(set(""), set("")), None);
    assert_eq!(state_home(None, set("home/someone")), None);
}

/// Section 1 gives a socket with no core behind it these two.
#[test]
fn nothing_listening_is_told_apart_from_something_going_wrong() {
    assert!(nobody_listening(&io::Error::new(
        io::ErrorKind::ConnectionRefused,
        "refused"
    )));
    assert!(nobody_listening(&io::Error::new(
        io::ErrorKind::NotFound,
        "no such file"
    )));
    assert!(!nobody_listening(&io::Error::new(
        io::ErrorKind::PermissionDenied,
        "denied"
    )));
    assert!(!nobody_listening(&io::Error::other("something else")));
}

/// Whatever went wrong starting a core is reported as a failure to reach one, since reaching
/// one is what the caller asked for.
#[test]
fn giving_up_reads_as_a_failure_to_reach_the_core() {
    let e = gave_up("nothing here");
    assert_eq!(e.kind(), io::ErrorKind::Other);
    assert_eq!(e.to_string(), "nothing here");
}
