//! Terminal UI shell for lmml.
//!
//! The library exposes testable app state, action dispatch, event-loop, tab, and
//! widget modules. The binary entry point only handles terminal setup.

pub mod action;
pub mod app;
pub mod event_loop;
pub mod footer;
pub mod runtime_cli;
pub mod tabs;
pub mod widgets;
