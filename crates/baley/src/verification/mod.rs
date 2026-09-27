//! Native verification authority, independent from execution completion.
pub mod audit;
pub mod completion;
pub mod dispatch;
pub mod human;
pub mod inputs;
pub mod instructions;
pub mod model;
pub mod persistence;
pub mod projections;
pub mod render;
pub mod runner;
pub mod status;
pub mod verdicts;
pub mod waivers;

#[cfg(test)]
mod accounting_tests;
