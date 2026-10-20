use moli_core::AccessWindow;

#[test]
fn a_window_includes_its_start_and_excludes_its_end() {
    let window = AccessWindow::new(100, 200).unwrap();
    assert_eq!(window.start(), 100);
    assert_eq!(window.end(), 200);
    assert!(!window.allows(99));
    assert!(window.allows(100));
    assert!(window.allows(199));
    assert!(!window.allows(200));
}

#[test]
fn empty_and_reversed_windows_cannot_be_constructed() {
    assert!(AccessWindow::new(100, 100).is_none());
    assert!(AccessWindow::new(200, 100).is_none());
}

#[test]
fn checking_the_largest_timestamp_never_adds_to_it() {
    let window = AccessWindow::new(u64::MAX - 1, u64::MAX).unwrap();
    assert!(!window.allows(u64::MAX - 2));
    assert!(window.allows(u64::MAX - 1));
    assert!(!window.allows(u64::MAX));
}
