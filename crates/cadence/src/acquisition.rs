//! Admission is a value decision; filesystem adapters only gather observations.
use std::{fmt, io};

pub const MAX_SOURCE_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_STORE_BYTES: u64 = 1024 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class { Source, Store }

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Crossing {
    pub file: String,
    pub size: u64,
    pub bound: u64,
}

impl fmt::Display for Crossing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: size {} exceeds acquisition bound {}", self.file, self.size, self.bound)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action { Read { bound: u64 }, Refuse(Crossing) }

pub fn decide(_file: &str, class: Class, _size: u64) -> Action {
    let bound = match class { Class::Source => MAX_SOURCE_BYTES, Class::Store => MAX_STORE_BYTES };
    Action::Read { bound }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Observation {
    pub device: u64,
    pub inode: u64,
    pub size: u64,
    pub mode: u32,
    pub modified: (i64, i64),
    pub changed: (i64, i64),
    pub regular: bool,
}

#[derive(Debug)]
pub enum Error {
    Crossing(Crossing),
    Changed(String),
    NotRegular(String),
    Io(io::Error),
    Store(crate::store::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Crossing(crossing) => crossing.fmt(f),
            Self::Changed(file) => write!(f, "{file}: input changed during acquisition"),
            Self::NotRegular(file) => write!(f, "{file}: input is not a regular file"),
            Self::Io(error) => error.fmt(f),
            Self::Store(error) => error.fmt(f),
        }
    }
}
impl std::error::Error for Error {}
impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self { Self::Io(error) }
}

pub fn store_error(error: Error) -> crate::store::Error {
    match error {
        Error::Crossing(crossing) => crate::store::Error::Invalid(crossing.to_string()),
        Error::Io(error) => error.into(),
        Error::Store(error) => error,
        error @ Error::Changed(_) => crate::store::Error::Conflict(error.to_string()),
        error @ Error::NotRegular(_) => crate::store::Error::Invalid(error.to_string()),
    }
}

pub fn revalidate(_file: &str, _class: Class, _before: &Observation, _after: &Observation,
    _acquired: u64) -> Result<(), Error>
{
    Ok(())
}

#[cfg(test)]
mod tests;
