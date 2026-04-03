#[path = "global_models/helpers.rs"]
mod global_models_helpers;
#[path = "global_models/routes.rs"]
mod global_models_routes;

use global_models_helpers::*;
pub(crate) use global_models_routes::maybe_build_local_admin_global_models_response;
