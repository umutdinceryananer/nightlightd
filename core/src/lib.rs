//! Pure logic for `nightlightd`: colour temperature, solar elevation, and
//! timezone lookup.
//!
//! This crate has no dependencies and never touches the screen, the X server,
//! or D-Bus. Everything here is testable without a display.
//!
//! The implementations are milestone M1 (issues #4-#9), added one module at a
//! time.

pub mod color;
