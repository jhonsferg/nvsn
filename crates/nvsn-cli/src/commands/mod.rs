//! Implementation of each command. Each function receives the `Ctx` and writes its
//! output through `ctx.out` (human or JSON).

pub mod activate;
pub mod cache;
pub mod doctor;
pub mod exec;
pub mod implode;
pub mod install;
pub mod list;
pub mod local;
pub mod outdated;
pub mod packages;
pub mod path;
pub mod prune;
pub mod run;
pub mod self_uninstall;
pub mod self_update;
pub mod settings;
pub mod shell;
pub mod state;
pub mod update_check;
