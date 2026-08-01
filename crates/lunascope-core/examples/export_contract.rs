use std::{env, fs, path::PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("crate must live under workspace/crates")
        .to_path_buf();
    let output = workspace.join("packages/runtime-contract");
    fs::create_dir_all(output.join("src"))?;
    fs::write(
        output.join("src/types.generated.ts"),
        lunascope_core::typescript_contract(),
    )?;
    fs::write(
        output.join("runtime-event.schema.json"),
        lunascope_core::event_schema()?,
    )?;
    println!("exported runtime contract to {}", output.display());
    Ok(())
}
