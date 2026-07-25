use anyhow::Result;
use vergen_git2::{Build, Cargo, Emitter, Git2, Rustc, Sysinfo};

pub fn main() -> Result<()> {
  Emitter::default()
    .add_instructions(&Build::all_build())?
    .add_instructions(&Cargo::all_cargo())?
    .add_instructions(&Git2::all_git())?
    .add_instructions(&Rustc::all_rustc())?
    .add_instructions(&Sysinfo::all_sysinfo())?
    .emit()
}
