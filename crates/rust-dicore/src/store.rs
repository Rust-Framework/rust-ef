use crate::entry::ServiceEntry;
use std::any::TypeId;
use std::collections::HashMap;

pub type ServiceStore = HashMap<TypeId, Vec<ServiceEntry>>;
