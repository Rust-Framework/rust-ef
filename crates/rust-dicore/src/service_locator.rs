use std::any::Any;
use std::sync::Arc;

/// Trait for service location support.
pub trait IServiceLocator: Send + Sync {
    fn get_any(&self, type_key: &str) -> Option<Arc<dyn Any + Send + Sync>>;
    fn get_any_named(&self, name: &str) -> Option<Arc<dyn Any + Send + Sync>>;
    fn register_named_any(&self, name: &str, service: Arc<dyn Any + Send + Sync>);
    fn remove_named(&self, name: &str);
}

/// Trait for named service registration (mutable).
pub trait INamedRegistrar: Send + Sync {
    fn register_named_any(&self, name: &str, service: Arc<dyn Any + Send + Sync>);
    fn remove_named(&self, name: &str);
}
