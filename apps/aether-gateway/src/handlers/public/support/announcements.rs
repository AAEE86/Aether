#[path = "announcements/admin_routes.rs"]
mod announcements_admin_routes;
#[path = "announcements/public_routes.rs"]
mod announcements_public_routes;
#[path = "announcements/shared.rs"]
mod announcements_shared;
#[path = "announcements/user_routes.rs"]
mod announcements_user_routes;

pub(crate) use self::announcements_admin_routes::maybe_build_local_admin_announcements_response;
pub(crate) use self::announcements_public_routes::maybe_build_local_public_announcements_response;
pub(crate) use self::announcements_user_routes::maybe_build_local_announcement_user_response;
