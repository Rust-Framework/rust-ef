//! Shared test types for integration tests.

#![allow(dead_code)]

use std::sync::Arc;

#[derive(Debug, PartialEq, Clone, Default)]
pub struct Logger {
    pub prefix: String,
}

#[derive(Debug, PartialEq, Default)]
pub struct MyService {
    pub value: i32,
}

#[derive(Debug)]
pub struct CompositeService {
    pub logger: Arc<Logger>,
}

pub trait IPlugin: Send + Sync {
    fn name(&self) -> &str;
}

#[derive(Default)]
pub struct TestPlugin;
impl IPlugin for TestPlugin {
    fn name(&self) -> &str {
        "test_plugin"
    }
}

#[derive(Default)]
pub struct AltPlugin;
impl IPlugin for AltPlugin {
    fn name(&self) -> &str {
        "alt_plugin"
    }
}
