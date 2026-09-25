//! Baley's domain code: views, projectors and the rules that decide what may
//! be recorded. Everything here reaches storage through the port in
//! `baley-store` and nothing here knows which engine is behind it
//! (design 0001, EVD-R12). The event itself, its canonical bytes and the
//! chain live in the port, because both sides of it speak them.

pub mod registry;

pub use registry::{Current, Fence, FenceReason, Registry, RegistryError, UpcastError, Upcaster};
