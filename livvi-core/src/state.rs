use std::any::{Any, TypeId};
use std::collections::HashMap;

/// Type-erased container for plugin-contributed state, keyed by concrete type.
///
/// Tools retrieve entries via the `State<T>` extractor; plugins insert them via
/// `PluginContext::insert_state` or `AgentBuilder::with_state`.
#[derive(Debug, Default)]
pub struct StateMap {
    map: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
}

impl StateMap {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert `value`, replacing any existing value of the same type.
    pub fn insert<T: Send + Sync + 'static>(&mut self, value: T) {
        self.map.insert(TypeId::of::<T>(), Box::new(value));
    }

    /// Borrow the value of type `T`, if one was inserted.
    pub fn get<T: Send + Sync + 'static>(&self) -> Option<&T> {
        self.map
            .get(&TypeId::of::<T>())
            .and_then(|v| v.downcast_ref::<T>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Alpha(u32);
    struct Beta(String);

    #[test]
    fn insert_and_get_roundtrip_for_distinct_types() {
        let mut map = StateMap::new();
        map.insert(Alpha(42));
        map.insert(Beta("hello".to_string()));

        assert_eq!(map.get::<Alpha>().map(|a| a.0), Some(42));
        assert_eq!(map.get::<Beta>().map(|b| b.0.as_str()), Some("hello"));
    }

    #[test]
    fn get_returns_none_for_never_inserted_type() {
        let map = StateMap::new();
        assert!(map.get::<Alpha>().is_none());
    }

    #[test]
    fn reinsert_replaces_existing_value() {
        let mut map = StateMap::new();
        map.insert(Alpha(1));
        map.insert(Alpha(2));

        assert_eq!(map.get::<Alpha>().map(|a| a.0), Some(2));
    }
}
