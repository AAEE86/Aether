#[path = "providers/routes.rs"]
mod providers_routes;
#[path = "providers/shared.rs"]
mod providers_shared;

pub(crate) use providers_routes::maybe_build_local_admin_providers_response;
use providers_shared::*;
