//! Pure logic for `nightlightd`: colour temperature, solar elevation, and
//! timezone lookup.
//!
//! This crate has no dependencies and never touches the screen, the X server,
//! or D-Bus. Everything here is meant to be testable without a display.
//!
//! The implementations land in milestone M1 (issues #4–#9). This file is an
//! intentionally empty skeleton for M0.
