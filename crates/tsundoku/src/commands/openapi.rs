use std::path::Path;

use utoipa::OpenApi;

use crate::api::docs::ApiDoc;

pub fn run(output: &Path) -> anyhow::Result<()> {
    let spec = ApiDoc::openapi();
    if let Some(parent) = output.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(output, spec.to_pretty_json()?)?;
    println!("OpenAPI spec written to {}", output.display());
    println!("Endpoints: {}", spec.paths.paths.len());
    Ok(())
}
