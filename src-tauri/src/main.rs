// The GUI must build as a windowed app; this attribute only takes effect in
// the crate that defines `fn main`, so it lives here rather than in lib.rs.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() { easytier_win_client_lib::run(); }
