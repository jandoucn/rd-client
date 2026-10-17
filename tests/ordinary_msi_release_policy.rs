#![cfg(t02_release_policy)]
use std::{cell::Cell, path::Path};
thread_local! {
    static PENDING: Cell<bool> = const { Cell::new(false) };
    static QUALIFIED: Cell<usize> = const { Cell::new(0) };
    static OLD: Cell<usize> = const { Cell::new(0) };
    static NEW: Cell<usize> = const { Cell::new(0) };
}
const ROOT: &str = "fixture-only";
fn install_allowed() -> Result<(), String> { Ok(()) }
fn receipt_path_at(_: &Path) -> Result<bool, String> {
    QUALIFIED.with(|n| n.set(n.get()+1));
    Ok(true)
}
fn refuse_pending_at(_: &Path) -> Result<(), String> {
    if PENDING.with(Cell::get) { Err("ordinary prior installation requires reconciliation".into()) } else { Ok(()) }
}
include!(concat!(env!("OUT_DIR"), "/policy.rs"));
include!(concat!(env!("OUT_DIR"), "/disk.rs"));
fn route(pending: bool) -> Result<(), String> {
    PENDING.with(|v|v.set(pending));
    for counter in [&QUALIFIED,&OLD,&NEW] { counter.with(|v|v.set(0)); }
    if use_receipt_path()? { NEW.with(|v|v.set(v.get()+1)); }
    else { OLD.with(|v|v.set(v.get()+1)); }
    Ok(())
}
#[test]
fn default_disabled_uses_old_once_without_qualification_or_intent() {
    assert_eq!(route(false), Ok(()));
    assert_eq!((OLD.with(Cell::get),NEW.with(Cell::get),QUALIFIED.with(Cell::get)),(1,0,0));
}
#[test]
fn active_pending_refuses_both_install_paths() {
    assert_eq!(route(true),Err("ordinary prior installation requires reconciliation".into()));
    assert_eq!((OLD.with(Cell::get),NEW.with(Cell::get),QUALIFIED.with(Cell::get)),(0,0,0));
}
