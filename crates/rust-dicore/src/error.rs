use thiserror::Error;
#[derive(Error, Debug)]
pub enum RdiError {
    #[error("service not registered: {0}")]
    ServiceNotFound(&'static str),
    #[error("keyed service not registered: key={key}, type={ty}")]
    KeyedServiceNotFound { key: String, ty: &'static str },
}
