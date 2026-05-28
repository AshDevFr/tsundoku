use std::path::PathBuf;

pub fn run(output: PathBuf, force: bool) -> anyhow::Result<()> {
    td_config::write_starter(&output, force)?;
    println!("Starter config written to {}", output.display());
    println!(
        "Edit it before exposing the API: set `auth.admin_token` and add at least one [[sources]] entry."
    );
    Ok(())
}
