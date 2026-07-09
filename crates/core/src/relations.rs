//! Relationship types (`BelongsTo<T>`, `HasMany<T>`, `HasOne<T>`).
//!
//! These types serve as container wrappers for navigation properties,
//! analogous to how EFCore represents navigation properties in the model.
//!
//! They are pure marker/container types and do NOT impose entity trait
//! bounds  - ?the constraint belongs at the usage site (DbContext, builders),
//! not on the container itself.
//!
//! ## Lazy loading (v1.1+)
//!
//! Each container holds an optional [`Arc<dyn LazyContext>`] and a `loaded`
//! flag. When lazy loading is enabled on the `DbContext`, `to_list()`
//! attaches a `LazyContext` to every navigation container on every
//! materialized entity. The user can then call `nav.load().await` to
//! trigger a single-entity navigation query on first access; subsequent
//! accesses read from the in-memory cache.
//!
//! `Clone` intentionally drops the lazy context and resets `loaded` to
//! `false`, matching the v1.0 semantics where cloning a navigation
//! container yields an empty (unloaded) copy.

use crate::entity::{IEntityType, IFromRow, ILazyInit};
use crate::error::{EFError, EFResult};
use crate::lazy::{load_collection_lazy, load_scalar_lazy, LazyContext, MAX_LAZY_DEPTH};
use std::marker::PhantomData;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// BelongsTo<T>
// ---------------------------------------------------------------------------

/// Represents a "belongs-to" navigation  - ?the dependent side of a one-to-many
/// or one-to-one relationship where the foreign key lives on this entity.
///
/// Corresponds to EFCore's reference navigation property.
pub struct BelongsTo<T> {
    inner: Option<Box<T>>,
    lazy_ctx: Option<Arc<dyn LazyContext>>,
    loaded: bool,
    _phantom: PhantomData<T>,
}

impl<T> BelongsTo<T> {
    pub fn new() -> Self {
        Self {
            inner: None,
            lazy_ctx: None,
            loaded: false,
            _phantom: PhantomData,
        }
    }

    pub fn with(entity: T) -> Self {
        Self {
            inner: Some(Box::new(entity)),
            lazy_ctx: None,
            loaded: true,
            _phantom: PhantomData,
        }
    }

    pub fn get(&self) -> Option<&T> {
        self.inner.as_deref()
    }

    pub fn get_mut(&mut self) -> Option<&mut T> {
        self.inner.as_deref_mut()
    }

    /// Extracts the related entity, leaving the container empty and unloaded.
    /// Used by batch nested-include loading to collect related entities
    /// across multiple parents into a single slice for batch SQL.
    pub fn take(&mut self) -> Option<T> {
        let taken = self.inner.take();
        self.loaded = false;
        taken.map(|b| *b)
    }

    /// Returns `true` if the navigation has been loaded (either eagerly
    /// via `Include` or lazily via [`load`]).
    pub fn is_loaded(&self) -> bool {
        self.loaded
    }

    /// Attaches a lazy-loading context to this container.
    ///
    /// Called by the `#[derive(EntityType)]` macro's `ILazyInit`
    /// implementation when lazy loading is enabled on the `DbContext`.
    pub fn set_lazy_context(&mut self, ctx: Arc<dyn LazyContext>) {
        self.lazy_ctx = Some(ctx);
    }

    /// Triggers lazy loading of this reference navigation.
    ///
    /// If the navigation is already loaded, this is a no-op. If no
    /// [`LazyContext`] is attached (lazy loading disabled), returns
    /// `Ok(())` without loading.
    ///
    /// After this call, [`get`] returns the loaded entity (or `None` if
    /// no matching row was found).
    pub async fn load(&mut self) -> EFResult<()>
    where
        T: IFromRow + IEntityType + ILazyInit,
    {
        if self.loaded {
            return Ok(());
        }
        let Some(ctx) = self.lazy_ctx.clone() else {
            // No lazy context attached — lazy loading disabled.
            return Ok(());
        };
        if ctx.depth() >= MAX_LAZY_DEPTH {
            return Err(EFError::other("lazy loading recursion limit exceeded"));
        }
        let maybe_entity = load_scalar_lazy::<T>(ctx.as_ref()).await?;
        if let Some(mut entity) = maybe_entity {
            // Attach lazy contexts to the child for nested lazy loading.
            let provider = ctx.provider().clone();
            let filter_map = ctx.filter_map().map(|m| Arc::new(m.clone()));
            entity.attach_lazy_contexts(provider, filter_map, ctx.depth() + 1);
            self.inner = Some(Box::new(entity));
        }
        self.loaded = true;
        Ok(())
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
            inner: None, // Navigation properties are not deep-cloned
            lazy_ctx: None,
            loaded: false,
            _phantom: PhantomData,
        }
    }
}

impl<T> std::fmt::Debug for BelongsTo<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BelongsTo")
            .field("loaded", &self.loaded)
            .finish()
    }
}

// ---------------------------------------------------------------------------
// HasMany<T>
// ---------------------------------------------------------------------------

/// Represents a "has-many" navigation  - ?a collection of related entities.
///
/// Corresponds to EFCore's collection navigation property
/// (e.g., `ICollection<Post>`).
pub struct HasMany<T, Join = ()> {
    items: Vec<T>,
    lazy_ctx: Option<Arc<dyn LazyContext>>,
    loaded: bool,
    _phantom: PhantomData<(T, Join)>,
}

impl<T, Join> HasMany<T, Join> {
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            lazy_ctx: None,
            loaded: false,
            _phantom: PhantomData,
        }
    }

    pub fn with(items: Vec<T>) -> Self {
        Self {
            items,
            lazy_ctx: None,
            loaded: true,
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

    /// Returns `true` if the navigation has been loaded (either eagerly
    /// via `Include` or lazily via [`load`]).
    pub fn is_loaded(&self) -> bool {
        self.loaded
    }

    /// Attaches a lazy-loading context to this container.
    ///
    /// Called by the `#[derive(EntityType)]` macro's `ILazyInit`
    /// implementation when lazy loading is enabled on the `DbContext`.
    pub fn set_lazy_context(&mut self, ctx: Arc<dyn LazyContext>) {
        self.lazy_ctx = Some(ctx);
    }

    /// Triggers lazy loading of this collection navigation.
    ///
    /// If the navigation is already loaded, this is a no-op. If no
    /// [`LazyContext`] is attached (lazy loading disabled), returns
    /// `Ok(())` without loading.
    ///
    /// After this call, [`items`] returns the loaded collection.
    pub async fn load(&mut self) -> EFResult<()>
    where
        T: IFromRow + IEntityType + ILazyInit,
        Join: 'static,
    {
        if self.loaded {
            return Ok(());
        }
        let Some(ctx) = self.lazy_ctx.clone() else {
            // No lazy context attached — lazy loading disabled.
            return Ok(());
        };
        if ctx.depth() >= MAX_LAZY_DEPTH {
            return Err(EFError::other("lazy loading recursion limit exceeded"));
        }
        let mut entities = load_collection_lazy::<T>(ctx.as_ref()).await?;
        // Attach lazy contexts to each child for nested lazy loading.
        let provider = ctx.provider().clone();
        let filter_map = ctx.filter_map().map(|m| Arc::new(m.clone()));
        let depth = ctx.depth() + 1;
        for entity in &mut entities {
            // Re-bind to split let style for readability.
            let p = provider.clone();
            let fm = filter_map.clone();
            entity.attach_lazy_contexts(p, fm, depth);
        }
        self.items = entities;
        self.loaded = true;
        Ok(())
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
            lazy_ctx: None,
            loaded: false,
            _phantom: PhantomData,
        }
    }
}

impl<T, Join> std::fmt::Debug for HasMany<T, Join> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HasMany")
            .field("loaded", &self.loaded)
            .field("len", &self.items.len())
            .finish()
    }
}

/// Type alias for HasMany with an explicit join entity (many-to-many).
pub type Through<Join> = Join;

// ---------------------------------------------------------------------------
// HasOne<T>
// ---------------------------------------------------------------------------

/// Represents a "has-one" navigation  - ?a single related entity
/// where the foreign key lives on the other side.
pub struct HasOne<T> {
    inner: Option<Box<T>>,
    lazy_ctx: Option<Arc<dyn LazyContext>>,
    loaded: bool,
    _phantom: PhantomData<T>,
}

impl<T> HasOne<T> {
    pub fn new() -> Self {
        Self {
            inner: None,
            lazy_ctx: None,
            loaded: false,
            _phantom: PhantomData,
        }
    }

    pub fn with(entity: T) -> Self {
        Self {
            inner: Some(Box::new(entity)),
            lazy_ctx: None,
            loaded: true,
            _phantom: PhantomData,
        }
    }

    pub fn get(&self) -> Option<&T> {
        self.inner.as_deref()
    }

    pub fn get_mut(&mut self) -> Option<&mut T> {
        self.inner.as_deref_mut()
    }

    /// Extracts the related entity, leaving the container empty and unloaded.
    /// Used by batch nested-include loading to collect related entities
    /// across multiple parents into a single slice for batch SQL.
    pub fn take(&mut self) -> Option<T> {
        let taken = self.inner.take();
        self.loaded = false;
        taken.map(|b| *b)
    }

    /// Returns `true` if the navigation has been loaded.
    pub fn is_loaded(&self) -> bool {
        self.loaded
    }

    /// Attaches a lazy-loading context to this container.
    pub fn set_lazy_context(&mut self, ctx: Arc<dyn LazyContext>) {
        self.lazy_ctx = Some(ctx);
    }

    /// Triggers lazy loading of this reference navigation.
    pub async fn load(&mut self) -> EFResult<()>
    where
        T: IFromRow + IEntityType + ILazyInit,
    {
        if self.loaded {
            return Ok(());
        }
        let Some(ctx) = self.lazy_ctx.clone() else {
            return Ok(());
        };
        if ctx.depth() >= MAX_LAZY_DEPTH {
            return Err(EFError::other("lazy loading recursion limit exceeded"));
        }
        let maybe_entity = load_scalar_lazy::<T>(ctx.as_ref()).await?;
        if let Some(mut entity) = maybe_entity {
            let provider = ctx.provider().clone();
            let filter_map = ctx.filter_map().map(|m| Arc::new(m.clone()));
            entity.attach_lazy_contexts(provider, filter_map, ctx.depth() + 1);
            self.inner = Some(Box::new(entity));
        }
        self.loaded = true;
        Ok(())
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
            inner: None,
            lazy_ctx: None,
            loaded: false,
            _phantom: PhantomData,
        }
    }
}

impl<T> std::fmt::Debug for HasOne<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HasOne")
            .field("loaded", &self.loaded)
            .finish()
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

impl DeleteBehavior {
    /// Maps to the SQL `ON DELETE` clause keyword.
    pub fn to_sql_clause(self) -> &'static str {
        match self {
            DeleteBehavior::Cascade => "CASCADE",
            DeleteBehavior::Restrict => "RESTRICT",
            DeleteBehavior::SetNull => "SET NULL",
            DeleteBehavior::NoAction => "NO ACTION",
        }
    }
}
