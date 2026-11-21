//! The set of [`SessionDriver`] implementations available to the core (`ClaudeDriver`,
//! `AgyDriver`, or fakes in tests), keyed by [`DriverKind`].

use std::collections::BTreeMap;
use std::sync::Arc;

use dovetail_api::DriverKind;
use dovetail_driver::SessionDriver;

/// The session drivers the core can launch (`ClaudeDriver`, `AgyDriver`, or fakes in tests).
#[derive(Clone, Default)]
pub struct Drivers(BTreeMap<DriverKind, Arc<dyn SessionDriver>>);

impl Drivers {
    /// An empty driver set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers `driver` under its [`SessionDriver::kind`], replacing any earlier one of that kind.
    pub fn insert(&mut self, driver: Arc<dyn SessionDriver>) {
        self.0.insert(driver.kind(), driver);
    }

    /// A set containing only `driver` (for tests with one fake).
    pub fn single(driver: Arc<dyn SessionDriver>) -> Self {
        let mut drivers = Self::new();
        drivers.insert(driver);
        drivers
    }

    /// The driver registered for `kind`, if any.
    pub fn get(&self, kind: DriverKind) -> Option<&Arc<dyn SessionDriver>> {
        self.0.get(&kind)
    }

    /// Every registered driver, in [`DriverKind`] order.
    pub fn iter(&self) -> impl Iterator<Item = (DriverKind, &Arc<dyn SessionDriver>)> {
        self.0.iter().map(|(kind, driver)| (*kind, driver))
    }
}
