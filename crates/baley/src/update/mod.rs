//! The update check of design 0012 section 5 and its records: staged versions
//! beside the stable path, gathering kept apart from judging, unsigned
//! development artifacts only.
//!
//! Every download passes through [`manifest::verify_download`], the seam that
//! checks the SHA-256 of a development artifact. It does not check a
//! signature; that replaces its body before any release.

pub mod claim;
pub mod deliver;
pub mod events;
pub mod fetch;
pub mod installation;
pub mod manifest;
pub mod receipt;
pub mod record;
pub mod seed;
pub mod version;
