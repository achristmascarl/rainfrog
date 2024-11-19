use anyhow::Result;
use vergen_git2::{BuildBuilder, CargoBuilder, Emitter, Git2Builder, RustcBuilder, SysinfoBuilder};

pub fn main() -> Result<()> {
  Emitter::default()
    .add_instructions(&BuildBuilder::all_build()?)?
    .add_instructions(&CargoBuilder::all_cargo()?)?
    .add_instructions(&Git2Builder::all_git()?)?
    .add_instructions(&RustcBuilder::all_rustc()?)?
    .add_instructions(&SysinfoBuilder::all_sysinfo()?)?
    .emit()
}
