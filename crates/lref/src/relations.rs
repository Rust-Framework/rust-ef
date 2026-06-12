//! Relationship types (`BelongsTo<T>`, `HasMany<T>`, `HasOne<T>`).
//!
//! These types serve as container wrappers for navigation properties,
//! analogous to how EFCore represents navigation properties in the model.
//!
//! They are pure marker/container types and do NOT impose entity trait
//! bounds — the constraint belongs at the usage site (DbContext, builders),
//! not on the container itself.

use std::marker::PhantomData;

// ---------------------------------------------------------------------------
// BelongsTo<T>
// ---------------------------------------------------------------------------

/// Represents a "belongs-to" navigation — the dependent side of a one-to-many
/// or one-to-one relationship where the foreign key lives on this entity.
///
/// Corresponds to EFCore's reference navigation property.
pub struct BelongsTo<T> {
    _inner: Option<Box<T>>,
    _phantom: PhantomData<T>,
}

impl<T> BelongsTo<T> {
    pub fn new() -> Self {
        Self {
            _inner: None,
            _phantom: PhantomData,
        }
    }

    pub fn with(entity: T) -> Self {
        Self {
            _inner: Some(Box::new(entity)),
            _phantom: PhantomData,
        }
    }

    pub fn get(&self) -> Option<&T> {
        self._inner.as_deref()
    }

    pub fn get_mut(&mut self) -> Option<&mut T> {
        self._inner.as_deref_mut()
    }
}

impl<T> Default for BelongsTo<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Clone for BelongsTo<T> {
    fn clone(&self) -> Self {
        Self {
            _inner: None, // Navigation properties are not deep-cloned
            _phantom: PhantomData,
        }
    }
}

impl<T> std::fmt::Debug for BelongsTo<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BelongsTo").finish()
    }
}

// ---------------------------------------------------------------------------
// HasMany<T>
// ---------------------------------------------------------------------------

/// Represents a "has-many" navigation — a collection of related entities.
///
/// Corresponds to EFCore's collection navigation property
/// (e.g., `ICollection<Post>`).
pub struct HasMany<T, Join = ()> {
    items: Vec<T>,
    _phantom: PhantomData<(T, Join)>,
}

impl<T, Join> HasMany<T, Join> {
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            _phantom: PhantomData,
        }
    }

    pub fn with(items: Vec<T>) -> Self {
        Self {
            items,
            _phantom: PhantomData,
        }
    }

    pub fn items(&self) -> &[T] {
        &self.items
    }

    pub fn items_mut(&mut self) -> &mut Vec<T> {
        &mut self.items
    }

    pub fn add(&mut self, item: T) {
        self.items.push(item);
    }

    pub fn remove(&mut self, index: usize) -> Option<T> {
        if index < self.items.len() {
            Some(self.items.remove(index))
        } else {
            None
        }
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

impl<T, Join> Default for HasMany<T, Join> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T, Join> Clone for HasMany<T, Join> {
    fn clone(&self) -> Self {
        Self {
            items: Vec::new(), // Navigation collections are not deep-cloned
            _phantom: PhantomData,
        }
    }
}

impl<T, Join> std::fmt::Debug for HasMany<T, Join> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HasMany").finish()
    }
}

/// Type alias for HasMany with an explicit join entity (many-to-many).
pub type Through<Join> = Join;

// ---------------------------------------------------------------------------
// HasOne<T>
// ---------------------------------------------------------------------------

/// Represents a "has-one" navigation — a single related entity
/// where the foreign key lives on the other side.
pub struct HasOne<T> {
    _inner: Option<Box<T>>,
    _phantom: PhantomData<T>,
}

impl<T> HasOne<T> {
    pub fn new() -> Self {
        Self {
            _inner: None,
            _phantom: PhantomData,
        }
    }

    pub fn with(entity: T) -> Self {
        Self {
            _inner: Some(Box::new(entity)),
            _phantom: PhantomData,
        }
    }

    pub fn get(&self) -> Option<&T> {
        self._inner.as_deref()
    }

    pub fn get_mut(&mut self) -> Option<&mut T> {
        self._inner.as_deref_mut()
    }
}

impl<T> Default for HasOne<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Clone for HasOne<T> {
    fn clone(&self) -> Self {
        Self {
            _inner: None,
            _phantom: PhantomData,
        }
    }
}

impl<T> std::fmt::Debug for HasOne<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HasOne").finish()
    }
}

// ---------------------------------------------------------------------------
// DeleteBehavior (for cascade configuration)
// ---------------------------------------------------------------------------

/// Specifies the delete behavior for a relationship.
/// Corresponds to EFCore's `DeleteBehavior`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteBehavior {
    Cascade,
    Restrict,
    SetNull,
    NoAction,
}
