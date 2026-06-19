use crate::entry::ServiceFactory;
use crate::lifetime::ServiceLifetime;
use std::any::TypeId;

#[derive(Clone)]
pub struct ServiceDescriptor {
    pub type_id: TypeId,
    pub type_name: &'static str,
    pub key: Option<String>,
    pub factory: ServiceFactory,
    pub lifetime: ServiceLifetime,
}
