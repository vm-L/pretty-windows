#![allow(non_snake_case)]

//! Lightweight XAML Visual Tree diagnostics TAP helper (`PrettyWindowsTap.dll`)
//! for safely toggling the Win11 XAML taskbar background opacity on the UI thread.

pub mod tree_match;

#[cfg(windows)]
mod win;
