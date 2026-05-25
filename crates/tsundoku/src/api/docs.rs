use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    info(title = "tsundoku API", version = env!("CARGO_PKG_VERSION")),
    paths(super::health::health),
    components(schemas(super::health::Health)),
    tags((name = "tsundoku", description = "Manga discovery service that polls sources and resolves releases to MangaBaka series"))
)]
pub struct ApiDoc;
