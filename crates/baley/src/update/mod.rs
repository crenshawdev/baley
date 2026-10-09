//! The update check of design 0012 section 5 and its records: staged versions
//! beside the stable path, gathering kept apart from judging, unsigned
//! development artifacts only.
//!
//! Every download passes through `verify_download`, the seam that
//! checks the SHA-256 of a development artifact. It does not check a
//! signature; that replaces its body before any release.

pub mod version;
