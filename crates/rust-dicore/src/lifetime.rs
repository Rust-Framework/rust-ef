#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ServiceLifetime {
    Transient,
    Scoped,
    Singleton,
}
