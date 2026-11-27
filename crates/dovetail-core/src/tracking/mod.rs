//! Tracking domain: projects, registration, Home loop, efforts, and task/decision formatting & SQL.

pub(crate) mod db;
pub(crate) mod efforts;
pub mod home;
pub(crate) mod project_admin;
pub mod projects;
pub(crate) mod read;
pub(crate) mod registration;

pub use registration::RegisterError;
