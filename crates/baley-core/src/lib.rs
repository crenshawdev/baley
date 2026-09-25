//! Baley's domain code: events, views, projectors and the rules that decide
//! what may be recorded. Everything here reaches storage through the port in
//! `baley-store` and nothing here knows which engine is behind it
//! (design 0001, EVD-R12).
